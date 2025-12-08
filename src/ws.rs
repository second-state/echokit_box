#[allow(unused)]
fn print_stack_high() {
    let stack_high =
        unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark2(std::ptr::null_mut()) };
    log::info!("Stack high: {}", stack_high);
}

use crate::{app::Event, protocol::ServerEvent};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio_websockets::Message;

enum SubmitItem {
    JSON(crate::protocol::ClientCommand),
    AudioChunk(Vec<u8>),
    Close,
}

async fn ws_manager(
    mut ws: tokio_websockets::WebSocketStream<
        tokio_websockets::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut rx: tokio::sync::mpsc::Receiver<SubmitItem>,
    tx: tokio::sync::mpsc::Sender<ServerEvent>,
) -> anyhow::Result<()> {
    enum SelectItem {
        Recv(Option<Result<Message, tokio_websockets::error::Error>>),
        Send(Option<SubmitItem>),
    }

    loop {
        let recv_fut = ws.next();
        let send_fut = rx.recv();
        let item = tokio::select! {
            recv = recv_fut => {
                SelectItem::Recv(recv)
            },
            send = send_fut => {
                SelectItem::Send(send)
            },
        };

        match item {
            SelectItem::Recv(Some(Ok(msg))) => {
                if msg.is_binary() {
                    let payload = msg.into_payload();
                    let evt = rmp_serde::from_slice::<ServerEvent>(&payload)
                        .map_err(|e| anyhow::anyhow!("Failed to deserialize binary data: {}", e))?;
                    tx.send(evt)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to send event to channel: {}", e))?;
                } else {
                    log::error!("Unexpected non-binary WebSocket message received");
                    continue;
                }
            }
            SelectItem::Recv(None) => {
                log::info!("WebSocket stream ended");
                return Ok(());
            }
            SelectItem::Recv(Some(Err(e))) => {
                log::error!("WebSocket receive error: {}", e);
                return Err(anyhow::anyhow!("WebSocket receive error: {}", e));
            }
            SelectItem::Send(Some(msg)) => {
                log::debug!("WebSocket message sent");
                match msg {
                    SubmitItem::JSON(cmd) => {
                        let payload = serde_json::to_string(&cmd).map_err(|e| {
                            anyhow::anyhow!("Failed to serialize command to JSON: {}", e)
                        })?;
                        let msg = Message::text(payload);
                        ws.send(msg)
                            .await
                            .map_err(|e| anyhow::anyhow!("WebSocket send error: {}", e))?;
                    }
                    SubmitItem::AudioChunk(chunk) => {
                        let msg = Message::binary(bytes::Bytes::from(chunk));
                        ws.send(msg)
                            .await
                            .map_err(|e| anyhow::anyhow!("WebSocket send error: {}", e))?;
                    }
                    SubmitItem::Close => {
                        ws.close()
                            .await
                            .map_err(|e| anyhow::anyhow!("WebSocket close error: {}", e))?;
                        log::info!("WebSocket closed by client request");
                        return Ok(());
                    }
                }
            }
            SelectItem::Send(None) => {
                log::info!("WebSocket send channel closed");
                return Ok(());
            }
        }
    }
}

async fn connect_handler(
    ws: tokio_websockets::WebSocketStream<tokio_websockets::MaybeTlsStream<tokio::net::TcpStream>>,
) -> (
    tokio::sync::mpsc::Sender<SubmitItem>,
    tokio::sync::mpsc::Receiver<ServerEvent>,
) {
    let (tx_ws, rx) = tokio::sync::mpsc::channel::<SubmitItem>(32);
    let (tx, rx_ws) = tokio::sync::mpsc::channel::<ServerEvent>(32);

    tokio::spawn(async move {
        if let Err(e) = ws_manager(ws, rx, tx).await {
            log::error!("WebSocket manager error: {}", e);
        }
    });

    (tx_ws, rx_ws)
}

pub struct Server {
    pub url: String,
    pub id: String,
    timeout: std::time::Duration,
    tx: tokio::sync::mpsc::Sender<SubmitItem>,
    rx: tokio::sync::mpsc::Receiver<ServerEvent>,
}

impl Server {
    pub async fn new(id: String, url: String) -> anyhow::Result<Self> {
        let u = if url.ends_with("/") {
            format!("{}{}", url, id)
        } else {
            format!("{}/{}", url, id)
        };

        let (ws, _resp) = tokio_websockets::ClientBuilder::new()
            .uri(&u)?
            .add_header(
                http::HeaderName::from_static("sec-websocket-extensions"),
                http::HeaderValue::from_static("permessage-deflate; client_max_window_bits"),
            )?
            .connect()
            .await?;

        let timeout = std::time::Duration::from_secs(30);

        let (tx, rx) = connect_handler(ws).await;

        Ok(Self {
            id,
            url,
            timeout,
            tx,
            rx,
        })
    }

    #[allow(unused)]
    pub fn set_timeout(&mut self, timeout: std::time::Duration) {
        self.timeout = timeout;
    }

    pub async fn reconnect(&mut self) -> anyhow::Result<()> {
        let u = if self.url.ends_with("/") {
            format!("{}{}?reconnect=true", self.url, self.id)
        } else {
            format!("{}/{}?reconnect=true", self.url, self.id)
        };

        let (ws, _resp) = tokio_websockets::ClientBuilder::new()
            .uri(&u)?
            .add_header(
                http::HeaderName::from_static("sec-websocket-extensions"),
                http::HeaderValue::from_static("permessage-deflate; client_max_window_bits"),
            )?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reconnect: {}", e))?;

        let (tx, rx) = connect_handler(ws).await;
        self.tx = tx;
        self.rx = rx;
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
        let _ = self.send(SubmitItem::Close).await;
        Ok(())
    }

    async fn send(&mut self, msg: SubmitItem) -> anyhow::Result<()> {
        tokio::time::timeout(self.timeout, self.tx.send(msg))
            .map_err(|_| anyhow::anyhow!("Timeout sending message"))
            .await?
            .map_err(|_| anyhow::anyhow!("Failed to send message"))?;
        Ok(())
    }

    pub async fn send_client_command(
        &mut self,
        cmd: crate::protocol::ClientCommand,
    ) -> anyhow::Result<()> {
        // let payload = serde_json::to_string(&cmd)
        //     .map_err(|e| anyhow::anyhow!("Failed to serialize command: {}", e))?;
        // let msg = Message::text(payload);
        self.send(SubmitItem::JSON(cmd)).await
    }

    pub async fn send_client_audio_chunk(&mut self, chunk: Vec<u8>) -> anyhow::Result<()> {
        self.send(SubmitItem::AudioChunk(chunk)).await
    }

    pub async fn send_client_audio_chunk_i16(&mut self, chunk: Vec<i16>) -> anyhow::Result<()> {
        let audio_buffer_u8 =
            unsafe { std::slice::from_raw_parts(chunk.as_ptr() as *const u8, chunk.len() * 2) };

        self.send_client_audio_chunk(audio_buffer_u8.to_vec()).await
    }

    pub async fn recv(&mut self) -> anyhow::Result<Event> {
        let msg = self
            .rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("WS channel closed"))?;
        Ok(Event::ServerEvent(msg))
    }
}
