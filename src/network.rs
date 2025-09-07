use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::peripheral,
    http::{client::EspHttpConnection, Method},
    wifi::{AuthMethod, BlockingWifi, EspWifi},
};
use log::info;

pub fn wifi(
    ssid: &str,
    pass: &str,
    modem: impl peripheral::Peripheral<P = esp_idf_svc::hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<Box<EspWifi<'static>>> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if ssid.is_empty() {
        anyhow::bail!("Missing WiFi name")
    }
    if pass.is_empty() {
        auth_method = AuthMethod::None;
        info!("Wifi password is empty");
    }
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        esp_idf_svc::wifi::ClientConfiguration {
            ssid: ssid
                .try_into()
                .expect("Could not parse the given SSID into WiFi config"),
            password: pass
                .try_into()
                .expect("Could not parse the given password into WiFi config"),
            auth_method,
            ..Default::default()
        },
    ))?;

    wifi.start()?;

    info!("Connecting wifi...");

    wifi.connect()?;

    info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Wifi DHCP info: {ip_info:?}");

    Ok(Box::new(esp_wifi))
}

#[allow(unused)]
pub fn http_get(url: &str) -> anyhow::Result<EspHttpConnection> {
    let configuration = esp_idf_svc::http::client::Configuration::default();
    let mut conn = EspHttpConnection::new(&configuration)?;
    conn.initiate_request(Method::Get, url, &[])?;

    conn.initiate_response()?;

    Ok(conn)
}

#[allow(unused)]
pub fn http_post(url: &str, data: &[u8]) -> anyhow::Result<EspHttpConnection> {
    let configuration = esp_idf_svc::http::client::Configuration::default();
    let len = data.len().to_string();
    let mut conn = EspHttpConnection::new(&configuration)?;
    conn.initiate_request(Method::Post, url, &[("Content-Length", &len)])?;

    let mut offset = 0;

    while offset < data.len() {
        offset += conn.write(&data[offset..])?;
        log::info!("Wrote {offset} bytes");
    }

    conn.initiate_response()?;

    Ok(conn)
}
