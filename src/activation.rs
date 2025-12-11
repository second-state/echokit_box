//! 设备激活模块
//!
//! 实现 6 位数字激活码绑定流程：
//! 1. 设备请求激活码
//! 2. 显示激活码给用户
//! 3. 轮询验证激活状态
//! 4. 激活成功后保存配置

use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Read;
use serde::{Deserialize, Serialize};

/// 激活码请求响应
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationResponse {
    /// 6 位数字激活码
    pub code: String,
    /// 64 字符挑战值（用于验证）
    pub challenge: String,
    /// 有效期（秒）
    pub expires_in: u64,
}

/// 验证请求体
#[derive(Debug, Serialize)]
struct VerifyRequest {
    device_id: String,
    challenge: String,
    firmware_version: String,
}

/// 验证成功响应
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyBoundResponse {
    pub status: String,
    pub user_id: String,
    pub device_name: String,
    pub proxy_url: String,
}

/// 验证等待响应
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPendingResponse {
    pub status: String,
    pub retry_after_ms: u64,
}

/// 验证响应（统一）
#[derive(Debug)]
pub enum VerifyResponse {
    /// 激活成功
    Bound(VerifyBoundResponse),
    /// 等待用户确认
    Pending(VerifyPendingResponse),
}

/// 激活错误
#[derive(Debug)]
pub enum ActivationError {
    /// HTTP 请求失败
    HttpError(String),
    /// JSON 解析失败
    ParseError(String),
    /// 服务端错误
    ServerError(u16, String),
    /// 激活超时
    Timeout,
    /// Challenge 不匹配
    InvalidChallenge,
    /// 激活码已过期
    Expired,
}

impl std::fmt::Display for ActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivationError::HttpError(msg) => write!(f, "HTTP 错误: {}", msg),
            ActivationError::ParseError(msg) => write!(f, "解析错误: {}", msg),
            ActivationError::ServerError(code, msg) => write!(f, "服务端错误 {}: {}", code, msg),
            ActivationError::Timeout => write!(f, "激活超时"),
            ActivationError::InvalidChallenge => write!(f, "Challenge 不匹配"),
            ActivationError::Expired => write!(f, "激活码已过期"),
        }
    }
}

/// 激活会话信息
pub struct ActivationSession {
    /// 设备 ID（12 位小写十六进制）
    pub device_id: String,
    /// Proxy HTTP URL（用于激活 API）
    pub proxy_url: String,
    /// 激活码
    pub code: String,
    /// Challenge
    pub challenge: String,
    /// 超时时间（毫秒）
    pub timeout_ms: u64,
    /// 固件版本
    pub firmware_version: String,
}

impl ActivationSession {
    /// 创建新的激活会话
    ///
    /// server_url 可以是以下格式：
    /// - `ws://host:port/ws` -> 转换为 `http://host:port`
    /// - `wss://host:port/ws` -> 转换为 `https://host:port`
    /// - `http://host:port` -> 保持不变
    /// - `https://host:port` -> 保持不变
    pub fn new(device_id: String, server_url: String, firmware_version: String) -> Self {
        let proxy_url = convert_ws_to_http(&server_url);
        log::info!("[Activation] URL 转换: {} -> {}", server_url, proxy_url);
        log::info!("[Activation] 固件版本: {}", firmware_version);

        Self {
            device_id,
            proxy_url,
            code: String::new(),
            challenge: String::new(),
            timeout_ms: 300000, // 默认 5 分钟
            firmware_version,
        }
    }

    /// 请求激活码
    ///
    /// GET /api/activation?device_id={device_id}
    pub fn request_activation_code(&mut self) -> Result<(), ActivationError> {
        let url = format!(
            "{}/api/activation?device_id={}",
            self.proxy_url.trim_end_matches('/'),
            self.device_id
        );

        log::info!("[Activation] 请求激活码: {}", url);

        // 创建 HTTP 连接
        let config = HttpConfig {
            timeout: Some(std::time::Duration::from_secs(10)),
            ..Default::default()
        };

        let mut conn = EspHttpConnection::new(&config)
            .map_err(|e| ActivationError::HttpError(format!("创建连接失败: {:?}", e)))?;

        // 发送 GET 请求
        conn.initiate_request(Method::Get, &url, &[])
            .map_err(|e| ActivationError::HttpError(format!("发送请求失败: {:?}", e)))?;

        conn.initiate_response()
            .map_err(|e| ActivationError::HttpError(format!("获取响应失败: {:?}", e)))?;

        let status = conn.status();
        log::info!("[Activation] 响应状态码: {}", status);

        // 读取响应体
        let mut buf = [0u8; 1024];
        let mut response_data = Vec::new();

        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => response_data.extend_from_slice(&buf[..n]),
                Err(e) => {
                    return Err(ActivationError::HttpError(format!("读取响应失败: {:?}", e)));
                }
            }
        }

        let response_str = String::from_utf8_lossy(&response_data);
        log::info!("[Activation] 响应内容: {}", response_str);

        if status != 200 {
            return Err(ActivationError::ServerError(
                status,
                response_str.to_string(),
            ));
        }

        // 解析 JSON
        let resp: ActivationResponse = serde_json::from_slice(&response_data)
            .map_err(|e| ActivationError::ParseError(format!("JSON 解析失败: {:?}", e)))?;

        // 保存到会话（expires_in 是秒，转换为毫秒）
        self.code = resp.code;
        self.challenge = resp.challenge;
        self.timeout_ms = resp.expires_in * 1000;

        log::info!(
            "[Activation] 获取激活码成功: code={}, expires_in={}s ({}ms)",
            self.code,
            resp.expires_in,
            self.timeout_ms
        );

        Ok(())
    }

    /// 验证激活状态
    ///
    /// POST /api/activation/verify
    /// { device_id, challenge }
    pub fn verify_activation(&self) -> Result<VerifyResponse, ActivationError> {
        let url = format!(
            "{}/api/activation/verify",
            self.proxy_url.trim_end_matches('/')
        );

        log::info!("[Activation] 验证激活状态: {}", url);

        // 构建请求体
        let request_body = VerifyRequest {
            device_id: self.device_id.clone(),
            challenge: self.challenge.clone(),
            firmware_version: self.firmware_version.clone(),
        };

        let body_json = serde_json::to_string(&request_body)
            .map_err(|e| ActivationError::ParseError(format!("序列化请求体失败: {:?}", e)))?;

        log::info!("[Activation] 请求体: {}", body_json);

        // 创建 HTTP 连接
        let config = HttpConfig {
            timeout: Some(std::time::Duration::from_secs(10)),
            ..Default::default()
        };

        let mut conn = EspHttpConnection::new(&config)
            .map_err(|e| ActivationError::HttpError(format!("创建连接失败: {:?}", e)))?;

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
        .map_err(|e| ActivationError::HttpError(format!("发送请求失败: {:?}", e)))?;

        // 写入请求体
        use esp_idf_svc::io::Write;
        conn.write_all(body_bytes)
            .map_err(|e| ActivationError::HttpError(format!("写入请求体失败: {:?}", e)))?;

        conn.initiate_response()
            .map_err(|e| ActivationError::HttpError(format!("获取响应失败: {:?}", e)))?;

        let status = conn.status();
        log::info!("[Activation] 验证响应状态码: {}", status);

        // 读取响应体
        let mut buf = [0u8; 1024];
        let mut response_data = Vec::new();

        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => response_data.extend_from_slice(&buf[..n]),
                Err(e) => {
                    return Err(ActivationError::HttpError(format!("读取响应失败: {:?}", e)));
                }
            }
        }

        let response_str = String::from_utf8_lossy(&response_data);
        log::info!("[Activation] 验证响应内容: {}", response_str);

        match status {
            200 => {
                // 激活成功
                let resp: VerifyBoundResponse = serde_json::from_slice(&response_data)
                    .map_err(|e| ActivationError::ParseError(format!("JSON 解析失败: {:?}", e)))?;
                log::info!(
                    "[Activation] 激活成功: user={}, device_name={}",
                    resp.user_id,
                    resp.device_name
                );
                Ok(VerifyResponse::Bound(resp))
            }
            202 => {
                // 等待确认
                let resp: VerifyPendingResponse = serde_json::from_slice(&response_data)
                    .map_err(|e| ActivationError::ParseError(format!("JSON 解析失败: {:?}", e)))?;
                log::info!("[Activation] 等待用户确认，{}ms 后重试", resp.retry_after_ms);
                Ok(VerifyResponse::Pending(resp))
            }
            401 => Err(ActivationError::InvalidChallenge),
            404 | 410 => Err(ActivationError::Expired),
            _ => Err(ActivationError::ServerError(
                status,
                response_str.to_string(),
            )),
        }
    }

    /// 获取激活码用于显示
    pub fn get_code(&self) -> &str {
        &self.code
    }

    /// 获取激活码的各个数字（用于语音播报）
    pub fn get_code_digits(&self) -> Vec<char> {
        self.code.chars().collect()
    }
}

/// 激活流程配置
pub struct ActivationConfig {
    /// 轮询间隔（毫秒）
    pub poll_interval_ms: u64,
    /// 最大轮询次数
    pub max_poll_count: u32,
}

impl Default for ActivationConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 5000,  // 5 秒
            max_poll_count: 60,      // 最多 60 次 = 5 分钟
        }
    }
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
    // 找到 host:port 之后的第一个 /
    if let Some(scheme_end) = http_url.find("://") {
        let after_scheme = &http_url[scheme_end + 3..];
        if let Some(path_start) = after_scheme.find('/') {
            // 只保留 scheme + host:port
            return http_url[..scheme_end + 3 + path_start].to_string();
        }
    }

    http_url
}

#[cfg(test)]
mod tests {
    use super::{convert_ws_to_http, ActivationSession};

    #[test]
    fn test_code_digits() {
        let mut session = ActivationSession::new(
            "aabbccddeeff".to_string(),
            "http://localhost:8081".to_string(),
            "1.0.0".to_string(),
        );
        session.code = "123456".to_string();

        let digits = session.get_code_digits();
        assert_eq!(digits, vec!['1', '2', '3', '4', '5', '6']);
    }

    #[test]
    fn test_convert_ws_to_http() {
        // ws:// with path
        assert_eq!(
            convert_ws_to_http("ws://192.168.0.103:10086/ws"),
            "http://192.168.0.103:10086"
        );

        // wss:// with path
        assert_eq!(
            convert_ws_to_http("wss://proxy.echokit.dev/ws"),
            "https://proxy.echokit.dev"
        );

        // ws:// without path
        assert_eq!(
            convert_ws_to_http("ws://localhost:8081"),
            "http://localhost:8081"
        );

        // http:// unchanged
        assert_eq!(
            convert_ws_to_http("http://localhost:3000"),
            "http://localhost:3000"
        );

        // https:// unchanged
        assert_eq!(
            convert_ws_to_http("https://api.echokit.dev"),
            "https://api.echokit.dev"
        );
    }
}
