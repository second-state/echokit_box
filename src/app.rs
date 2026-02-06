use std::sync::Arc;

use tokio::sync::mpsc;

use crate::{
    audio::{self, AudioEvent, EventRx},
    protocol::{self, ServerEvent},
    ui::DisplayTargetDrive,
    ws::Server,
};

#[derive(Debug)]
pub enum Event {
    Event(&'static str),
    ServerEvent(ServerEvent),
    MicAudioChunk(Vec<i16>),
    MicAudioEnd,
    Vowel(u8),
    #[cfg_attr(not(feature = "extra_server"), allow(unused))]
    ServerUrl(String),
}

#[allow(unused)]
impl Event {
    pub const IDLE: &'static str = "idle";
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
    pub const VOL_SWITCH: &'static str = "vol_switch";

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

    let timeout_f = tokio::time::sleep(timeout);

    tokio::select! {
        _ = timeout_f => {
            // log::info!("Event select timeout");
             Some(Event::Event(Event::IDLE))
        }
        Ok(msg) = s_fut => {
            match msg {
                Event::ServerEvent(ServerEvent::AudioChunk { .. })=>{
                    log::debug!("[Select] Received AudioChunk");
                }
                Event::ServerEvent(ServerEvent::AudioChunki16 { .. })=>{
                    log::debug!("[Select] Received AudioChunki16");
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
                Event::Vowel(v) => {
                    log::debug!("[Select] Received Vowel: {}", v);
                }
                Event::ServerUrl(url) => {
                    log::info!("[Select] Received ServerUrl: {}", url);
                }
            }
            Some(evt)
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
            timeout_sec: 15,
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

const SPEED_LIMIT: f64 = 1.0;
const NORMAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 5);

struct SubmitState {
    submit_audio: f32,
    start_submit: bool,
    audio_buffer: Vec<i16>,
    got_asr_result: bool,
}

impl SubmitState {
    fn clear(&mut self) {
        self.submit_audio = 0.0;
        self.start_submit = false;
        self.audio_buffer.clear();
        self.got_asr_result = false;
    }
}

pub async fn main_work<'d, const N: usize>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mut evt_rx: EventRx,
    framebuffer: &mut crate::boards::ui::DisplayBuffer,
    gui: &mut crate::boards::ui::ChatUI<N>,
) -> anyhow::Result<()> {
    #[derive(PartialEq, Eq)]
    enum State {
        Listening,
        Waiting,
        Choices,
        Speaking,
        Idle,
    }

    gui.set_state("Idle".to_string());
    gui.set_text("".to_string());
    gui.render_to_target(framebuffer)?;
    framebuffer.flush()?;

    let mut state = State::Idle;
    let mut choices_index = 0;
    let mut choices_items: Vec<String> = Vec::new();

    let mut submit_state = SubmitState {
        submit_audio: 0.0,
        start_submit: false,
        audio_buffer: Vec::with_capacity(8192),
        got_asr_result: false,
    };

    let mut recv_audio_buffer = Vec::with_capacity(8192);

    let mut metrics = DownloadMetrics::new();
    let mut need_compute = true;
    let mut start_audio = false;
    let mut speed = 0.5;
    let mut vol = 3u8;

    let mut hello_wav = Vec::with_capacity(1024 * 30);

    let notify: Arc<tokio::sync::Notify> = Arc::new(tokio::sync::Notify::new());
    let mut wait_notify = false;
    let mut init_hello = false;
    let mut allow_interrupt = false;
    let timeout = NORMAL_TIMEOUT;

    while let Some(evt) = select_evt(&mut evt_rx, &mut server, &notify, wait_notify, timeout).await
    {
        match evt {
            Event::Event(Event::K0) => {
                log::info!("Received event: k0");

                if state == State::Listening {
                    state = State::Idle;
                    gui.set_state("Idle".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                    server.close().await?;
                } else if state == State::Choices {
                    server.send_client_select(choices_index).await?;
                    state = State::Waiting;
                    gui.set_state("Waiting...".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                } else {
                    gui.set_state("Connecting...".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;

                    crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                    server.reconnect_with_retry(3).await?;

                    let hello_notify = Arc::new(tokio::sync::Notify::new());
                    player_tx
                        .send(AudioEvent::Hello(hello_notify.clone()))
                        .map_err(|e| anyhow::anyhow!("Error sending hello: {e:?}"))?;
                    log::info!("Waiting for hello response");
                    let _ = hello_notify.notified().await;

                    submit_state.clear();

                    log::info!("Hello response received");

                    state = State::Listening;
                    gui.set_state("Ready".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                }
            }
            Event::Event(Event::K0_) => {
                #[cfg(feature = "voice_interrupt")]
                {
                    allow_interrupt = !allow_interrupt;
                    log::info!("Set allow_interrupt to {}", allow_interrupt);
                    gui.set_state(format!("Interrupt: {}", allow_interrupt));
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                }
            }
            Event::Event(Event::VOL_UP) if state != State::Choices => {
                vol += 1;
                if vol > 5 {
                    vol = 5;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", vol);
                gui.set_state(format!("Volume: {}", vol));
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::Event(Event::VOL_UP) => {
                choices_index += 1;
                choices_index %= choices_items.len();

                let choices_string = choices_items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if i == choices_index {
                            format!("> {}", item)
                        } else {
                            format!("  {}", item)
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                gui.set_text(choices_string);
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::Event(Event::VOL_DOWN) if state != State::Choices => {
                vol -= 1;
                if vol < 1 {
                    vol = 1;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", vol);
                gui.set_state(format!("Volume: {}", vol));
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::Event(Event::VOL_DOWN) => {
                if choices_index != 0 {
                    choices_index -= 1;
                }

                let choices_string = choices_items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if i == choices_index {
                            format!("> {}", item)
                        } else {
                            format!("  {}", item)
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                gui.set_text(choices_string);
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::Event(Event::VOL_SWITCH) if state != State::Choices => {
                vol -= 1;
                if vol < 1 {
                    vol = 5;
                }
                player_tx
                    .send(AudioEvent::VolSet(vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", vol);
                gui.set_state(format!("Volume: {}", vol));
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::Event(Event::YES | Event::K1) => {}
            Event::Event(Event::IDLE) => {
                log::info!("Received idle event");
                if state == State::Listening {
                    state = State::Idle;
                    gui.set_state("Idle".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
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
            Event::Vowel(v) => {
                if gui.set_avatar_index(v as usize) {
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                }
            }
            Event::MicAudioChunk(data) if state == State::Listening => {
                submit_state.submit_audio += data.len() as f32 / 16000.0;
                submit_state.audio_buffer.extend_from_slice(&data);

                if !submit_state.start_submit {
                    log::info!("Start submitting audio");
                    server
                        .send_client_command(protocol::ClientCommand::StartChat)
                        .await?;
                    log::info!("Submitted StartChat command");
                    gui.set_state("Listening...".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                    submit_state.start_submit = true;
                    submit_state.got_asr_result = false;
                }

                if submit_state.audio_buffer.len() >= 8192 && submit_state.submit_audio > 0.3 {
                    server
                        .send_client_audio_chunk_i16(submit_state.audio_buffer)
                        .await?;
                    submit_state.audio_buffer = Vec::with_capacity(8192);

                    if submit_state.submit_audio > 10.0 && !submit_state.got_asr_result {
                        log::info!("No ASR result after 10s audio, ending request");
                        crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                        submit_state.clear();

                        state = State::Listening;
                        gui.set_state("Ready".to_string());
                        gui.render_to_target(framebuffer)?;
                        framebuffer.flush()?;
                        recv_audio_buffer.clear();
                    }
                }
            }
            Event::MicAudioChunk(data) if state == State::Speaking && allow_interrupt => {
                submit_state.submit_audio += data.len() as f32 / 16000.0;
                submit_state.audio_buffer.extend_from_slice(&data);

                if submit_state.submit_audio > 0.6 {
                    state = State::Listening;
                    gui.set_state("Listening...".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;

                    server.reconnect_with_retry(3).await?;

                    submit_state.start_submit = true;
                    submit_state.got_asr_result = false;

                    server
                        .send_client_command(protocol::ClientCommand::StartChat)
                        .await?;

                    player_tx
                        .send(AudioEvent::ClearSpeech)
                        .map_err(|_| anyhow::anyhow!("Error sending clear"))?;
                }
            }
            Event::MicAudioChunk(_) => {
                log::info!("Received MicAudioChunk while no Listening/Speaking state, ignoring");
                audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            Event::MicAudioEnd => {
                log::info!("Received MicAudioEnd");
            }
            Event::ServerEvent(ServerEvent::ASR { text }) => {
                log::info!("Received ASR: {:?}", text);
                submit_state.got_asr_result = true;
                gui.set_state("ASR".to_string());
                gui.set_asr(text.trim().to_string());
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::ServerEvent(ServerEvent::Action { action }) => {
                log::info!("Received action");
                gui.set_state(format!("Action: {}", action));
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::ServerEvent(ServerEvent::Choices { message, items }) => {
                log::info!("Received choices");
                state = State::Choices;
                choices_index = 0;
                choices_items = items;
                let choices_string = choices_items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if i == choices_index {
                            format!("> {}", item)
                        } else {
                            format!("  {}", item)
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                gui.set_asr(message);
                gui.set_text(choices_string);
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::ServerEvent(ServerEvent::StartAudio { text }) => {
                start_audio = true;
                state = State::Speaking;
                log::info!("Received audio start: {:?}", text);
                gui.set_state(format!("[{:.2}x]|Speaking...", speed));
                gui.set_text(text.trim().to_string());
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
                player_tx
                    .send(AudioEvent::StartSpeech)
                    .map_err(|e| anyhow::anyhow!("Error sending start: {e:?}"))?;
            }
            Event::ServerEvent(ServerEvent::AudioChunki16 { data, vowel }) => {
                log::debug!("Received audio chunk");
                if state != State::Speaking {
                    log::debug!("Received audio chunk while not speaking");
                    continue;
                }

                if need_compute {
                    if start_audio {
                        metrics.reset();
                        start_audio = false;
                    }
                    metrics.add_data(data.len() * 2);
                }

                if speed < SPEED_LIMIT {
                    if let Err(e) = player_tx.send(AudioEvent::SpeechChunki16WithVowel(data, vowel))
                    {
                        log::error!("Error sending audio chunk: {:?}", e);
                        gui.set_state("Error on audio chunk".to_string());
                        gui.render_to_target(framebuffer)?;
                        framebuffer.flush()?;
                    }
                } else {
                    recv_audio_buffer.extend_from_slice(&data);
                }
            }
            Event::ServerEvent(ServerEvent::EndAudio) => {
                log::info!("Received audio end");

                if state != State::Speaking {
                    log::debug!("Received EndAudio while not in speaking state");
                    continue;
                }

                start_audio = false;

                if recv_audio_buffer.len() > 0 {
                    if let Err(e) = player_tx.send(AudioEvent::SpeechChunki16(recv_audio_buffer)) {
                        log::error!("Error sending audio chunk: {:?}", e);
                        gui.set_state("Error on audio chunk".to_string());
                        gui.render_to_target(framebuffer)?;
                        framebuffer.flush()?;
                    }
                    recv_audio_buffer = Vec::with_capacity(8192);
                }

                if let Err(e) = player_tx.send(AudioEvent::EndSpeech(notify.clone())) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.set_state("Error on audio chunk".to_string());
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
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
                crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                submit_state.clear();

                if state != State::Idle {
                    state = State::Listening;
                }
                gui.set_state("Ready".to_string());
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
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
                        gui.set_state("Error on hello end".to_string());
                        gui.render_to_target(framebuffer)?;
                        framebuffer.flush()?;
                    }
                    hello_wav = Vec::with_capacity(1024 * 30);
                    init_hello = true;
                }
            }

            Event::ServerEvent(ServerEvent::StartVideo | ServerEvent::EndVideo) => {}
            Event::ServerEvent(ServerEvent::AudioChunk { .. }) => {
                log::warn!("Received deprecated AudioChunk, please use AudioChunki16 instead");
            }
            Event::ServerEvent(ServerEvent::AudioChunkWithVowel { .. }) => {
                log::warn!(
                    "Received deprecated AudioChunkWithVowel, please use AudioChunki16 instead"
                );
            }
            Event::ServerEvent(ServerEvent::EndVad) => {
                log::info!("Received EndVad event from server");
                crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                if state != State::Listening && state != State::Speaking {
                    log::debug!("Received EndVad while no Listening/Speaking state, ignoring");
                    continue;
                }

                need_compute = metrics.is_timeout();

                submit_state.clear();

                wait_notify = false;
                state = State::Waiting;
                gui.set_state("Waiting...".to_string());
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::ServerEvent(ServerEvent::HasNotification) => {
                log::info!("Received HasNotification event from server");
                gui.set_state("Notification".to_string());
                gui.render_to_target(framebuffer)?;
                framebuffer.flush()?;
            }
            Event::ServerUrl(url) => {
                log::info!("Received ServerUrl: {}", url);
                if url != server.url {
                    init_hello = false;
                    server = Server::new(server.id, url).await?;
                    state = State::Idle;
                    gui.set_state("Idle".to_string());
                    gui.set_text(format!("Server URL updated:\n{}", server.url));
                    gui.render_to_target(framebuffer)?;
                    framebuffer.flush()?;
                }
            }
        }
    }

    log::info!("Main work done");

    Ok(())
}
