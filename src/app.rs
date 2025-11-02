use std::sync::Arc;

use tokio::sync::mpsc;

use crate::{
    audio::{self, AudioEvent, MicRx},
    protocol::{self, ServerEvent},
    ws::Server,
};

#[derive(Debug)]
pub enum Event {
    Event(&'static str),
    ServerEvent(ServerEvent),
    MicAudioChunk(Vec<i16>),
    MicAudioEnd,
    MicInterrupt(Vec<i16>),
    MicInterruptWaitTimeout,
}

#[allow(dead_code)]
impl Event {
    pub const IDLE: &'static str = "idle";
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

    pub const NOTIFY: &'static str = "notify";
}

async fn select_evt(
    evt_rx: &mut mpsc::Receiver<Event>,
    server: &mut Server,
    notify: &tokio::sync::Notify,
    wait_notify: bool,
    timeout: std::time::Duration,
) -> Option<Event> {
    let s_fut = async {
        if wait_notify {
            notify.notified().await;
            Ok(Event::Event(Event::NOTIFY))
        } else {
            server.recv().await
        }
    };
    let timeout_event = if timeout == INTERNAL_TIMEOUT {
        Some(Event::MicInterruptWaitTimeout)
    } else {
        Some(Event::Event(Event::IDLE))
    };

    let timeout_f = tokio::time::sleep(timeout);

    tokio::select! {
        _ = timeout_f => {
            log::info!("Event select timeout");
            timeout_event
        }
        Some(evt) = evt_rx.recv() => {
            match &evt {
                Event::Event(_) => {
                    log::info!("[Select] Received event: {:?}", evt);
                },
                Event::MicAudioEnd => {
                    log::info!("[Select] Received MicAudioEnd");
                },
                Event::MicAudioChunk(data) => {
                    log::debug!("[Select] Received MicAudioChunk with {} bytes", data.len());
                },
                Event::ServerEvent(_) => {
                    log::info!("[Select] Received ServerEvent: {:?}", evt);
                },
                Event::MicInterrupt(data) => {
                    log::info!("[Select] Received MicInterrupt with {} samples", data.len());
                },
                Event::MicInterruptWaitTimeout => {
                    log::info!("[Select] Received MicInterruptWaitTimeout");
                }
            }
            Some(evt)
        }
        Ok(msg) = s_fut => {
            match msg {
                Event::ServerEvent(ServerEvent::AudioChunk { .. })=>{
                    log::debug!("[Select] Received AudioChunk");
                }
                Event::ServerEvent(ServerEvent::HelloChunk { .. })=>{
                    log::debug!("[Select] Received HelloChunk");
                }
                _=> {
                    log::debug!("[Select] Received message: {:?}", msg);
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

const SPEED_LIMIT: f64 = 1.5;
const INTERNAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const NORMAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn main_work<'d>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mut evt_rx: MicRx,
    backgroud_buffer: Option<&'d [u8]>,
) -> anyhow::Result<()> {
    #[derive(PartialEq, Eq)]
    enum State {
        Listening,
        Waiting,
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
    let mut speed = 1.5;
    let mut vol = 3u8;

    let mut hello_wav = Vec::with_capacity(1024 * 30);

    let notify: Arc<tokio::sync::Notify> = Arc::new(tokio::sync::Notify::new());
    let mut wait_notify = false;
    let mut init_hello = false;
    let mut allow_interrupt = false;
    let mut timeout = NORMAL_TIMEOUT;

    while let Some(evt) = select_evt(&mut evt_rx, &mut server, &notify, wait_notify, timeout).await
    {
        match evt {
            Event::Event(Event::GAIA | Event::K0) => {
                log::info!("Received event: gaia");
                // gui.state = "gaia".to_string();
                // gui.display_flush().unwrap();

                if state == State::Listening {
                    state = State::Idle;
                    gui.state = "Idle".to_string();
                    gui.display_flush().unwrap();
                    server.close().await?;
                } else {
                    let hello_notify = Arc::new(tokio::sync::Notify::new());
                    player_tx
                        .send(AudioEvent::Hello(hello_notify.clone()))
                        .map_err(|e| anyhow::anyhow!("Error sending hello: {e:?}"))?;
                    log::info!("Waiting for hello response");
                    let _ = hello_notify.notified().await;

                    server.reconnect_with_retry(3).await?;

                    start_submit = false;
                    submit_audio = 0.0;
                    audio_buffer = Vec::with_capacity(8192);

                    log::info!("Hello response received");

                    state = State::Listening;
                    gui.state = "Listening...".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::Event(Event::K0_) => {
                allow_interrupt = !allow_interrupt;
                log::info!("Set allow_interrupt to {}", allow_interrupt);
                gui.state = format!("Interrupt: {}", allow_interrupt);
                gui.display_flush().unwrap();
            }
            Event::Event(Event::VOL_UP) => {
                vol += 1;
                if vol > 5 {
                    vol = 5;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", vol);
                gui.state = format!("Volume: {}", vol);
                gui.display_flush().unwrap();
            }
            Event::Event(Event::VOL_DOWN) => {
                vol -= 1;
                if vol < 1 {
                    vol = 1;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", vol);
                gui.state = format!("Volume: {}", vol);
                gui.display_flush().unwrap();
            }
            Event::Event(Event::YES | Event::K1) => {}
            Event::Event(Event::IDLE) => {
                if state == State::Listening {
                    state = State::Idle;
                    gui.state = "Idle".to_string();
                    gui.display_flush().unwrap();
                    server.close().await?;
                }
            }
            Event::Event(Event::NOTIFY) => {
                log::info!("Received notify event");
                wait_notify = false;
            }
            Event::Event(evt) => {
                log::info!("Received event: {:?}", evt);
            }
            Event::MicAudioChunk(data) => {
                if state != State::Listening {
                    log::debug!("Received MicAudioChunk while no Listening state, ignoring");
                    continue;
                }
                submit_audio += data.len() as f32 / 16000.0;
                audio_buffer.extend_from_slice(&data);
                // 0.25秒提交一次
                if audio_buffer.len() >= 8192 && submit_audio > 0.5 {
                    if !start_submit {
                        log::info!("Start submitting audio");
                        server
                            .send_client_command(protocol::ClientCommand::StartChat)
                            .await?;
                        log::info!("Submitted StartChat command");
                    }
                    start_submit = true;
                    server.send_client_audio_chunk_i16(audio_buffer).await?;
                    audio_buffer = Vec::with_capacity(8192);
                }
            }
            Event::MicAudioEnd => {
                log::info!("Received MicAudioEnd");
                if state != State::Listening {
                    log::debug!("Received MicAudioEnd while no Listening state, ignoring");
                    continue;
                }
                if submit_audio > 0.5 {
                    if !audio_buffer.is_empty() {
                        server.send_client_audio_chunk_i16(audio_buffer).await?;
                        audio_buffer = Vec::with_capacity(8192);
                    }
                    server
                        .send_client_command(protocol::ClientCommand::Submit)
                        .await?;
                    log::info!("Submitted audio");
                    need_compute = metrics.is_timeout();

                    submit_audio = 0.0;
                    start_submit = false;
                    wait_notify = false;
                    state = State::Waiting;
                    gui.state = "Waiting...".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::MicInterrupt(interrupt_data) => {
                log::info!(
                    "Received MicInterrupt with {} samples",
                    interrupt_data.len()
                );
                if !(state == State::Listening || state == State::Speaking) {
                    log::debug!(
                        "Received MicInterrupt while no Listening or Speaking state, ignoring"
                    );
                    continue;
                }

                if !allow_interrupt {
                    log::info!("Interrupts are disabled, ignoring MicInterrupt");
                    continue;
                }

                let interrupt_audio_sec = interrupt_data.len() as f32 / 16000.0;
                if interrupt_audio_sec < 1.2 {
                    log::info!(
                        "Interrupt audio too short ({} s), ignoring",
                        interrupt_audio_sec
                    );
                    continue;
                }

                let int_notify = Arc::new(tokio::sync::Notify::new());
                player_tx
                    .send(AudioEvent::Interrupt(int_notify.clone()))
                    .map_err(|e| anyhow::anyhow!("Error sending interrupt: {e:?}"))?;
                log::info!("Waiting for interrupt response");
                let _ = int_notify.notified().await;
                log::info!("Interrupt response received");

                server.reconnect_with_retry(3).await?;

                start_submit = false;
                submit_audio = interrupt_audio_sec;
                audio_buffer = interrupt_data;

                state = State::Listening;
                gui.state = "Listening...".to_string();
                gui.display_flush().unwrap();
                timeout = INTERNAL_TIMEOUT;
            }
            Event::MicInterruptWaitTimeout => {
                log::info!("Received MicInterruptWaitTimeout");
                timeout = NORMAL_TIMEOUT;
                if start_submit {
                    log::info!("Already started submit, ignoring timeout");
                    continue;
                }
                server
                    .send_client_command(protocol::ClientCommand::StartChat)
                    .await?;
                log::info!("Submitted StartChat command due to interrupt timeout");

                server.send_client_audio_chunk_i16(audio_buffer).await?;
                server
                    .send_client_command(protocol::ClientCommand::Submit)
                    .await?;
                log::info!("Submitted audio");
                need_compute = metrics.is_timeout();

                audio_buffer = Vec::with_capacity(8192);
                submit_audio = 0.0;
                start_submit = false;
                wait_notify = false;
                state = State::Waiting;
                gui.state = "Waiting...".to_string();
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::ASR { text }) => {
                log::info!("Received ASR: {:?}", text);
                state = State::Speaking;
                gui.state = "ASR".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
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
                if state != State::Speaking {
                    log::debug!("Received StartAudio while not in speaking state");
                    continue;
                }
                log::info!("Received audio start: {:?}", text);
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
                    log::debug!("Received audio chunk while not speaking");
                    continue;
                }

                if need_compute {
                    metrics.add_data(data.len());
                }

                if speed < SPEED_LIMIT {
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

                if state != State::Speaking {
                    log::debug!("Received EndAudio while not in speaking state");
                    continue;
                }

                if recv_audio_buffer.len() > 0 {
                    if let Err(e) = player_tx.send(AudioEvent::SpeechChunki16(recv_audio_buffer)) {
                        log::error!("Error sending audio chunk: {:?}", e);
                        gui.state = "Error on audio chunk".to_string();
                        gui.display_flush().unwrap();
                    }
                    recv_audio_buffer = Vec::with_capacity(8192);
                }

                if let Err(e) = player_tx.send(AudioEvent::EndSpeech(notify.clone())) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }

                if need_compute {
                    speed = metrics.speed();
                    need_compute = false;
                }

                log::info!("Audio speed: {:.2}x", speed);

                wait_notify = true;

                crate::log_heap();
            }

            Event::ServerEvent(ServerEvent::EndResponse) => {
                log::info!("Received request end");
                state = State::Listening;
                gui.state = "Listening...".to_string();
                gui.display_flush().unwrap();
                recv_audio_buffer.clear();
            }
            Event::ServerEvent(ServerEvent::HelloStart) => {
                log::info!("Received hello start");
                hello_wav.clear();
            }
            Event::ServerEvent(ServerEvent::HelloChunk { data }) => {
                log::debug!("Received hello chunk");
                if !init_hello {
                    hello_wav.extend_from_slice(&data);
                }
            }
            Event::ServerEvent(ServerEvent::HelloEnd) => {
                log::info!("Received hello end");
                if !init_hello {
                    if let Err(_) = player_tx.send(AudioEvent::SetHello(hello_wav)) {
                        log::error!("Error sending hello end");
                        gui.state = "Error on hello end".to_string();
                        gui.display_flush().unwrap();
                    }
                    hello_wav = Vec::with_capacity(1024 * 30);
                    init_hello = true;
                }
            }

            Event::ServerEvent(ServerEvent::StartVideo | ServerEvent::EndVideo) => {}
        }
    }

    log::info!("Main work done");

    Ok(())
}
