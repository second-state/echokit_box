use tokio::sync::mpsc;
use tokio_websockets::Message;

use crate::{
    audio::{self, AudioEvent},
    protocol::{self, ServerEvent},
    ws::Server,
};

#[derive(Debug)]
pub enum Event {
    Event(&'static str),
    ServerEvent(ServerEvent),
    MicAudioChunk(Vec<i16>),
    MicAudioEnd,
}

#[allow(dead_code)]
impl Event {
    pub const GAIA: &'static str = "gaia";
    pub const NO: &'static str = "no";
    pub const YES: &'static str = "yes";
    pub const NOISE: &'static str = "noise";
    pub const RESET: &'static str = "reset";
    pub const UNKNOWN: &'static str = "unknown";
    pub const K0: &'static str = "k0";
    pub const K0_: &'static str = "k0_";

    pub const K1: &'static str = "k1";
    pub const K2: &'static str = "k2";
    pub const VOL_UP: &'static str = "vol_up";
    pub const VOL_DOWN: &'static str = "vol_down";
}

async fn select_evt(evt_rx: &mut mpsc::Receiver<Event>, server: &mut Server) -> Option<Event> {
    tokio::select! {
        Some(evt) = evt_rx.recv() => {
            match &evt {
                Event::Event(_)=>{
                    log::info!("Received event: {:?}", evt);
                },
                Event::MicAudioEnd=>{
                    log::info!("Received MicAudioEnd");
                },
                Event::MicAudioChunk(data)=>{
                    log::debug!("Received MicAudioChunk with {} bytes", data.len());
                },
                Event::ServerEvent(_)=>{
                    log::info!("Received ServerEvent: {:?}", evt);
                },
            }
            Some(evt)
        }
        Ok(msg) = server.recv() => {
            match msg {
                Event::ServerEvent(ServerEvent::AudioChunk { .. })=>{
                    log::debug!("Received AudioChunk");
                }
                Event::ServerEvent(ServerEvent::HelloChunk { .. })=>{
                    log::debug!("Received HelloChunk");
                }
                _=> {
                    log::debug!("Received message: {:?}", msg);
                }
            }
            Some(msg)
        }
        else => {
            log::info!("No events");
            None
        }
    }
}

struct DownloadMetrics {
    start_time: std::time::Instant,
    data_size: usize,
    timeout_sec: u64,
}

impl DownloadMetrics {
    fn new() -> Self {
        Self {
            start_time: std::time::Instant::now() - std::time::Duration::from_secs(300),
            data_size: 0,
            timeout_sec: 30,
        }
    }

    fn is_timeout(&self) -> bool {
        self.start_time.elapsed().as_secs() > self.timeout_sec
    }

    fn reset(&mut self) {
        self.start_time = std::time::Instant::now();
        self.data_size = 0;
    }

    fn add_data(&mut self, size: usize) {
        self.data_size += size;
    }

    fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    fn speed(&self) -> f64 {
        self.elapsed().as_secs_f64() / ((self.data_size as f64) / 32000.0)
    }
}

// TODO: 按键打断
// TODO: 超时不监听
pub async fn main_work<'d>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mut evt_rx: mpsc::Receiver<Event>,
    backgroud_buffer: Option<&'d [u8]>,
) -> anyhow::Result<()> {
    #[derive(PartialEq, Eq)]
    enum State {
        Listening,
        Speaking,
        Idle,
    }

    let mut gui = crate::ui::UI::new(backgroud_buffer)?;

    gui.state = "Idle".to_string();
    gui.display_flush().unwrap();

    let mut state = State::Idle;

    let mut submit_audio = 0.0;
    let mut start_submit = false;

    let mut audio_buffer = Vec::with_capacity(8192);
    let mut recv_audio_buffer = Vec::with_capacity(8192);

    let mut metrics = DownloadMetrics::new();
    let mut need_compute = true;
    let mut speed = 0.8;
    let mut vol = 0.5;

    let mut hello_wav = Vec::with_capacity(1024 * 30);

    while let Some(evt) = select_evt(&mut evt_rx, &mut server).await {
        match evt {
            Event::Event(Event::GAIA | Event::K0) => {
                log::info!("Received event: gaia");
                // gui.state = "gaia".to_string();
                // gui.display_flush().unwrap();

                if state == State::Listening {
                    state = State::Idle;
                    gui.state = "Idle".to_string();
                    gui.display_flush().unwrap();
                } else {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    player_tx
                        .send(AudioEvent::Hello(tx))
                        .map_err(|e| anyhow::anyhow!("Error sending hello: {e:?}"))?;
                    log::info!("Waiting for hello response");
                    let _ = rx.await;
                    log::info!("Hello response received");

                    state = State::Listening;
                    gui.state = "Listening...".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::Event(Event::K0_) => {}
            Event::Event(Event::VOL_UP) => {
                vol += 0.1;
                if vol > 1.0 {
                    vol = 1.0;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {:.1}", vol);
                gui.state = format!("Volume: {:.1}", vol);
                gui.display_flush().unwrap();
            }
            Event::Event(Event::VOL_DOWN) => {
                vol -= 0.1;
                if vol < 0.1 {
                    vol = 0.1;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {:.1}", vol);
                gui.state = format!("Volume: {:.1}", vol);
                gui.display_flush().unwrap();
            }
            Event::Event(Event::YES | Event::K1) => {}
            Event::Event(Event::NO) => {}
            Event::Event(evt) => {
                log::info!("Received event: {:?}", evt);
            }
            Event::MicAudioChunk(data) => {
                submit_audio += data.len() as f32 / 16000.0;
                audio_buffer.extend_from_slice(&data);
                // 0.25秒提交一次
                if audio_buffer.len() >= 8192 && submit_audio > 0.5 {
                    if !start_submit {
                        log::info!("Start submitting audio");
                        let msg = protocol::ClientCommand::StartChat;
                        server
                            .send(Message::text(serde_json::to_string(&msg).unwrap()))
                            .await?;
                    }
                    start_submit = true;
                    let audio_buffer_u8 = unsafe {
                        std::slice::from_raw_parts(
                            audio_buffer.as_ptr() as *const u8,
                            audio_buffer.len() * 2,
                        )
                    };
                    server
                        .send(Message::binary(bytes::Bytes::from(audio_buffer_u8)))
                        .await?;
                    audio_buffer = Vec::with_capacity(8192);
                }
            }
            Event::MicAudioEnd => {
                if submit_audio > 0.5 {
                    if !audio_buffer.is_empty() {
                        let audio_buffer_u8 = unsafe {
                            std::slice::from_raw_parts(
                                audio_buffer.as_ptr() as *const u8,
                                audio_buffer.len() * 2,
                            )
                        };
                        server
                            .send(Message::binary(bytes::Bytes::from(audio_buffer_u8)))
                            .await?;
                        audio_buffer = Vec::with_capacity(8192);
                    }
                    server
                        .send(Message::text(
                            serde_json::to_string(&protocol::ClientCommand::Submit).unwrap(),
                        ))
                        .await?;
                    need_compute = metrics.is_timeout();
                }
                submit_audio = 0.0;
                start_submit = false;
            }
            Event::ServerEvent(ServerEvent::ASR { text }) => {
                log::info!("Received ASR: {:?}", text);
                gui.state = "ASR".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
                if !text.trim().is_empty() {
                    player_tx
                        .send(AudioEvent::StopSpeech)
                        .map_err(|_| anyhow::anyhow!("Error sending stop speech"))?;
                }
            }
            Event::ServerEvent(ServerEvent::Action { action }) => {
                log::info!("Received action");
                gui.state = format!("Action: {}", action);
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::StartAudio { text }) => {
                if need_compute {
                    metrics.reset();
                }
                log::info!("Received audio start: {:?}", text);
                state = State::Speaking;
                gui.state = format!("[{:.2}x]|Speaking...", speed);
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
                player_tx
                    .send(AudioEvent::StartSpeech)
                    .map_err(|e| anyhow::anyhow!("Error sending start: {e:?}"))?;
            }
            Event::ServerEvent(ServerEvent::AudioChunk { data }) => {
                log::debug!("Received audio chunk");
                if state != State::Speaking {
                    log::warn!("Received audio chunk while not speaking");
                    continue;
                }

                if need_compute {
                    metrics.add_data(data.len());
                }

                if speed < 1.0 {
                    if let Err(e) = player_tx.send(AudioEvent::SpeechChunk(data)) {
                        log::error!("Error sending audio chunk: {:?}", e);
                        gui.state = "Error on audio chunk".to_string();
                        gui.display_flush().unwrap();
                    }
                } else {
                    let data_ = unsafe {
                        std::slice::from_raw_parts(data.as_ptr() as *const i16, data.len() / 2)
                    };
                    recv_audio_buffer.extend_from_slice(data_);
                }
            }
            Event::ServerEvent(ServerEvent::EndAudio) => {
                log::info!("Received audio end");

                if need_compute {
                    speed = metrics.speed();
                    need_compute = false;
                }

                log::info!("Audio speed: {:.2}x", speed);

                if speed > 1.0 && recv_audio_buffer.len() > 0 {
                    if let Err(e) = player_tx.send(AudioEvent::SpeechChunki16(recv_audio_buffer)) {
                        log::error!("Error sending audio chunk: {:?}", e);
                        gui.state = "Error on audio chunk".to_string();
                        gui.display_flush().unwrap();
                    }
                    recv_audio_buffer = Vec::with_capacity(8192);
                }

                let (tx, rx) = tokio::sync::oneshot::channel();
                if let Err(e) = player_tx.send(AudioEvent::EndSpeech(tx)) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }
                let _ = rx.await;
                gui.display_flush().unwrap();
            }

            Event::ServerEvent(ServerEvent::EndResponse) => {
                log::info!("Received request end");
                state = State::Listening;
                gui.state = "Listening...".to_string();
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::HelloStart) => {
                hello_wav.clear();
            }
            Event::ServerEvent(ServerEvent::HelloChunk { data }) => {
                log::info!("Received hello chunk");
                hello_wav.extend_from_slice(&data);
            }
            Event::ServerEvent(ServerEvent::HelloEnd) => {
                log::info!("Received hello end");
                if let Err(_) = player_tx.send(AudioEvent::SetHello(hello_wav)) {
                    log::error!("Error sending hello end");
                    gui.state = "Error on hello end".to_string();
                    gui.display_flush().unwrap();
                } else {
                    gui.state = "Hello set".to_string();
                    gui.display_flush().unwrap();
                }
                hello_wav = Vec::with_capacity(1024 * 30);
            }

            Event::ServerEvent(ServerEvent::StartVideo | ServerEvent::EndVideo) => {}
        }
    }

    log::info!("Main work done");

    Ok(())
}
