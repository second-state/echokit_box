//! SoftAP + Web 配网模块
//!
//! 提供基于 WiFi AP 模式和 HTTP 服务器的设备配网功能。

mod handlers;
mod html;
mod server;

pub use server::CaptivePortal;
