use std::{collections::LinkedList, fmt::Debug};

use tokio::sync::mpsc;

use crate::{
    audio::{self, AudioEvent, EventRx},
    protocol::{self, ServerEvent},
    ui::DisplayTargetDrive,
    ws::Server,
};

pub enum Event {
    Event(&'static str),
    ServerEvent(ServerEvent),
    MicAudioChunk(Vec<i16>),
    MicAudioEnd,
    Vowel(u8),
    PlaybackEnded,
    #[cfg_attr(not(feature = "extra_server"), allow(unused))]
    ServerUrl(String),
}

impl Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Event(s) => write!(f, "Event::Event({})", s),
            Event::ServerEvent(e) => write!(f, "Event::ServerEvent({:?})", e),
            Event::MicAudioChunk(data) => write!(f, "Event::MicAudioChunk(len={})", data.len()),
            Event::MicAudioEnd => write!(f, "Event::MicAudioEnd"),
            Event::Vowel(v) => write!(f, "Event::Vowel({})", v),
            Event::PlaybackEnded => write!(f, "Event::PlaybackEnded"),
            Event::ServerUrl(url) => write!(f, "Event::ServerUrl({})", url),
        }
    }
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
}

async fn select_evt(
    evt_rx: &mut mpsc::Receiver<Event>,
    server: &mut Server,
    block_server: bool,
) -> Option<Event> {
    struct NeverReady;
    impl std::future::Future for NeverReady {
        type Output = anyhow::Result<Event>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    let s_fut = async {
        if block_server {
            NeverReady.await
        } else {
            server.recv().await
        }
    };

    let timeout_f = tokio::time::sleep(NORMAL_TIMEOUT);

    tokio::select! {
        _ = timeout_f => {
            // log::info!("Event select timeout");
             Some(Event::Event(Event::IDLE))
        }
        Ok(msg) = s_fut => {
            Some(msg)
        }
        Some(evt) = evt_rx.recv() => {
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

#[derive(PartialEq, Eq)]
enum State {
    Listening,
    Submitting,
    Waiting,
    Choices { index: usize },
    Speaking { block_server: bool },
    Idle,
}

impl Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Listening => write!(f, "State::Listening"),
            State::Submitting => write!(f, "State::Submitting"),
            State::Waiting => write!(f, "State::Waiting"),
            State::Choices { index, .. } => {
                write!(f, "State::Choices(index={})", index)
            }
            State::Speaking { block_server } => {
                write!(f, "State::Speaking(block_server={})", block_server)
            }
            State::Idle => write!(f, "State::Idle"),
        }
    }
}

struct App<'gui, const N: usize> {
    state: State,
    submit_state: SubmitState,
    recv_audio_buffer: LinkedList<(Vec<i16>, u8)>,
    gui: &'gui mut crate::boards::ui::ChatUI<N>,
    framebuffer: &'gui mut crate::boards::ui::DisplayBuffer,

    vol: u8,
    hello_wav: Vec<u8>,
    set_hello: bool,

    metrics: DownloadMetrics,
    need_compute: bool,
    start_audio: bool,
    speed: f64,

    allow_interrupt: bool,
}

impl<'gui, const N: usize> App<'gui, N> {
    fn goto_idle(&mut self) -> anyhow::Result<()> {
        self.state = State::Idle;
        self.gui.set_state("Idle".to_string());
        self.gui.set_text(String::new());
        self.flush_gui()
    }

    fn goto_submitting(&mut self) -> anyhow::Result<()> {
        self.state = State::Submitting;
        self.gui.set_state("Listening...".to_string());
        self.gui.set_text(String::new());
        self.flush_gui()
    }

    fn goto_listening(&mut self) -> anyhow::Result<()> {
        self.recv_audio_buffer.clear();

        self.state = State::Listening;
        self.gui.set_state("Ready".to_string());
        // self.gui.set_text(String::new());
        self.flush_gui()
    }

    fn display_connecting(&mut self) -> anyhow::Result<()> {
        self.gui.set_state("Connecting...".to_string());
        self.gui.set_text(String::new());
        self.flush_gui()
    }

    fn goto_waiting(&mut self) -> anyhow::Result<()> {
        self.state = State::Waiting;
        self.gui.set_state("Waiting...".to_string());
        self.gui.set_text("".to_string());
        self.flush_gui()
    }

    fn goto_speaking(&mut self, speed: f64, text: String) -> anyhow::Result<()> {
        self.state = State::Speaking {
            block_server: false,
        };
        self.gui.set_state(format!("[{:.2}x]|Speaking...", speed));
        self.gui.set_text(text);
        self.flush_gui()
    }

    fn trigger_interrupt(&mut self) -> anyhow::Result<()> {
        self.allow_interrupt = !self.allow_interrupt;
        self.gui.set_allow_interrupt(self.allow_interrupt);
        self.flush_gui()
    }

    fn flush_gui(&mut self) -> anyhow::Result<()> {
        self.gui.render_to_target(self.framebuffer)?;
        self.framebuffer.flush()?;
        Ok(())
    }

    fn handle_key_up(&mut self, player_tx: &audio::PlayerTx) -> anyhow::Result<()> {
        match &mut self.state {
            State::Choices { index } => {
                *index = self.gui.update_choice_index(*index + 1);
                self.flush_gui()?;
            }
            State::Listening | State::Speaking { .. } => {
                // increase volume
                self.vol += 1;
                if self.vol > 5 {
                    self.vol = 5;
                }
                player_tx
                    .send(AudioEvent::VolSet(self.vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", self.vol);
                self.gui.set_volume(self.vol);
                self.flush_gui()?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_key_down(&mut self, player_tx: &audio::PlayerTx) -> anyhow::Result<()> {
        match &mut self.state {
            State::Choices { index } => {
                if *index > 0 {
                    *index = self.gui.update_choice_index(*index - 1);
                    self.flush_gui()?;
                }
            }
            State::Listening | State::Speaking { .. } => {
                // decrease volume
                if self.vol > 1 {
                    self.vol -= 1;
                }
                player_tx
                    .send(AudioEvent::VolSet(self.vol))
                    .map_err(|e| anyhow::anyhow!("Error sending volume set: {e:?}"))?;
                log::info!("Volume set to {}", self.vol);
                self.gui.set_volume(self.vol);
                self.flush_gui()?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_key0(
        &mut self,
        server: &mut Server,
        player_tx: &audio::PlayerTx,
    ) -> anyhow::Result<()> {
        match &self.state {
            State::Listening | State::Waiting | State::Submitting => {
                self.goto_idle()?;
            }
            State::Choices { index } => {
                // select choice
                log::info!("Selected choice index {}", index);
                server.send_client_select(*index).await?;
                self.goto_waiting()?;
            }
            State::Idle => {
                self.display_connecting()?;
                if let Err(e) = server.reconnect_with_retry(3).await {
                    log::error!("Error reconnecting to server: {:?}", e);
                    self.gui.set_state("Connect Server Failed".to_string());
                    self.flush_gui()?;
                    return Ok(());
                }
                self.submit_state.clear();
                self.goto_listening()?;
            }
            State::Speaking { .. } => {
                log::info!("Interrupting speaking");

                if let Err(e) = server.reconnect_with_retry(3).await {
                    log::error!("Error reconnecting to server: {:?}", e);
                    self.gui.set_state("Reconnect Server Failed".to_string());
                    self.flush_gui()?;
                    return Ok(());
                }

                self.submit_state.got_asr_result = false;

                player_tx
                    .send(AudioEvent::ClearSpeech)
                    .map_err(|_| anyhow::anyhow!("Error sending clear"))?;

                self.goto_listening()?;
            }
            _ => {
                self.goto_idle()?;
            }
        }

        Ok(())
    }

    async fn handle_mic_audio(
        &mut self,
        server: &mut Server,
        player_tx: &audio::PlayerTx,
        data: Vec<i16>,
    ) -> anyhow::Result<()> {
        match &self.state {
            State::Listening | State::Submitting => {
                self.submit_state.submit_audio += data.len() as f32 / 16000.0;
                self.submit_state.audio_buffer.extend_from_slice(&data);

                if self.state == State::Listening {
                    log::info!("Start submitting audio");
                    server
                        .send_client_command(protocol::ClientCommand::StartChat)
                        .await?;
                    self.goto_submitting()?;

                    self.submit_state.got_asr_result = false;
                }

                if self.submit_state.audio_buffer.len() >= 8192
                    && self.submit_state.submit_audio > 0.3
                {
                    let mut submit_audio_data = Vec::with_capacity(8192);
                    std::mem::swap(&mut submit_audio_data, &mut self.submit_state.audio_buffer);

                    server
                        .send_client_audio_chunk_i16(submit_audio_data)
                        .await?;

                    if self.submit_state.submit_audio > 10.0 && !self.submit_state.got_asr_result {
                        log::info!("No ASR result after 10s audio, ending request");
                        crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                        self.submit_state.clear();
                    }
                }
            }
            State::Speaking { .. } if self.allow_interrupt => {
                self.submit_state.submit_audio += data.len() as f32 / 16000.0;
                self.submit_state.audio_buffer.extend_from_slice(&data);

                if self.submit_state.submit_audio > 0.5 {
                    // TODO: interrupt current speaking, don't reconnect
                    if let Err(e) = server.reconnect_with_retry(3).await {
                        log::error!("Error reconnecting to server: {:?}", e);
                        self.gui.set_state("Reconnect Server Failed".to_string());
                        self.flush_gui()?;
                        return Ok(());
                    }

                    self.submit_state.got_asr_result = false;

                    server
                        .send_client_command(protocol::ClientCommand::StartChat)
                        .await?;

                    player_tx
                        .send(AudioEvent::ClearSpeech)
                        .map_err(|_| anyhow::anyhow!("Error sending clear"))?;

                    self.goto_submitting()?;
                }
            }

            _ => {
                log::debug!("Received MicAudioChunk while no Listening state, ignoring");
                audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Ok(())
    }

    async fn handle_playback_ended(&mut self) -> anyhow::Result<()> {
        match &mut self.state {
            State::Speaking { block_server } => {
                *block_server = false;
            }
            _ => {
                log::info!("Received PlaybackEnded while {:?}, ignoring", self.state);
            }
        }
        Ok(())
    }

    async fn handle_server_event(
        &mut self,
        player_tx: &audio::PlayerTx,
        evt: ServerEvent,
    ) -> anyhow::Result<()> {
        match evt {
            ServerEvent::EndVad => {
                log::info!("Received EndVad event from server");
                if self.state != State::Submitting {
                    log::debug!("Received EndVad while {:?}, ignoring", self.state);
                    return Ok(());
                }

                crate::audio::VAD_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

                self.need_compute = self.metrics.is_timeout();

                self.submit_state.clear();

                self.goto_waiting()?;
            }
            ServerEvent::ASR { text } => {
                log::info!("Received ASR: {:?}", text);
                self.submit_state.got_asr_result = true;
                self.gui.set_state("ASR".to_string());
                self.gui.set_asr(text.trim().to_string());
                self.flush_gui()?;
            }

            ServerEvent::Action { action } => {
                log::info!("Received action");
                self.gui.set_state(format!("{}", action));
                self.flush_gui()?;
            }

            ServerEvent::Choices { message, items } => {
                log::info!("Received choices");
                self.state = State::Choices { index: 0 };
                self.gui.set_choices(message, items);
                self.flush_gui()?;
            }

            ServerEvent::StartAudio { text } => {
                if !matches!(self.state, State::Waiting | State::Speaking { .. }) {
                    log::warn!("Received StartAudio while {:?}, ignoring", self.state);
                    return Ok(());
                }

                self.start_audio = true;
                self.goto_speaking(self.speed, text.trim().to_string())?;
                player_tx
                    .send(AudioEvent::StartSpeech)
                    .map_err(|e| anyhow::anyhow!("Error sending start: {e:?}"))?;
            }
            ServerEvent::AudioChunki16 { data, vowel } => {
                log::debug!("Received audio chunk");
                if let State::Speaking { .. } = self.state {
                    if self.need_compute {
                        if self.start_audio {
                            self.metrics.reset();
                            self.start_audio = false;
                        }
                        self.metrics.add_data(data.len() * 2);
                    }

                    if self.speed < SPEED_LIMIT {
                        if let Err(e) =
                            player_tx.send(AudioEvent::SpeechChunki16WithVowel(data, vowel))
                        {
                            log::error!("Error sending audio chunk: {:?}", e);
                            self.gui.set_state("Error on audio chunk".to_string());
                            self.flush_gui()?;
                        }
                    } else {
                        self.recv_audio_buffer.push_back((data, vowel));
                    }
                } else {
                    log::debug!("Received audio chunk while not speaking");
                }
            }

            ServerEvent::AudioChunk { .. } => {
                log::debug!("Received audio chunk (non-i16), ignoring");
            }
            ServerEvent::AudioChunkWithVowel { .. } => {
                log::debug!("Received audio chunk with vowel (non-i16), ignoring");
            }
            ServerEvent::DisplayText { text } => {
                log::info!("Received display text: {}", text);
                self.gui.set_text(text);
                self.flush_gui()?;
            }
            ServerEvent::EndResponse => {
                log::info!("Received EndResponse");
                if self.state != State::Idle {
                    self.goto_listening()?;
                }
            }

            ServerEvent::EndAudio => {
                log::info!("Received audio end");

                if let State::Speaking { .. } = self.state {
                    self.start_audio = false;

                    while let Some((data, vowel)) = self.recv_audio_buffer.pop_front() {
                        if let Err(e) =
                            player_tx.send(AudioEvent::SpeechChunki16WithVowel(data, vowel))
                        {
                            log::error!("Error sending audio chunk: {:?}", e);
                            self.gui.set_state("Error on audio chunk".to_string());
                            self.flush_gui()?;
                        }
                    }

                    if let Err(e) = player_tx.send(AudioEvent::EndSpeech) {
                        log::error!("Error sending audio chunk: {:?}", e);
                        self.gui.set_state("Error on audio chunk".to_string());
                        self.flush_gui()?;
                    } else {
                        if let State::Speaking { block_server } = &mut self.state {
                            *block_server = true;
                        }
                    }

                    if self.need_compute {
                        self.speed = self.metrics.speed();
                        self.need_compute = false;
                    }

                    log::info!("Audio speed: {:.2}x", self.speed);
                } else {
                    log::debug!("Received EndAudio while {:?}", self.state);
                }
            }
            ServerEvent::HelloStart => {
                if self.set_hello {
                    log::debug!("Received HelloStart, ignoring");
                    return Ok(());
                }
                log::info!("Received HelloStart");
                self.hello_wav.clear();
            }
            ServerEvent::HelloChunk { data } => {
                if self.set_hello {
                    log::debug!("Received HelloChunk, ignoring");
                    return Ok(());
                }
                self.hello_wav.extend_from_slice(&data);
            }
            ServerEvent::HelloEnd => {
                if self.set_hello {
                    log::debug!("Received HelloEnd, ignoring");
                    return Ok(());
                }
                log::info!(
                    "Received HelloEnd, total size: {} bytes",
                    self.hello_wav.len()
                );
                let hello = std::mem::take(&mut self.hello_wav);
                player_tx
                    .send(AudioEvent::SetHello(hello))
                    .map_err(|_| anyhow::anyhow!("Error sending hello wav"))?;
            }
            ServerEvent::HasNotification => {
                if self.state == State::Idle {
                    self.gui.set_state("Notification".to_string());
                    self.flush_gui()?;
                }
            }
            ServerEvent::StartVideo | ServerEvent::EndVideo => {
                log::debug!("Received {:?}, ignoring", evt);
            }
        }
        Ok(())
    }

    fn is_block_server(&self) -> bool {
        match &self.state {
            State::Speaking { block_server } => *block_server,
            _ => false,
        }
    }
}

pub async fn main_work<'d, const N: usize>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mut evt_rx: EventRx,
    framebuffer: &mut crate::boards::ui::DisplayBuffer,
    gui: &mut crate::boards::ui::ChatUI<N>,
) -> anyhow::Result<()> {
    let mut app = App {
        state: State::Idle,
        submit_state: SubmitState {
            submit_audio: 0.0,
            start_submit: false,
            audio_buffer: Vec::with_capacity(8192),
            got_asr_result: false,
        },
        recv_audio_buffer: Default::default(),
        gui,
        framebuffer,
        vol: 3u8,
        hello_wav: Vec::with_capacity(1024 * 30),
        set_hello: false,

        metrics: DownloadMetrics::new(),
        need_compute: true,
        start_audio: false,
        speed: 0.5,

        allow_interrupt: false,
    };

    app.goto_idle()?;

    while let Some(evt) = select_evt(&mut evt_rx, &mut server, app.is_block_server()).await {
        match evt {
            Event::Event(Event::K0) => {
                log::info!("Received event: k0");

                app.handle_key0(&mut server, &player_tx).await?;
            }
            Event::Event(Event::K0_) => {
                #[cfg(feature = "voice_interrupt")]
                {
                    app.trigger_interrupt()?;
                }
            }
            Event::Event(Event::VOL_UP) => {
                app.handle_key_up(&player_tx)?;
            }
            Event::Event(Event::VOL_DOWN) => {
                app.handle_key_down(&player_tx)?;
            }
            Event::Event(Event::YES | Event::K1) => {}
            Event::Event(Event::IDLE) => {
                app.goto_idle()?;
            }

            Event::Event(evt) => {
                log::info!("Received event: {:?}", evt);
            }
            Event::Vowel(v) => {
                if app.gui.set_avatar_index(v as usize) {
                    app.gui.render_to_target(app.framebuffer)?;
                    app.framebuffer.flush()?;
                }
            }

            Event::MicAudioChunk(data) => {
                app.handle_mic_audio(&mut server, &player_tx, data).await?;
            }
            Event::MicAudioEnd => {
                log::info!("Received MicAudioEnd");
            }
            Event::PlaybackEnded => {
                app.handle_playback_ended().await?;
            }
            Event::ServerEvent(server_event) => {
                app.handle_server_event(&player_tx, server_event).await?;
            }
            Event::ServerUrl(url) => {
                log::info!("Received ServerUrl: {}", url);
                if url != server.url {
                    app.set_hello = false;
                    server = Server::new(server.id, url).await?;
                    app.goto_idle()?;
                }
            }
        }
    }

    log::info!("Main work done");

    Ok(())
}
