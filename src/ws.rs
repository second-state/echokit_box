#[allow(unused)]
fn print_stack_high() {
    let stack_high =
        unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark2(std::ptr::null_mut()) };
    log::info!("Stack high: {}", stack_high);
}

use crate::{app::Event, protocol::ServerEvent};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio_websockets::Message;

pub struct Server {
    pub uri: String,
    timeout: std::time::Duration,
    ws: tokio_websockets::WebSocketStream<tokio_websockets::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Server {
    pub async fn new(uri: String) -> anyhow::Result<Self> {
        let (ws, _resp) = tokio_websockets::ClientBuilder::new()
            .uri(&uri)?
            .connect()
            .await?;

        let timeout = std::time::Duration::from_secs(30);

        Ok(Self { uri, timeout, ws })
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
