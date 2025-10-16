use std::sync::{Arc, Mutex};

use esp_idf_svc::eventloop::EspSystemEventLoop;

mod app;
mod audio;
mod bt;
mod hal;
mod network;
mod protocol;
mod ui;
mod ws;

#[derive(Debug, Clone)]
struct Setting {
    ssid: String,
    pass: String,
    server_url: String,
    background_gif: (Vec<u8>, bool), // (data, ended)
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = esp_idf_svc::hal::prelude::Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20)?;
    let partition = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
    let nvs = esp_idf_svc::nvs::EspDefaultNvs::new(partition, "setting", true)?;

    log_heap();

    crate::hal::audio_init();
    ui::lcd_init().unwrap();

    log_heap();

    let state = nvs.get_u8("state").ok().flatten().unwrap_or(0);

    let mut ssid_buf = [0; 32];
    let ssid = nvs
        .get_str("ssid", &mut ssid_buf)
        .map_err(|e| log::error!("Failed to get ssid: {:?}", e))
        .ok()
        .flatten();

    let mut pass_buf = [0; 64];
    let pass = nvs
        .get_str("pass", &mut pass_buf)
        .map_err(|e| log::error!("Failed to get pass: {:?}", e))
        .ok()
        .flatten();

    let mut server_url = [0; 128];
    let server_url = nvs
        .get_str("server_url", &mut server_url)
        .map_err(|e| log::error!("Failed to get server_url: {:?}", e))
        .ok()
        .flatten();

    // 1MB buffer for GIF
    let mut gif_buf = vec![0; 1024 * 1024];
    let background_gif = nvs
        .get_blob("background_gif", &mut gif_buf)?
        .unwrap_or(ui::DEFAULT_BACKGROUND);

    log::info!("SSID: {:?}", ssid);
    log::info!("PASS: {:?}", pass);
    log::info!("Server URL: {:?}", server_url);

    nvs.set_u8("state", 0).unwrap();

    log_heap();
    let _ = ui::backgroud(&background_gif);

    // Configures the button
    let mut button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio0)?;
    button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    let b = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut gui = ui::UI::new(None).unwrap();

    let setting = Arc::new(Mutex::new((
        Setting {
            ssid: ssid.unwrap_or_default().to_string(),
            pass: pass.unwrap_or_default().to_string(),
            server_url: server_url.unwrap_or_default().to_string(),
            background_gif: (Vec::with_capacity(1024 * 1024), false), // 1MB
        },
        nvs,
    )));

    log_heap();

    let need_init = {
        let setting = setting.lock().unwrap();
        setting.0.ssid.is_empty()
            || setting.0.pass.is_empty()
            || setting.0.server_url.is_empty()
            || button.is_low()
            || state == 1
    };
    if need_init {
        let ble_addr = bt::bt(setting.clone()).unwrap();
        log_heap();

        gui.state = "Please setup device by bt".to_string();
        gui.text = format!("Goto https://echokit.dev/setup/ to set up the device.\nPress K0 to continue\nDevice Name: EchoKit-{}", ble_addr);
        gui.display_qrcode("https://echokit.dev/setup/").unwrap();

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

        b.block_on(button.wait_for_falling_edge()).unwrap();
        {
            let mut setting = setting.lock().unwrap();
            if setting.0.background_gif.1 {
                gui.text = "Testing background GIF...".to_string();
                gui.display_flush().unwrap();

                let mut new_gif = Vec::new();
                std::mem::swap(&mut setting.0.background_gif.0, &mut new_gif);

                let _ = ui::backgroud(&new_gif);
                log::info!("Background GIF set from NVS");

                gui.text = "Background GIF set OK".to_string();
                gui.display_flush().unwrap();

                if !new_gif.is_empty() {
                    setting
                        .1
                        .set_blob("background_gif", &new_gif)
                        .map_err(|e| log::error!("Failed to save background GIF to NVS: {:?}", e))
                        .unwrap();
                    log::info!("Background GIF saved to NVS");
                }
            }
        }

        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    gui.state = "Connecting to wifi...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    let _wifi = {
        let setting = setting.lock().unwrap();
        network::wifi(
            &setting.0.ssid,
            &setting.0.pass,
            peripherals.modem,
            sysloop.clone(),
        )
    };
    if _wifi.is_err() {
        gui.state = "Failed to connect to wifi".to_string();
        gui.text = "Press K0 to open settings".to_string();
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        let setting = setting.lock().unwrap();
        setting.1.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let wifi = _wifi.unwrap();
    log_heap();

    let mac = wifi.ap_netif().get_mac().unwrap();
    let mac_str = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let (evt_tx, evt_rx) = tokio::sync::mpsc::channel(64);
    let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();

    #[cfg(feature = "box")]
    let i2s_task = {
        let bclk = peripherals.pins.gpio21;
        let din = peripherals.pins.gpio47;
        let dout = peripherals.pins.gpio14;
        let ws = peripherals.pins.gpio13;

        audio::i2s_task(
            peripherals.i2s0,
            bclk.into(),
            din.into(),
            dout.into(),
            ws.into(),
            (evt_tx.clone(), rx1),
        )
    };

    #[cfg(feature = "boards")]
    let i2s_task = {
        let sck = peripherals.pins.gpio5;
        let din = peripherals.pins.gpio6;
        let dout = peripherals.pins.gpio7;
        let ws = peripherals.pins.gpio4;
        let bclk = peripherals.pins.gpio15;
        let lrclk = peripherals.pins.gpio16;

        audio::i2s_task_(
            peripherals.i2s0,
            ws.into(),
            sck.into(),
            din.into(),
            peripherals.i2s1,
            bclk.into(),
            lrclk.into(),
            dout.into(),
            (evt_tx.clone(), rx1),
        )
    };

    gui.state = "Connecting to server...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    log_heap();

    let server_url = {
        let setting = setting.lock().unwrap();
        format!("{}{}", setting.0.server_url, mac_str)
    };
    let server = b.block_on(ws::Server::new(server_url.clone()));
    if server.is_err() {
        gui.state = "Failed to connect to server".to_string();
        gui.text = format!("Please check your server URL: {server_url}\nPress K0 to open settings");
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        let setting = setting.lock().unwrap();
        setting.1.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let server = server.unwrap();

    let ws_task = app::main_work(server, tx1, evt_rx, Some(background_gif));

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

    b.spawn(i2s_task);
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
            "Free SPIRAM heap size: {}",
            heap_caps_get_free_size(MALLOC_CAP_SPIRAM)
        );
        log::info!(
            "Free INTERNAL heap size: {}",
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL)
        );
    }
}
