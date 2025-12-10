//! HTTP 服务器和 SoftAP 管理

use std::sync::{Arc, Mutex};

use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    http::server::{Configuration, EspHttpServer},
    ipv4::{self, Mask, Subnet},
    netif::{EspNetif, NetifConfiguration, NetifStack},
    nvs::EspDefaultNvs,
    wifi::{
        AccessPointConfiguration, AuthMethod, BlockingWifi, Configuration as WifiConfig, EspWifi,
        WifiDriver,
    },
};

use super::handlers;
use crate::Setting;

/// AP 模式的固定 IP 地址
const AP_IP: ipv4::Ipv4Addr = ipv4::Ipv4Addr::new(192, 168, 4, 1);
const AP_GATEWAY: ipv4::Ipv4Addr = ipv4::Ipv4Addr::new(192, 168, 4, 1);
const AP_NETMASK: Mask = Mask(24);

pub struct CaptivePortal<'a> {
    _wifi: BlockingWifi<EspWifi<'a>>,
    _server: EspHttpServer<'a>,
}

impl<'a> CaptivePortal<'a> {
    pub fn start(
        modem: Modem,
        sysloop: EspSystemEventLoop,
        mac_suffix: &str,
        setting: Arc<Mutex<(Setting, EspDefaultNvs)>>,
    ) -> anyhow::Result<Self> {
        // 1. 配置并启动 SoftAP
        let wifi = Self::start_ap(modem, sysloop, mac_suffix)?;
        log::info!("SoftAP started: EchoKit-{}", mac_suffix);

        // 2. 启动 HTTP 服务器
        let server = Self::start_http_server(setting)?;
        log::info!("HTTP server started on 192.168.4.1:80");

        Ok(Self {
            _wifi: wifi,
            _server: server,
        })
    }

    fn start_ap(
        modem: Modem,
        sysloop: EspSystemEventLoop,
        mac_suffix: &str,
    ) -> anyhow::Result<BlockingWifi<EspWifi<'a>>> {
        // 配置 AP 网络接口，使用固定 IP 192.168.4.1
        let ap_netif_config = NetifConfiguration {
            ip_configuration: Some(ipv4::Configuration::Router(ipv4::RouterConfiguration {
                subnet: Subnet {
                    gateway: AP_GATEWAY,
                    mask: AP_NETMASK,
                },
                dhcp_enabled: true,
                dns: Some(AP_IP),
                secondary_dns: None,
            })),
            ..NetifConfiguration::wifi_default_router()
        };

        // 创建自定义 AP netif
        let ap_netif = EspNetif::new_with_conf(&ap_netif_config)?;

        // 创建 WiFi 驱动
        let driver = WifiDriver::new(modem, sysloop.clone(), None)?;

        // 创建 STA netif（虽然 AP 模式不使用，但 API 需要）
        let sta_netif = EspNetif::new(NetifStack::Sta)?;

        // 使用 wrap_all 创建 EspWifi
        let mut wifi = BlockingWifi::wrap(
            EspWifi::wrap_all(driver, sta_netif, ap_netif)?,
            sysloop,
        )?;

        let ssid = format!("EchoKit-{}", mac_suffix);
        let ap_config = AccessPointConfiguration {
            ssid: ssid.as_str().try_into().unwrap(),
            ssid_hidden: false,
            channel: 1,
            auth_method: AuthMethod::None,
            max_connections: 4,
            ..Default::default()
        };

        wifi.set_configuration(&WifiConfig::AccessPoint(ap_config))?;
        wifi.start()?;

        Ok(wifi)
    }

    fn start_http_server(
        setting: Arc<Mutex<(Setting, EspDefaultNvs)>>,
    ) -> anyhow::Result<EspHttpServer<'a>> {
        let config = Configuration {
            stack_size: 8192,
            max_uri_handlers: 12,
            ..Default::default()
        };

        let mut server = EspHttpServer::new(&config)?;

        handlers::register_routes(&mut server, setting)?;

        Ok(server)
    }

    pub fn get_ap_ip() -> &'static str {
        "192.168.4.1"
    }
}
