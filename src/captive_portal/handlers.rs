//! HTTP 路由处理器

use std::io::Read;
use std::sync::{Arc, Mutex};

use esp_idf_svc::{
    http::{
        server::{EspHttpConnection, EspHttpServer, Request},
        Method,
    },
    io::Write,
    nvs::EspDefaultNvs,
};

use super::html;
use crate::Setting;

/// 注册所有 HTTP 路由
pub fn register_routes(
    server: &mut EspHttpServer<'_>,
    setting: Arc<Mutex<(Setting, EspDefaultNvs)>>,
) -> anyhow::Result<()> {
    // 主页
    server.fn_handler("/", Method::Get, |req| {
        handle_index(req)
    })?;

    // 设备状态 API
    let setting_status = setting.clone();
    server.fn_handler("/api/status", Method::Get, move |req| {
        handle_status(req, &setting_status)
    })?;

    // 获取配置 API
    let setting_get = setting.clone();
    server.fn_handler("/api/config", Method::Get, move |req| {
        handle_config_get(req, &setting_get)
    })?;

    // 保存配置 API
    let setting_post = setting.clone();
    server.fn_handler("/api/config", Method::Post, move |req| {
        handle_config_post(req, &setting_post)
    })?;

    // Captive Portal 检测端点
    server.fn_handler::<anyhow::Error, _>("/generate_204", Method::Get, |req| {
        // Android 检测
        req.into_response(204, None, &[])?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/hotspot-detect.html", Method::Get, |req| {
        // iOS/macOS 检测
        let mut resp = req.into_ok_response()?;
        resp.write_all(b"<HTML><HEAD><TITLE>Success</TITLE></HEAD><BODY>Success</BODY></HTML>")?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/connecttest.txt", Method::Get, |req| {
        // Windows 检测
        let mut resp = req.into_ok_response()?;
        resp.write_all(b"Microsoft Connect Test")?;
        Ok(())
    })?;

    Ok(())
}

fn handle_index(req: Request<&mut EspHttpConnection<'_>>) -> anyhow::Result<()> {
    let mut resp = req.into_ok_response()?;
    resp.write_all(html::INDEX_HTML.as_bytes())?;
    Ok(())
}

fn handle_status(
    req: Request<&mut EspHttpConnection<'_>>,
    setting: &Arc<Mutex<(Setting, EspDefaultNvs)>>,
) -> anyhow::Result<()> {
    let setting = setting.lock().unwrap();
    let version = env!("CARGO_PKG_VERSION");

    let json = format!(
        r#"{{"version":"{}","ssid":"{}","server_url":"{}"}}"#,
        version,
        setting.0.ssid,
        setting.0.server_url
    );

    let mut resp = req.into_ok_response()?;
    resp.write_all(json.as_bytes())?;
    Ok(())
}

fn handle_config_get(
    req: Request<&mut EspHttpConnection<'_>>,
    setting: &Arc<Mutex<(Setting, EspDefaultNvs)>>,
) -> anyhow::Result<()> {
    let setting = setting.lock().unwrap();

    // 密码脱敏显示
    let pass_masked = if setting.0.pass.is_empty() {
        "".to_string()
    } else {
        "*".repeat(setting.0.pass.len().min(8))
    };

    let json = format!(
        r#"{{"ssid":"{}","pass":"{}","server_url":"{}"}}"#,
        setting.0.ssid,
        pass_masked,
        setting.0.server_url
    );

    let mut resp = req.into_ok_response()?;
    resp.write_all(json.as_bytes())?;
    Ok(())
}

fn handle_config_post(
    mut req: Request<&mut EspHttpConnection<'_>>,
    setting: &Arc<Mutex<(Setting, EspDefaultNvs)>>,
) -> anyhow::Result<()> {
    // 读取请求体
    let mut buf = [0u8; 1024];
    let len = req.read(&mut buf)?;
    let body = std::str::from_utf8(&buf[..len])?;

    log::info!("Received config: {}", body);

    // 解析 JSON
    #[derive(serde::Deserialize)]
    struct ConfigRequest {
        ssid: String,
        pass: String,
        server_url: String,
    }

    let config: ConfigRequest = serde_json::from_str(body)?;

    // 保存到 NVS
    {
        let mut setting = setting.lock().unwrap();

        if let Err(e) = setting.1.set_str("ssid", &config.ssid) {
            log::error!("Failed to save ssid: {:?}", e);
        } else {
            setting.0.ssid = config.ssid;
        }

        if let Err(e) = setting.1.set_str("pass", &config.pass) {
            log::error!("Failed to save pass: {:?}", e);
        } else {
            setting.0.pass = config.pass;
        }

        if let Err(e) = setting.1.set_str("server_url", &config.server_url) {
            log::error!("Failed to save server_url: {:?}", e);
        } else {
            setting.0.server_url = config.server_url;
        }
    }

    // 返回成功响应
    let mut resp = req.into_ok_response()?;
    resp.write_all(br#"{"status":"ok","message":"Configuration saved. Rebooting..."}"#)?;

    // 延迟重启
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(2));
        unsafe { esp_idf_svc::sys::esp_restart() }
    });

    Ok(())
}
