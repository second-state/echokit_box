//! 设备信息上报模块
//!
//! 提供设备固件版本上报功能，用于 OTA 升级后通知服务器新版本。

use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Read;
use serde::{Deserialize, Serialize};

/// 上报错误类型
#[derive(Debug)]
pub enum ReportError {
    /// HTTP 请求失败
    HttpError(String),
    /// JSON 解析失败
    ParseError(String),
    /// 服务端错误
    ServerError(u16, String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::HttpError(msg) => write!(f, "HTTP 错误: {}", msg),
            ReportError::ParseError(msg) => write!(f, "解析错误: {}", msg),
            ReportError::ServerError(code, msg) => write!(f, "服务端错误 {}: {}", code, msg),
        }
    }
}

/// 设备信息上报请求体
#[derive(Debug, Serialize)]
struct ReportRequest {
    device_id: String,
    mac_address: String,
    firmware_version: String,
}

/// 设备信息上报响应
#[derive(Debug, Deserialize)]
struct ReportResponse {
    status: String,
}

/// 将 WebSocket URL 转换为 HTTP URL
///
/// - `ws://host:port/ws` -> `http://host:port`
/// - `wss://host:port/ws` -> `https://host:port`
/// - 其他格式保持不变
fn convert_ws_to_http(url: &str) -> String {
    let url = url.trim();

    // 替换协议
    let http_url = if url.starts_with("wss://") {
        url.replacen("wss://", "https://", 1)
    } else if url.starts_with("ws://") {
        url.replacen("ws://", "http://", 1)
    } else {
        return url.to_string();
    };

    // 移除路径部分（如 /ws）
    if let Some(scheme_end) = http_url.find("://") {
        let after_scheme = &http_url[scheme_end + 3..];
        if let Some(path_start) = after_scheme.find('/') {
            return http_url[..scheme_end + 3 + path_start].to_string();
        }
    }

    http_url
}

/// 上报设备固件版本到服务器
///
/// # 参数
/// - `server_url`: 服务器 URL（支持 ws:// 或 http:// 格式）
/// - `device_id`: 设备 ID（12 位小写十六进制）
/// - `mac_address`: MAC 地址（12 位小写十六进制）
/// - `firmware_version`: 固件版本号
///
/// # 返回
/// - `Ok(())`: 上报成功
/// - `Err(ReportError)`: 上报失败
///
/// # 示例
/// ```ignore
/// report_firmware_version(
///     "ws://192.168.1.100:10086/ws",
///     "aabbccddeeff",
///     "aabbccddeeff",
///     "1.0.0",
/// )?;
/// ```
pub fn report_firmware_version(
    server_url: &str,
    device_id: &str,
    mac_address: &str,
    firmware_version: &str,
) -> Result<(), ReportError> {
    let base_url = convert_ws_to_http(server_url);
    let url = format!("{}/api/devices/report", base_url.trim_end_matches('/'));

    log::info!(
        "[Report] 上报固件版本: url={}, device_id={}, firmware={}",
        url,
        device_id,
        firmware_version
    );

    // 构建请求体
    let request_body = ReportRequest {
        device_id: device_id.to_lowercase(),
        mac_address: mac_address.to_lowercase(),
        firmware_version: firmware_version.to_string(),
    };

    let body_json = serde_json::to_string(&request_body)
        .map_err(|e| ReportError::ParseError(format!("序列化请求体失败: {:?}", e)))?;

    log::info!("[Report] 请求体: {}", body_json);

    // 创建 HTTP 连接
    let config = HttpConfig {
        timeout: Some(std::time::Duration::from_secs(10)),
        ..Default::default()
    };

    let mut conn = EspHttpConnection::new(&config)
        .map_err(|e| ReportError::HttpError(format!("创建连接失败: {:?}", e)))?;

    let body_bytes = body_json.as_bytes();
    let content_length = body_bytes.len().to_string();

    // 发送 POST 请求
    conn.initiate_request(
        Method::Post,
        &url,
        &[
            ("Content-Type", "application/json"),
            ("Content-Length", &content_length),
        ],
    )
    .map_err(|e| ReportError::HttpError(format!("发送请求失败: {:?}", e)))?;

    // 写入请求体
    use esp_idf_svc::io::Write;
    conn.write_all(body_bytes)
        .map_err(|e| ReportError::HttpError(format!("写入请求体失败: {:?}", e)))?;

    conn.initiate_response()
        .map_err(|e| ReportError::HttpError(format!("获取响应失败: {:?}", e)))?;

    let status = conn.status();
    log::info!("[Report] 响应状态码: {}", status);

    // 读取响应体
    let mut buf = [0u8; 512];
    let mut response_data = Vec::new();

    loop {
        match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response_data.extend_from_slice(&buf[..n]),
            Err(e) => {
                return Err(ReportError::HttpError(format!("读取响应失败: {:?}", e)));
            }
        }
    }

    let response_str = String::from_utf8_lossy(&response_data);
    log::info!("[Report] 响应内容: {}", response_str);

    match status {
        200 => {
            log::info!("[Report] 固件版本上报成功");
            Ok(())
        }
        _ => Err(ReportError::ServerError(status, response_str.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::convert_ws_to_http;

    #[test]
    fn test_convert_ws_to_http() {
        assert_eq!(
            convert_ws_to_http("ws://192.168.0.103:10086/ws"),
            "http://192.168.0.103:10086"
        );

        assert_eq!(
            convert_ws_to_http("wss://proxy.echokit.dev/ws"),
            "https://proxy.echokit.dev"
        );

        assert_eq!(
            convert_ws_to_http("http://localhost:3000"),
            "http://localhost:3000"
        );
    }
}
