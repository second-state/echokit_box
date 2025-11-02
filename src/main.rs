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

const AUDIO_STACK_SIZE: usize = 15 * 1024;

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
    #[cfg(feature = "cube2")]
    let _backlight = {
        let mut backlight = ui::backlight_init(peripherals.pins.gpio13.into()).unwrap();
        ui::set_backlight(&mut backlight, 50).unwrap();
        backlight
    };

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

    let evt_tx_ = evt_tx.clone();

    #[cfg(feature = "box")]
    {
        let bclk = peripherals.pins.gpio21;
        let din = peripherals.pins.gpio47;
        let dout = peripherals.pins.gpio14;
        let ws = peripherals.pins.gpio13;

        let worker = audio::BoxAudioWorker {
            i2s: peripherals.i2s0,
            bclk: bclk.into(),
            din: din.into(),
            dout: dout.into(),
            ws: ws.into(),
            mclk: None,
        };

        std::thread::Builder::new()
            .stack_size(AUDIO_STACK_SIZE)
            .spawn(move || {
                let r = worker.run(rx1, evt_tx_);
                if let Err(e) = r {
                    log::error!("Audio worker error: {:?}", e);
                }
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn audio worker thread: {:?}", e))?;
    }

    #[cfg(not(feature = "box"))]
    {
        let sck = peripherals.pins.gpio5;
        let din = peripherals.pins.gpio6;
        let dout = peripherals.pins.gpio7;
        let ws = peripherals.pins.gpio4;
        let bclk = peripherals.pins.gpio15;
        let lrclk = peripherals.pins.gpio16;

        let worker = audio::BoardsAudioWorker {
            out_i2s: peripherals.i2s1,
            out_ws: lrclk.into(),
            out_clk: bclk.into(),
            dout: dout.into(),
            out_mclk: None,

            in_i2s: peripherals.i2s0,
            in_ws: ws.into(),
            in_clk: sck.into(),
            din: din.into(),
            in_mclk: None,
        };

        // let mut conf =
        //     esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration::get().unwrap_or_default();
        // conf.pin_to_core = Some(esp_idf_svc::hal::cpu::Core::Core1);
        // let r = conf.set();
        // if let Err(e) = r {
        //     log::error!("Failed to set thread stack alloc caps: {:?}", e);
        // }

        std::thread::Builder::new()
            .stack_size(AUDIO_STACK_SIZE)
            .spawn(move || {
                log::info!(
                    "Starting audio worker thread in core {:?}",
                    esp_idf_svc::hal::cpu::core()
                );
                let r = worker.run(rx1, evt_tx_);
                if let Err(e) = r {
                    log::error!("Audio worker error: {:?}", e);
                }
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn audio worker thread: {:?}", e))?;

        if cfg!(feature = "cube") {
            let mut vol_up_btn = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio10)?;
            vol_up_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_up_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_up = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_up_btn.wait_for_falling_edge().await;
                    log::info!("Button vol up pressed {:?}", vol_up_btn.get_level());
                    if evt_tx_vol_up
                        .send(app::Event::Event(app::Event::VOL_UP))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_UP event");
                        break;
                    }
                }
            });

            let mut vol_down_btn =
                esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio39)?;
            vol_down_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_down_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_down = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_down_btn.wait_for_falling_edge().await;
                    log::info!("Button vol down pressed {:?}", vol_down_btn.get_level());
                    if evt_tx_vol_down
                        .send(app::Event::Event(app::Event::VOL_DOWN))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_DOWN event");
                        break;
                    }
                }
            });
        } else if cfg!(feature = "cube2") {
            let mut vol_up_btn = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio40)?;
            vol_up_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_up_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_up = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_up_btn.wait_for_falling_edge().await;
                    log::info!("Button vol up pressed {:?}", vol_up_btn.get_level());
                    if evt_tx_vol_up
                        .send(app::Event::Event(app::Event::VOL_UP))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_UP event");
                        break;
                    }
                }
            });

            let mut vol_down_btn =
                esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio39)?;
            vol_down_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_down_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_down = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_down_btn.wait_for_falling_edge().await;
                    log::info!("Button vol down pressed {:?}", vol_down_btn.get_level());
                    if evt_tx_vol_down
                        .send(app::Event::Event(app::Event::VOL_DOWN))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_DOWN event");
                        break;
                    }
                }
            });
        } else {
            let mut vol_up_btn = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio38)?;
            vol_up_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_up_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_up = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_up_btn.wait_for_falling_edge().await;
                    log::info!("Button vol up pressed {:?}", vol_up_btn.get_level());
                    if evt_tx_vol_up
                        .send(app::Event::Event(app::Event::VOL_UP))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_UP event");
                        break;
                    }
                }
            });

            let mut vol_down_btn =
                esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio39)?;
            vol_down_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
            vol_down_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

            let evt_tx_vol_down = evt_tx.clone();
            b.spawn(async move {
                loop {
                    let _ = vol_down_btn.wait_for_falling_edge().await;
                    log::info!("Button vol down pressed {:?}", vol_down_btn.get_level());
                    if evt_tx_vol_down
                        .send(app::Event::Event(app::Event::VOL_DOWN))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send VOL_DOWN event");
                        break;
                    }
                }
            });
        }
    }

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
