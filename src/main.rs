use std::sync::{Arc, Mutex};

use embedded_graphics::{
    prelude::{Dimensions, RgbColor, WebColors},
    Drawable,
};
use esp_idf_svc::eventloop::EspSystemEventLoop;

use crate::ui::DisplayTargetDrive;

mod app;
mod audio;
mod bt;
mod codec;
mod network;
mod protocol;
mod ui;
mod ws;

mod boards;

mod peripheral;

#[derive(Debug, Clone)]
struct Setting {
    ssid: String,
    pass: String,
    server_url: String,
    background_gif: (Vec<u8>, bool), // (data, ended)
    avatar_gif: (Vec<u8>, bool),     // (data, ended)
    state: u8,                       // if 1, enter setup mode
    // AFE parameters
    afe_linear_gain: f32,
    agc_target_level_dbfs: i32,
    agc_compression_gain_db: i32,
}

impl Setting {
    fn load_from_nvs(nvs: &esp_idf_svc::nvs::EspDefaultNvs) -> anyhow::Result<Self> {
        let mut str_buf = [0; 128];

        let ssid = nvs
            .get_str("ssid", &mut str_buf)
            .map_err(|e| log::error!("Failed to get ssid: {:?}", e))
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string();

        let pass = nvs
            .get_str("pass", &mut str_buf)
            .map_err(|e| log::error!("Failed to get pass: {:?}", e))
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string();

        static DEFAULT_SERVER_URL: Option<&str> = std::option_env!("DEFAULT_SERVER_URL");
        log::info!("DEFAULT_SERVER_URL: {:?}", DEFAULT_SERVER_URL);

        let server_url = nvs
            .get_str("server_url", &mut str_buf)
            .map_err(|e| log::error!("Failed to get server_url: {:?}", e))
            .ok()
            .flatten()
            .or(DEFAULT_SERVER_URL)
            .unwrap_or_default()
            .to_string();

        let background_gif = if nvs.contains("background_gif")? {
            let background_gif_size = nvs
                .blob_len("background_gif")
                .map_err(|e| log::error!("Failed to get background_gif size: {:?}", e))
                .ok()
                .flatten()
                .unwrap_or(1024 * 1024);

            let mut gif_buf = vec![0; background_gif_size];
            let gif_buf_ = nvs
                .get_blob("background_gif", &mut gif_buf)?
                .unwrap_or(ui::DEFAULT_BACKGROUND);

            if gif_buf_.len() != background_gif_size {
                log::warn!(
                    "Background GIF size mismatch: expected {}, got {}",
                    background_gif_size,
                    gif_buf_.len()
                );
                gif_buf_.to_vec()
            } else {
                gif_buf
            }
        } else {
            ui::DEFAULT_BACKGROUND.to_vec()
        };

        let avatar_gif = if nvs.contains("avatar_gif")? {
            let avatar_gif_size = nvs
                .blob_len("avatar_gif")
                .map_err(|e| log::error!("Failed to get avatar_gif size: {:?}", e))
                .ok()
                .flatten()
                .unwrap_or(128 * 1024);

            let mut gif_buf = vec![0; avatar_gif_size];
            let gif_buf_ = nvs.get_blob("avatar_gif", &mut gif_buf)?.unwrap_or(&[]);

            if gif_buf_.len() != avatar_gif_size {
                log::warn!(
                    "Avatar GIF size mismatch: expected {}, got {}",
                    avatar_gif_size,
                    gif_buf_.len()
                );
                gif_buf_.to_vec()
            } else {
                gif_buf
            }
        } else {
            Vec::new()
        };

        let state = nvs.get_u8("state")?.unwrap_or(0);

        let mut afe_linear_gain_buf = [0u8; 4];
        let afe_linear_gain = nvs
            .get_blob("afe_linear_gain", &mut afe_linear_gain_buf)
            .map_err(|e| {
                log::error!("Failed to get afe_linear_gain: {:?}", e);
            })
            .ok()
            .flatten()
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(unsafe { audio::AFE_LINEAR_GAIN });

        let agc_target_level_dbfs = nvs
            .get_i32("agc_tl_dbfs")
            .map_err(|e| {
                log::error!("Failed to get agc_target_level_dbfs: {:?}", e);
            })
            .ok()
            .flatten()
            .unwrap_or(unsafe { audio::AGC_TARGET_LEVEL_DBFS });

        let agc_compression_gain_db = nvs
            .get_i32("agc_cg_db")
            .map_err(|e| {
                log::error!("Failed to get agc_compression_gain_db: {:?}", e);
            })
            .ok()
            .flatten()
            .unwrap_or(unsafe { audio::AGC_COMPRESSION_GAIN_DB });

        Ok(Setting {
            ssid,
            pass,
            server_url,
            background_gif: (background_gif, false),
            avatar_gif: (avatar_gif, false),
            state,
            afe_linear_gain,
            agc_target_level_dbfs,
            agc_compression_gain_db,
        })
    }

    fn need_init(&self) -> bool {
        self.state == 1
            || self.ssid.is_empty()
            || self.pass.is_empty()
            || self.server_url.is_empty()
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = esp_idf_svc::hal::prelude::Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20)?;
    let partition = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
    let nvs = esp_idf_svc::nvs::EspDefaultNvs::new(partition, "setting", true)?;

    let mut setting = Setting::load_from_nvs(&nvs)?;
    nvs.set_u8("state", 0).unwrap();

    log::info!("SSID: {:?}", setting.ssid);
    log::info!("PASS: {:?}", setting.pass);
    log::info!("Server URL: {:?}", setting.server_url);

    log_heap();

    let (evt_tx, mut evt_rx) = tokio::sync::mpsc::channel(64);
    let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();

    crate::start_hal!(peripherals, evt_tx);

    let mut framebuffer = Box::new(boards::ui::DisplayBuffer::new(ui::ColorFormat::WHITE));
    framebuffer.flush()?;

    crate::ui::display_gif(framebuffer.as_mut(), &setting.background_gif.0).unwrap();

    // Configures the button
    let mut button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio0)?;
    button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::AnyEdge)?;

    let b = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    log_heap();

    let mut chat_ui = boards::ui::new_chat_ui::<6>(framebuffer.as_mut(), &setting.avatar_gif.0)?;

    #[cfg(feature = "extra_server")]
    {
        chat_ui.set_state("Initializing...".to_string());
        chat_ui.set_text("Loading Server URL...".to_string());

        chat_ui.render_to_target(framebuffer.as_mut())?;
        framebuffer.flush()?;

        while let Some(event) = evt_rx.blocking_recv() {
            if let app::Event::ServerUrl(url) = event {
                log::info!("Received ServerUrl event: {}", url);
                if !url.is_empty() {
                    setting.server_url = url;
                }
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(500));
        chat_ui.set_text(format!("Server URL: {}\nContinuing...", setting.server_url));
        chat_ui.render_to_target(framebuffer.as_mut())?;
        framebuffer.flush()?;
        std::thread::sleep(std::time::Duration::from_millis(2000));
    }

    let need_init = button.is_low() || setting.need_init();

    if need_init {
        // let mut config_ui = ui::new_config_ui(start_ui, "https://echokit.dev/setup/")?;

        let esp_wifi = esp_idf_svc::wifi::EspWifi::new(peripherals.modem, sysloop, None)?;
        let mac = esp_wifi.sta_netif().get_mac()?;
        let dev_id = format!(
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        setting.background_gif.0.clear();
        setting.avatar_gif.0.clear();
        let setting = Arc::new(Mutex::new((setting, nvs)));

        bt::bt(&dev_id, setting.clone(), evt_tx).unwrap();
        log_heap();

        let version = env!("CARGO_PKG_VERSION");

        framebuffer.fill_color(ui::ColorFormat::CSS_GRAY)?;
        let mut config_ui = boards::ui::ConfiguresUI::new(framebuffer.bounding_box(), "https://echokit.dev/setup/", format!("Goto https://echokit.dev/setup/ to set up the device.\nDevice Name: EchoKit-{}\nVersion: {}", dev_id, version)).unwrap();

        config_ui.draw(framebuffer.as_mut())?;
        framebuffer.flush()?;

        #[cfg(feature = "boards")]
        {
            let dout = peripherals.pins.gpio7;
            let bclk = peripherals.pins.gpio15;
            let lrclk = peripherals.pins.gpio16;
            audio::player_welcome(
                peripherals.i2s0,
                bclk.into(),
                dout.into(),
                lrclk.into(),
                None,
                None,
            );
        }

        b.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

            tokio::select! {
                _ = button.wait_for_falling_edge() =>{
                    log::info!("Button k0 pressed to enter setup");
                }
                _ = evt_rx.recv() => {
                    log::info!("Received event to enter setup");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        });

        {
            let mut setting = setting.lock().unwrap();
            if setting.0.background_gif.1 {
                config_ui.set_info("Testing background GIF...".to_string());
                config_ui.draw(framebuffer.as_mut())?;
                framebuffer.flush()?;

                let mut new_gif = Vec::new();
                std::mem::swap(&mut setting.0.background_gif.0, &mut new_gif);

                crate::ui::display_gif(framebuffer.as_mut(), &new_gif).unwrap();
                log::info!("Background GIF set from NVS");

                config_ui.set_info("Background GIF set OK".to_string());
                config_ui.draw(framebuffer.as_mut())?;
                framebuffer.flush()?;

                setting
                    .1
                    .set_blob("background_gif", &new_gif)
                    .map_err(|e| log::error!("Failed to save background GIF to NVS: {:?}", e))
                    .unwrap();
                log::info!("Background GIF saved to NVS");
            }

            if setting.0.avatar_gif.1 {
                config_ui.set_info("Testing avatar GIF...".to_string());
                config_ui.draw(framebuffer.as_mut())?;
                framebuffer.flush()?;

                let mut new_gif = Vec::new();
                std::mem::swap(&mut setting.0.avatar_gif.0, &mut new_gif);

                crate::ui::display_gif(framebuffer.as_mut(), &new_gif).unwrap();
                log::info!("Avatar GIF set from NVS");

                config_ui.set_info("Avatar GIF set OK".to_string());
                config_ui.draw(framebuffer.as_mut())?;
                framebuffer.flush()?;

                setting
                    .1
                    .set_blob("avatar_gif", &new_gif)
                    .map_err(|e| log::error!("Failed to save avatar GIF to NVS: {:?}", e))
                    .unwrap();
                log::info!("Avatar GIF saved to NVS");
            }
        }

        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    unsafe {
        audio::AFE_LINEAR_GAIN = setting.afe_linear_gain;
        audio::AGC_TARGET_LEVEL_DBFS = setting.agc_target_level_dbfs;
        audio::AGC_COMPRESSION_GAIN_DB = setting.agc_compression_gain_db;
    }

    chat_ui.set_state("Connecting to wifi...".to_string());
    chat_ui.render_to_target(framebuffer.as_mut())?;
    framebuffer.flush()?;

    let _wifi = network::wifi(
        &setting.ssid,
        &setting.pass,
        peripherals.modem,
        sysloop.clone(),
    );
    if _wifi.is_err() {
        chat_ui.set_state("Failed to connect to wifi".to_string());
        chat_ui.set_text("Press K0 to open settings".to_string());
        chat_ui.render_to_target(framebuffer.as_mut())?;
        framebuffer.flush()?;

        b.block_on(button.wait_for_falling_edge()).unwrap();
        nvs.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let wifi = _wifi.unwrap();
    log_heap();

    let mac = wifi.sta_netif().get_mac().unwrap();
    let dev_id = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    chat_ui.set_state("Connecting to server...".to_string());
    chat_ui.set_text("".to_string());
    chat_ui.render_to_target(framebuffer.as_mut())?;
    framebuffer.flush()?;

    log_heap();

    chat_ui.set_state("Failed to connect to server".to_string());
    chat_ui.set_text(format!(
        "Please check your server URL: {}\nPress K0 to open settings",
        setting.server_url
    ));
    let server = b.block_on(ws::Server::new(dev_id, setting.server_url));
    if server.is_err() {
        log::info!("Failed to connect to server: {:?}", server.err());
        chat_ui.render_to_target(framebuffer.as_mut())?;
        framebuffer.flush()?;
        b.block_on(button.wait_for_falling_edge()).unwrap();
        nvs.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let server = server.unwrap();

    crate::start_audio_workers!(peripherals, rx1, evt_tx.clone(), &b);

    let ws_task = app::main_work(server, tx1, evt_rx, &mut framebuffer, &mut chat_ui);

    b.spawn(async move {
        loop {
            let _ = button.wait_for_falling_edge().await;
            log::info!("Button k0 pressed {:?}", button.get_level());

            let r = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                button.wait_for_rising_edge(),
            )
            .await;
            match r {
                Ok(_) => {
                    if evt_tx
                        .send(app::Event::Event(app::Event::K0))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K0 event");
                        break;
                    }
                }
                Err(_) => {
                    if evt_tx
                        .send(app::Event::Event(app::Event::K0_))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K0 event");
                        break;
                    }
                }
            }
        }
    });

    b.block_on(async move {
        let r = ws_task.await;
        if let Err(e) = r {
            log::error!("Error: {:?}", e);
        } else {
            log::info!("WebSocket task finished successfully");
        }
    });
    log::error!("WebSocket task finished");
    unsafe { esp_idf_svc::sys::esp_restart() }
}

pub fn log_heap() {
    unsafe {
        use esp_idf_svc::sys::{heap_caps_get_free_size, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM};

        log::info!(
            "Free SPIRAM heap size: {}KB",
            heap_caps_get_free_size(MALLOC_CAP_SPIRAM) / 1024
        );
        log::info!(
            "Free INTERNAL heap size: {}KB",
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL) / 1024
        );
    }
}

fn print_stack_high() {
    let stack_high =
        unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark2(std::ptr::null_mut()) };
    log::info!("Stack high: {}", stack_high);
}
