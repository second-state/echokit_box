use std::sync::{Arc, Mutex};

use esp_idf_svc::eventloop::EspSystemEventLoop;

mod app;
mod audio;
mod bt;
mod captive_portal;
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
}

/// 配网模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProvisioningMode {
    /// 无需配网
    None,
    /// Web 配网（SoftAP + HTTP）
    Web,
    /// BLE 配网
    Ble,
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = esp_idf_svc::hal::prelude::Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20)?;
    let partition = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
    let nvs = esp_idf_svc::nvs::EspDefaultNvs::new(partition, "setting", true)?;

    let state = nvs.get_u8("state").ok().flatten().unwrap_or(0);

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

    let mut server_url = nvs
        .get_str("server_url", &mut str_buf)
        .map_err(|e| log::error!("Failed to get server_url: {:?}", e))
        .ok()
        .flatten()
        .or(DEFAULT_SERVER_URL)
        .unwrap_or_default()
        .to_string();

    // 1MB buffer for GIF
    let has_bg = nvs.contains("background_gif").unwrap_or(false);
    let mut gif_buf = if has_bg {
        vec![0; 1024 * 1024]
    } else {
        Vec::new()
    };

    let background_gif = nvs
        .get_blob("background_gif", &mut gif_buf)?
        .unwrap_or(ui::DEFAULT_BACKGROUND);

    log::info!("SSID: {:?}", ssid);
    log::info!("PASS: {:?}", pass);
    log::info!("Server URL: {:?}", server_url);

    nvs.set_u8("state", 0).unwrap();

    log_heap();

    let (evt_tx, mut evt_rx) = tokio::sync::mpsc::channel(64);
    let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();

    crate::start_hal!(peripherals, evt_tx);

    let _ = ui::backgroud(&background_gif, boards::flush_display);

    // Configures the button
    let mut button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio0)?;
    button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    let b = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut gui = ui::UI::new(None, boards::flush_display).unwrap();

    log_heap();

    #[cfg(feature = "extra_server")]
    {
        gui.state = "Initializing...".to_string();
        gui.text = "Loading Server URL...".to_string();
        gui.display_flush().unwrap();

        while let Some(event) = evt_rx.blocking_recv() {
            if let app::Event::ServerUrl(url) = event {
                log::info!("Received ServerUrl event: {}", url);
                if !url.is_empty() {
                    server_url = url;
                }
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(500));
        gui.text = format!("Server URL: {}\nContinuing...", server_url);
        gui.display_flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2000));
    }

    // 获取 WiFi STA MAC 地址（用于 BLE 特征值和设备 ID）
    let mut wifi_sta_mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            wifi_sta_mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    let mac_address = format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        wifi_sta_mac[0], wifi_sta_mac[1], wifi_sta_mac[2],
        wifi_sta_mac[3], wifi_sta_mac[4], wifi_sta_mac[5]
    );

    // 检测配网模式
    let provisioning_mode = {
        let config_missing = ssid.is_empty() || pass.is_empty() || server_url.is_empty();
        let button_pressed = button.is_low();

        if !config_missing && !button_pressed && state != 1 {
            ProvisioningMode::None
        } else if button_pressed {
            // 检测按钮按压时长：短按 -> Web，长按（>2秒）-> BLE
            gui.state = "检测配网模式...".to_string();
            gui.text = "松开按钮: Web配网\n持续按住2秒: BLE配网".to_string();
            gui.display_flush().unwrap();

            let start = std::time::Instant::now();
            while button.is_low() && start.elapsed().as_secs() < 2 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            if button.is_low() {
                // 长按超过2秒，使用 BLE 配网
                log::info!("Long press detected, using BLE provisioning");
                ProvisioningMode::Ble
            } else {
                // 短按，使用 Web 配网
                log::info!("Short press detected, using Web provisioning");
                ProvisioningMode::Web
            }
        } else {
            // 配置缺失或 state == 1，默认使用 Web 配网
            log::info!("Config missing or state=1, using Web provisioning");
            ProvisioningMode::Web
        }
    };

    log::info!("Provisioning mode: {:?}", provisioning_mode);

    if provisioning_mode != ProvisioningMode::None {
        gif_buf.clear();
        let setting = Arc::new(Mutex::new((
            Setting {
                ssid,
                pass,
                server_url,
                background_gif: (gif_buf, false), // 1MB
            },
            nvs,
        )));

        let version = env!("CARGO_PKG_VERSION");
        let mac_suffix = &mac_address[9..].replace(":", ""); // 取后6位作为后缀

        match provisioning_mode {
            ProvisioningMode::Web => {
                // Web 配网模式
                gui.state = "Web 配网模式".to_string();
                gui.text = format!(
                    "1. 连接 WiFi: EchoKit-{}\n2. 打开浏览器访问:\n   http://192.168.4.1\n\n长按按钮切换到BLE配网",
                    mac_suffix
                );
                gui.display_qrcode("http://192.168.4.1").unwrap();

                log::info!("Starting Web provisioning...");
                let _portal = captive_portal::CaptivePortal::start(
                    peripherals.modem,
                    sysloop.clone(),
                    mac_suffix,
                    setting.clone(),
                );

                match _portal {
                    Ok(portal) => {
                        log::info!("CaptivePortal started successfully");
                        // 等待配置完成或按钮长按切换到 BLE
                        b.block_on(async {
                            loop {
                                tokio::select! {
                                    _ = button.wait_for_falling_edge() => {
                                        // 检测是否长按
                                        let start = std::time::Instant::now();
                                        while button.is_low() && start.elapsed().as_secs() < 2 {
                                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                        }
                                        if button.is_low() {
                                            log::info!("Long press detected, switching to BLE mode");
                                            break;
                                        }
                                    }
                                    _ = evt_rx.recv() => {
                                        log::info!("Received config event");
                                        break;
                                    }
                                }
                            }
                        });
                        drop(portal);
                    }
                    Err(e) => {
                        log::error!("Failed to start CaptivePortal: {:?}", e);
                        gui.state = "Web配网启动失败".to_string();
                        gui.text = format!("错误: {:?}\n按按钮重启", e);
                        gui.display_flush().unwrap();
                        b.block_on(button.wait_for_falling_edge()).unwrap();
                    }
                }
            }
            ProvisioningMode::Ble => {
                // BLE 配网模式
                let ble_addr = bt::bt(mac_address.clone(), setting.clone(), evt_tx).unwrap();
                log_heap();

                gui.state = "BLE 配网模式".to_string();
                gui.text = format!(
                    "请使用 EchoKit App 配置设备\n设备名: EchoKit-{}\n版本: {}\n\n短按按钮切换到Web配网",
                    ble_addr, version
                );
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

                b.block_on(async {
                    tokio::select! {
                        _ = button.wait_for_falling_edge() => {
                            log::info!("Button k0 pressed to enter setup");
                        }
                        _ = evt_rx.recv() => {
                            log::info!("Received event to enter setup");
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                });
            }
            ProvisioningMode::None => unreachable!(),
        }

        // 处理背景 GIF
        {
            let mut setting = setting.lock().unwrap();
            if setting.0.background_gif.1 {
                gui.text = "Testing background GIF...".to_string();
                gui.display_flush().unwrap();

                let mut new_gif = Vec::new();
                std::mem::swap(&mut setting.0.background_gif.0, &mut new_gif);

                let _ = ui::backgroud(&new_gif, boards::flush_display);
                log::info!("Background GIF set from NVS");

                gui.text = "Background GIF set OK".to_string();
                gui.display_flush().unwrap();

                setting
                    .1
                    .set_blob("background_gif", &new_gif)
                    .map_err(|e| log::error!("Failed to save background GIF to NVS: {:?}", e))
                    .unwrap();
                log::info!("Background GIF saved to NVS");
            }
        }

        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    gui.state = "Connecting to wifi...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    let _wifi = network::wifi(&ssid, &pass, peripherals.modem, sysloop.clone());
    if _wifi.is_err() {
        gui.state = "Failed to connect to wifi".to_string();
        gui.text = "Press K0 to open settings".to_string();
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        nvs.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let wifi = _wifi.unwrap();
    log_heap();

    // 使用 WiFi STA MAC 地址（与 BLE 特征值中暴露的地址一致）
    let mac = wifi.sta_netif().get_mac().unwrap();
    let dev_id = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    gui.state = "Connecting to server...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    log_heap();

    gui.state = "Failed to connect to server".to_string();
    gui.text = format!("Please check your server URL: {server_url}\nPress K0 to open settings");
    let server = b.block_on(ws::Server::new(dev_id, server_url));
    if server.is_err() {
        log::info!("Failed to connect to server: {:?}", server.err());
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        nvs.set_u8("state", 1).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let server = server.unwrap();

    crate::start_audio_workers!(peripherals, rx1, evt_tx.clone(), &b);

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
