#[allow(unused)]
fn print_stack_high() {
    let stack_high =
        unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark2(std::ptr::null_mut()) };
    log::info!("Stack high: {}", stack_high);
}

use crate::{app::Event, protocol::ServerEvent};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_websockets::{Connector, Message};

pub struct Server {
    timeout: std::time::Duration,
    ws: tokio_websockets::WebSocketStream<tokio_websockets::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Server {
    pub async fn new(uri: String) -> anyhow::Result<Self> {
        log::info!("uri: {}", uri);

        let (scheme, rest) = uri.split_once("://").unwrap();
        let default_port = match scheme {
            "wss" => 443,
            _ => 80,
        };

        // 提取 host[:port] 部分
        let host_port = rest.split('/').next().unwrap();
        let (host, port) = if let Some((h, p)) = host_port.split_once(':') {
            (h, p.parse::<u16>().unwrap_or(default_port))
        } else {
            (host_port, default_port)
        };
        log::info!("connecting to {}:{}", host, port);

        log::info!("establish tcp connection");
        let tcp_stream = TcpStream::connect(format!("{host}:{port}")).await?;
        let stream = match scheme {
            "ws" => Connector::Plain.wrap(host, tcp_stream).await?,
            _ => {
                log::info!("init tls provider");
                let provider = Arc::new(rustls_rustcrypto::provider());
                log::info!("init tls connector");
                let connector = Connector::new_rustls_with_crypto_provider(provider)?;
                log::info!("warp tls connection");
                connector.wrap(host, tcp_stream).await?
            }
        };
        let (ws, resp) = tokio_websockets::ClientBuilder::new()
            .uri(&uri)?
            .connect_on(stream)
            .await?;

        log::info!(
            "ws resp status: {:?}, headers: {:?} ",
            resp.status(),
            resp.headers()
        );

        let timeout = std::time::Duration::from_secs(30);

        Ok(Self { timeout, ws })
    }

    pub fn set_timeout(&mut self, timeout: std::time::Duration) {
        self.timeout = timeout;
    }

    pub async fn reconnect(&mut self) -> anyhow::Result<()> {
        let uri = self.uri.clone();
        let (ws, _resp) = tokio_websockets::ClientBuilder::new()
            .uri(&format!("{uri}?reconnect=true"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reconnect: {}", e))?;

        self.ws = ws;
        Ok(())
    }

    pub async fn reconnect_with_retry(&mut self, retries: usize) -> anyhow::Result<()> {
        for attempt in 0..retries {
            match self.reconnect().await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    log::warn!(
                        "Reconnect attempt {}/{} failed: {}",
                        attempt + 1,
                        retries,
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
            }
        }
        Err(anyhow::anyhow!("All reconnect attempts failed"))
    }

    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.ws.close().await?;
        Ok(())
    }

    pub async fn send(&mut self, msg: Message) -> anyhow::Result<()> {
        tokio::time::timeout(self.timeout, self.ws.send(msg))
            .map_err(|_| anyhow::anyhow!("Timeout sending message"))
            .await??;
        Ok(())
    }

    pub async fn send_client_command(
        &mut self,
        cmd: crate::protocol::ClientCommand,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_string(&cmd)
            .map_err(|e| anyhow::anyhow!("Failed to serialize command: {}", e))?;
        let msg = Message::text(payload);
        self.send(msg).await
    }

    pub async fn send_client_audio_chunk(&mut self, chunk: bytes::Bytes) -> anyhow::Result<()> {
        let msg = Message::binary(chunk);
        self.send(msg).await
    }

    pub async fn send_client_audio_chunk_i16(&mut self, chunk: Vec<i16>) -> anyhow::Result<()> {
        let audio_buffer_u8 =
            unsafe { std::slice::from_raw_parts(chunk.as_ptr() as *const u8, chunk.len() * 2) };

        self.send_client_audio_chunk(bytes::Bytes::from(audio_buffer_u8))
            .await
    }

    pub async fn recv(&mut self) -> anyhow::Result<Event> {
        let msg = self
            .ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("WS channel closed"))??;

        if msg.is_binary() {
            let payload = msg.into_payload();
            let evt = rmp_serde::from_slice::<ServerEvent>(&payload)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize binary data: {}", e))?;
            Ok(Event::ServerEvent(evt))
        } else {
            Err(anyhow::anyhow!("Invalid message type"))
        }
    }
}
