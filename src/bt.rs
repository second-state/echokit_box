use std::sync::{Arc, Mutex};

use esp32_nimble::{utilities::BleUuid, uuid128, BLEAdvertisementData, NimbleProperties};

const SERVICE_ID: BleUuid = uuid128!("623fa3e2-631b-4f8f-a6e7-a7b09c03e7e0");
const SSID_ID: BleUuid = uuid128!("1fda4d6e-2f14-42b0-96fa-453bed238375");
const PASS_ID: BleUuid = uuid128!("a987ab18-a940-421a-a1d7-b94ee22bccbe");
const SERVER_URL_ID: BleUuid = uuid128!("cef520a9-bcb5-4fc6-87f7-82804eee2b20");
const BACKGROUND_GIF_ID: BleUuid = uuid128!("d1f3b2c4-5e6f-4a7b-8c9d-0e1f2a3b4c5d");
const RESET_ID: BleUuid = uuid128!("f0e1d2c3-b4a5-6789-0abc-def123456789");

pub fn bt(
    setting: Arc<Mutex<(super::Setting, esp_idf_svc::nvs::EspDefaultNvs)>>,
    evt_tx: tokio::sync::mpsc::Sender<crate::app::Event>,
) -> anyhow::Result<String> {
    let ble_device = esp32_nimble::BLEDevice::take();
    let ble_addr = ble_device.get_addr()?.to_string();
    let ble_advertising = ble_device.get_advertising();

    let server = ble_device.get_server();
    server.on_connect(|server, desc| {
        log::info!("Client connected: {:?}", desc);

        server
            .update_conn_params(desc.conn_handle(), 24, 48, 0, 60)
            .unwrap();

        if server.connected_count() < (esp_idf_svc::sys::CONFIG_BT_NIMBLE_MAX_CONNECTIONS as _) {
            log::info!("Multi-connect support: start advertising");
            ble_advertising.lock().start().unwrap();
        }
    });

    server.on_disconnect(|_desc, reason| {
        log::info!("Client disconnected ({:?})", reason);
    });

    let service = server.create_service(SERVICE_ID);

    let setting1 = setting.clone();
    let setting2 = setting.clone();

    let ssid_characteristic = service
        .lock()
        .create_characteristic(SSID_ID, NimbleProperties::READ | NimbleProperties::WRITE);
    ssid_characteristic
        .lock()
        .on_read(move |c, _| {
            log::info!("Read from SSID characteristic");
            let setting = setting1.lock().unwrap();
            c.set_value(setting.0.ssid.as_bytes());
        })
        .on_write(move |args| {
            log::info!(
                "Wrote to SSID characteristic: {:?} -> {:?}",
                args.current_data(),
                args.recv_data()
            );
            if let Ok(new_ssid) = String::from_utf8(args.recv_data().to_vec()) {
                log::info!("New SSID: {}", new_ssid);
                let mut setting = setting2.lock().unwrap();
                if let Err(e) = setting.1.set_str("ssid", &new_ssid) {
                    log::error!("Failed to save SSID to NVS: {:?}", e);
                } else {
                    setting.0.ssid = new_ssid;
                }
            } else {
                log::error!("Failed to parse new SSID from bytes.");
            }
        });

    let setting1 = setting.clone();
    let setting2 = setting.clone();
    let pass_characteristic = service
        .lock()
        .create_characteristic(PASS_ID, NimbleProperties::READ | NimbleProperties::WRITE);
    pass_characteristic
        .lock()
        .on_read(move |c, _| {
            log::info!("Read from pass characteristic");
            let setting = setting1.lock().unwrap();
            c.set_value(setting.0.pass.as_bytes());
        })
        .on_write(move |args| {
            log::info!(
                "Wrote to pass characteristic: {:?} -> {:?}",
                args.current_data(),
                args.recv_data()
            );
            if let Ok(new_pass) = String::from_utf8(args.recv_data().to_vec()) {
                log::info!("New pass: {}", new_pass);
                let mut setting = setting2.lock().unwrap();
                if let Err(e) = setting.1.set_str("pass", &new_pass) {
                    log::error!("Failed to save pass to NVS: {:?}", e);
                } else {
                    setting.0.pass = new_pass;
                }
            } else {
                log::error!("Failed to parse new pass from bytes.");
            }
        });

    let setting = setting.clone();
    let setting_ = setting.clone();
    let setting_gif = setting.clone();

    let server_url_characteristic = service.lock().create_characteristic(
        SERVER_URL_ID,
        NimbleProperties::READ | NimbleProperties::WRITE,
    );
    server_url_characteristic
        .lock()
        .on_read(move |c, _| {
            log::info!("Read from server URL characteristic");
            let setting = setting.lock().unwrap();
            c.set_value(setting.0.server_url.as_bytes());
        })
        .on_write(move |args| {
            log::info!(
                "Wrote to server URL characteristic: {:?} -> {:?}",
                args.current_data(),
                args.recv_data()
            );
            if let Ok(mut new_server_url) = String::from_utf8(args.recv_data().to_vec()) {
                log::info!("New server URL: {}", new_server_url);
                if !new_server_url.ends_with("/") {
                    new_server_url.push('/');
                }
                let mut setting = setting_.lock().unwrap();
                if let Err(e) = setting.1.set_str("server_url", &new_server_url) {
                    log::error!("Failed to save server URL to NVS: {:?}", e);
                } else {
                    setting.0.server_url = new_server_url;
                }
            } else {
                log::error!("Failed to parse new server URL from bytes.");
            }
        });

    let background_gif_characteristic = service
        .lock()
        .create_characteristic(BACKGROUND_GIF_ID, NimbleProperties::WRITE);
    background_gif_characteristic.lock().on_write(move |args| {
        let gif_chunk = args.recv_data();

        if gif_chunk.len() <= 1024 * 1024 && gif_chunk.len() > 0 {
            log::info!("New background GIF received, size: {}", gif_chunk.len());
            let mut setting = setting_gif.lock().unwrap();
            setting.0.background_gif.0.extend_from_slice(gif_chunk);
            if gif_chunk.len() < 512 {
                setting.0.background_gif.1 = true; // Mark as valid
            }
        } else {
            log::error!("Failed to parse new background GIF from bytes.");
        }
    });

    let reset_characteristic = service
        .lock()
        .create_characteristic(RESET_ID, NimbleProperties::WRITE);
    reset_characteristic.lock().on_write(move |args| {
        let reset_cmd = args.recv_data();
        if reset_cmd == b"RESET" {
            evt_tx
                .blocking_send(crate::app::Event::Event(crate::app::Event::RESET))
                .unwrap();
        } else {
            log::warn!("Invalid reset command received via BLE.");
        }
    });

    ble_advertising.lock().set_data(
        BLEAdvertisementData::new()
            .name(&format!("EchoKit-{}", ble_addr))
            .add_service_uuid(SERVICE_ID),
    )?;
    ble_advertising.lock().start()?;
    Ok(ble_addr)
}
