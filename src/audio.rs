use std::sync::Arc;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2s::{config, I2sDriver, I2S0, I2S1};

use esp_idf_svc::sys::esp_sr;

const SAMPLE_RATE: u32 = 16000;

unsafe fn afe_init() -> (
    *mut esp_sr::esp_afe_sr_iface_t,
    *mut esp_sr::esp_afe_sr_data_t,
) {
    let models = esp_sr::esp_srmodel_init(c"model".as_ptr());
    let afe_config = esp_sr::afe_config_init(
        c"MR".as_ptr() as _,
        models,
        esp_sr::afe_type_t_AFE_TYPE_SR,
        esp_sr::afe_mode_t_AFE_MODE_HIGH_PERF,
    );
    let afe_config = afe_config.as_mut().unwrap();

    afe_config.pcm_config.sample_rate = 16000;
    afe_config.afe_ringbuf_size = 40;
    afe_config.vad_min_noise_ms = 500;
    afe_config.vad_min_speech_ms = 400;
    afe_config.vad_mode = esp_sr::vad_mode_t_VAD_MODE_4;
    afe_config.agc_init = true;
    afe_config.afe_linear_gain = 2.0;
    afe_config.aec_init = true;
    afe_config.aec_mode = esp_sr::aec_mode_t_AEC_MODE_SR_HIGH_PERF;
    afe_config.ns_init = true;
    afe_config.wakenet_init = false;
    afe_config.memory_alloc_mode = esp_sr::afe_memory_alloc_mode_t_AFE_MEMORY_ALLOC_MORE_PSRAM;

    log::info!("{afe_config:?}");

    let afe_ringbuf_size = afe_config.afe_ringbuf_size;
    log::info!("afe ringbuf size: {}", afe_ringbuf_size);

    let afe_handle = esp_sr::esp_afe_handle_from_config(afe_config);
    let afe_handle = afe_handle.as_mut().unwrap();
    let afe_data = (afe_handle.create_from_config.unwrap())(afe_config);
    let audio_chunksize = (afe_handle.get_feed_chunksize.unwrap())(afe_data);
    log::info!("audio chunksize: {}", audio_chunksize);

    esp_sr::afe_config_free(afe_config);
    (afe_handle, afe_data)
}

struct AFE {
    handle: *mut esp_sr::esp_afe_sr_iface_t,
    data: *mut esp_sr::esp_afe_sr_data_t,
    #[allow(unused)]
    feed_chunksize: usize,
}

unsafe impl Send for AFE {}
unsafe impl Sync for AFE {}

struct AFEResult {
    data: Vec<i16>,
    speech: bool,
}

impl AFE {
    fn new() -> Self {
        unsafe {
            let (handle, data) = afe_init();
            let feed_chunksize =
                (handle.as_mut().unwrap().get_feed_chunksize.unwrap())(data) as usize;

            AFE {
                handle,
                data,
                feed_chunksize,
            }
        }
    }
    // returns the number of bytes fed

    #[allow(dead_code)]
    fn reset(&self) {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            (afe_handle.as_ref().unwrap().reset_vad.unwrap())(afe_data);
        }
    }

    fn feed(&self, data: &[u8]) -> i32 {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            (afe_handle.as_ref().unwrap().feed.unwrap())(afe_data, data.as_ptr() as *const i16)
        }
    }

    fn feed_i16(&self, data: &[i16]) -> i32 {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe { (afe_handle.as_ref().unwrap().feed.unwrap())(afe_data, data.as_ptr()) }
    }

    fn fetch(&self) -> Result<AFEResult, i32> {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            let result = (afe_handle.as_ref().unwrap().fetch.unwrap())(afe_data)
                .as_mut()
                .unwrap();

            if result.ret_value != 0 {
                return Err(result.ret_value);
            }

            let data_size = result.data_size;
            let vad_state = result.vad_state;
            let mut data = Vec::with_capacity((data_size + result.vad_cache_size) as usize / 2);
            if result.vad_cache_size > 0 {
                let data_ = std::slice::from_raw_parts(
                    result.vad_cache,
                    result.vad_cache_size as usize / 2,
                );
                data.extend_from_slice(data_);
            }
            if data_size > 0 {
                let data_ = std::slice::from_raw_parts(result.data, data_size as usize / 2);
                data.extend_from_slice(data_);
            }

            let speech = vad_state == esp_sr::vad_state_t_VAD_SPEECH;
            Ok(AFEResult { data, speech })
        }
    }
}

pub static WAKE_WAV: &[u8] = include_bytes!("../assets/hello_beep.wav");

pub type PlayerTx = tokio::sync::mpsc::UnboundedSender<AudioEvent>;
pub type PlayerRx = tokio::sync::mpsc::UnboundedReceiver<AudioEvent>;
pub type MicTx = tokio::sync::mpsc::Sender<crate::app::Event>;
pub type MicRx = tokio::sync::mpsc::Receiver<crate::app::Event>;

fn afe_worker(afe_handle: Arc<AFE>, tx: MicTx, trigger_mean_value: f32) -> anyhow::Result<()> {
    log::info!("AFE worker started");
    crate::log_heap();
    crate::print_stack_high();
    let mut speech = false;
    let mut cache_buffer = Vec::with_capacity(16000);
    loop {
        let result = afe_handle.fetch();
        if let Err(_e) = &result {
            continue;
        }
        let result = result.unwrap();
        if result.data.is_empty() {
            continue;
        }

        if result.speech {
            if !speech {
                log::info!("Speech started");
            }
            speech = true;
            log::debug!("Speech detected, sending {} bytes", result.data.len());
            if PLAYING.load(std::sync::atomic::Ordering::Relaxed) {
                cache_buffer.extend_from_slice(&result.data);
            } else {
                tx.blocking_send(crate::app::Event::MicAudioChunk(result.data))
                    .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            }
            continue;
        }

        if speech {
            log::info!("Speech ended");
            if cache_buffer.is_empty() {
                tx.blocking_send(crate::app::Event::MicAudioChunk(result.data))
                    .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            } else {
                cache_buffer.extend_from_slice(&result.data);
                let len = cache_buffer.len() as f32;
                let mean = cache_buffer
                    .iter()
                    .map(|x| x.abs() as f32 / len)
                    .sum::<f32>();

                if mean > trigger_mean_value {
                    log::info!("Sending cached {} bytes, mean:{}", cache_buffer.len(), mean);
                    tx.blocking_send(crate::app::Event::MicAudioChunk(cache_buffer))
                        .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
                    cache_buffer = Vec::with_capacity(16000);
                } else {
                    log::info!(
                        "Dropping cached {} bytes, mean:{} below trigger {}",
                        cache_buffer.len(),
                        mean,
                        trigger_mean_value
                    );
                    cache_buffer.clear();
                }
            }
            tx.blocking_send(crate::app::Event::MicAudioEnd)
                .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            speech = false;
        }
    }
}

pub const WELCOME_WAV: &[u8] = include_bytes!("../assets/welcome.wav");

pub fn player_welcome(
    i2s: I2S0,
    bclk: AnyIOPin,
    dout: AnyIOPin,
    lrclk: AnyIOPin,
    mclk: Option<AnyIOPin>,
    data: Option<&[u8]>,
) {
    let i2s_config = config::StdConfig::new(
        config::Config::default().auto_clear(true),
        config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
        config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits16,
            config::SlotMode::Mono,
        ),
        config::StdGpioConfig::default(),
    );

    let mut tx_driver = I2sDriver::new_std_tx(i2s, &i2s_config, bclk, dout, mclk, lrclk).unwrap();

    tx_driver.tx_enable().unwrap();

    if let Some(data) = data {
        tx_driver.write_all(data, 1000).unwrap();
    } else {
        tx_driver.write_all(WELCOME_WAV, 1000).unwrap();
    }
}

pub enum AudioEvent {
    Hello(tokio::sync::oneshot::Sender<()>),
    SetHello(Vec<u8>),
    StartSpeech,
    SpeechChunk(Vec<u8>),
    SpeechChunki16(Vec<i16>),
    EndSpeech(tokio::sync::oneshot::Sender<()>),
    StopSpeech,
}

enum SendBufferItem {
    Audio(Vec<i16>),
    EndSpeech(tokio::sync::oneshot::Sender<()>),
}
struct SendBuffer {
    cache: std::collections::LinkedList<SendBufferItem>,
    chunk_size: usize,
    pub volume: f32,
}

#[inline]
fn get_volume(value: i16, volume: f32) -> i16 {
    ((value as f32 / i16::MAX as f32 * volume) * (i16::MAX as f32)) as i16
}

impl SendBuffer {
    fn new(chunk_size: usize) -> Self {
        Self {
            cache: std::collections::LinkedList::new(),
            chunk_size,
            volume: 1.0,
        }
    }

    fn push_u8(&mut self, data: &[u8]) {
        for chunk in data.chunks(self.chunk_size * 2) {
            let mut v = vec![0i16; self.chunk_size];

            for i in 0..(chunk.len() / 2) {
                let v_ = i16::from_le_bytes([chunk[i * 2], chunk[i * 2 + 1]]);
                v[i] = get_volume(v_, self.volume);
            }
            self.cache.push_back(SendBufferItem::Audio(v));
        }
    }

    fn push_i16(&mut self, data: &[i16]) {
        for chunk in data.chunks(self.chunk_size) {
            let mut v = vec![0i16; self.chunk_size];
            for i in 0..chunk.len() {
                v[i] = get_volume(chunk[i], self.volume);
            }
            self.cache.push_back(SendBufferItem::Audio(v));
        }
    }

    fn push_back_end_speech(&mut self, tx: tokio::sync::oneshot::Sender<()>) {
        self.cache.push_back(SendBufferItem::EndSpeech(tx));
    }

    fn get_chunk(&mut self) -> Option<Vec<i16>> {
        loop {
            match self.cache.pop_front() {
                Some(SendBufferItem::Audio(v)) => return Some(v),
                Some(SendBufferItem::EndSpeech(tx)) => {
                    let _ = tx.send(());
                    continue;
                }
                None => return None,
            }
        }
    }
}

static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn audio_task_run(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AudioEvent>,
    fn_read: &mut dyn FnMut(&mut [i16]) -> Result<usize, esp_idf_svc::sys::EspError>,
    fn_write: &mut dyn FnMut(&[i16]) -> Result<usize, esp_idf_svc::sys::EspError>,
    afe_handle: &AFE,
) -> anyhow::Result<()> {
    let feed_chunksize = afe_handle.feed_chunksize;
    log::info!("feed_chunksize: {}", feed_chunksize);

    let mut read_buffer = vec![0i16; feed_chunksize];
    let mut send_buffer = SendBuffer::new(feed_chunksize);
    let empty_buffer = vec![0i16; feed_chunksize];
    let mut ref_data_: Option<Vec<i16>> = send_buffer.get_chunk();

    let offset = 0;

    let mut hello_wav = WAKE_WAV.to_vec();
    let mut allow_speech = false;
    let mut speech = false;

    send_buffer.volume = 0.2;

    loop {
        if let Ok(event) = rx.try_recv() {
            match event {
                AudioEvent::Hello(sender) => {
                    allow_speech = true;
                    send_buffer.push_u8(&hello_wav);
                    send_buffer.push_back_end_speech(sender);
                }
                AudioEvent::SetHello(hello) => {
                    hello_wav = hello;
                }
                AudioEvent::StartSpeech => {
                    allow_speech = true;
                }
                AudioEvent::SpeechChunk(items) => {
                    send_buffer.push_u8(&items);
                }
                AudioEvent::SpeechChunki16(items) => {
                    send_buffer.push_i16(&items);
                }
                AudioEvent::EndSpeech(sender) => {
                    send_buffer.push_back_end_speech(sender);
                }
                AudioEvent::StopSpeech => {
                    allow_speech = false;
                }
            }
        }
        let play_data_ = if allow_speech {
            send_buffer.get_chunk()
        } else {
            None
        };

        if play_data_.is_some() && !speech {
            speech = true;
            PLAYING.store(speech, std::sync::atomic::Ordering::Relaxed);
        } else if play_data_.is_none() && speech {
            speech = false;
            PLAYING.store(speech, std::sync::atomic::Ordering::Relaxed);
        }

        let play_data = play_data_.as_deref().unwrap_or(&empty_buffer);

        fn_write(play_data)?;

        let len = fn_read(&mut read_buffer)?;

        if len != feed_chunksize * 2 {
            log::warn!(
                "Read size mismatch: expected {}, got {}",
                feed_chunksize * 2,
                len
            );
            break;
        } else {
            let total = len / 2;
            let mut samples_with_ref = Vec::with_capacity(total);

            let ref_data = ref_data_.as_ref().unwrap_or(&empty_buffer);

            for i in 0..total {
                samples_with_ref.push(read_buffer[i]);
                if offset + i < total {
                    samples_with_ref.push(ref_data[offset + i])
                } else {
                    samples_with_ref.push(play_data[offset + i - total]);
                }
            }

            afe_handle.feed_i16(&samples_with_ref);
        }
        ref_data_ = play_data_;
    }

    log::warn!("I2S loop exited");
    Ok(())
}

pub struct BoxAudioWorker {
    pub i2s: I2S0,
    pub bclk: AnyIOPin,
    pub din: AnyIOPin,
    pub dout: AnyIOPin,
    pub ws: AnyIOPin,
    pub mclk: Option<AnyIOPin>,
}

impl BoxAudioWorker {
    pub fn run(self, mut rx: PlayerRx, tx: MicTx) -> anyhow::Result<()> {
        let i2s_config = config::StdConfig::new(
            config::Config::default()
                .auto_clear(true)
                .dma_buffer_count(2)
                .frames_per_buffer(512),
            config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
            config::StdSlotConfig::philips_slot_default(
                config::DataBitWidth::Bits16,
                config::SlotMode::Mono,
            ),
            config::StdGpioConfig::default(),
        );

        let mut driver = I2sDriver::new_std_bidir(
            self.i2s,
            &i2s_config,
            self.bclk,
            self.din,
            self.dout,
            self.mclk,
            self.ws,
        )
        .map_err(|e| anyhow::anyhow!("Error create RX: {:?}", e))?;

        let (mut rx_driver, mut tx_driver) = driver.split();
        rx_driver.rx_enable()?;
        tx_driver.tx_enable()?;

        let mut fn_write = |play_data: &[i16]| -> Result<usize, esp_idf_svc::sys::EspError> {
            tx_driver.write(
                unsafe {
                    std::slice::from_raw_parts(
                        play_data.as_ptr() as *const u8,
                        play_data.len() * std::mem::size_of::<i16>(),
                    )
                },
                esp_idf_svc::hal::delay::TickType::new_millis(50).0,
            )
        };

        let mut fn_read = |read_buffer: &mut [i16]| -> Result<usize, esp_idf_svc::sys::EspError> {
            let read_buffer_ = unsafe {
                std::slice::from_raw_parts_mut(
                    read_buffer.as_mut_ptr() as *mut u8,
                    read_buffer.len() * std::mem::size_of::<i16>(),
                )
            };

            rx_driver.read(
                read_buffer_,
                esp_idf_svc::hal::delay::TickType::new_millis(50).0,
            )
        };

        let afe_handle = Arc::new(AFE::new());
        let afe_handle_ = afe_handle.clone();
        crate::log_heap();

        // let mut conf = esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration::default();
        // conf.stack_alloc_caps = (esp_idf_svc::hal::task::thread::MallocCap::Spiram
        //     | esp_idf_svc::hal::task::thread::MallocCap::Cap8bit)
        //     .into();
        // // conf.stack_size = 40 * 1024;
        // let r = conf.set();
        // if let Err(e) = r {
        //     log::error!("Failed to set thread config: {:?}", e);
        // }
        let _afe_r = std::thread::Builder::new().stack_size(8 * 1024).spawn(|| {
            let r = afe_worker(afe_handle_, tx, 600.0);
            if let Err(e) = r {
                log::error!("AFE worker error: {:?}", e);
            }
        })?;

        audio_task_run(&mut rx, &mut fn_read, &mut fn_write, &afe_handle)
    }
}

pub struct BoardsAudioWorker {
    pub out_i2s: I2S1,
    pub out_ws: AnyIOPin,
    pub out_clk: AnyIOPin,
    pub dout: AnyIOPin,
    pub out_mclk: Option<AnyIOPin>,

    pub in_i2s: I2S0,
    pub in_ws: AnyIOPin,
    pub in_clk: AnyIOPin,
    pub din: AnyIOPin,
    pub in_mclk: Option<AnyIOPin>,
}

impl BoardsAudioWorker {
    pub fn run(self, mut rx: PlayerRx, tx: MicTx) -> anyhow::Result<()> {
        let i2s_config = config::StdConfig::new(
            config::Config::default()
                .auto_clear(false)
                .dma_buffer_count(2)
                .frames_per_buffer(512),
            config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
            config::StdSlotConfig::philips_slot_default(
                config::DataBitWidth::Bits16,
                config::SlotMode::Mono,
            ),
            config::StdGpioConfig::default(),
        );

        let mut rx_driver = I2sDriver::new_std_rx(
            self.in_i2s,
            &i2s_config,
            self.in_clk,
            self.din,
            self.in_mclk,
            self.in_ws,
        )
        .map_err(|e| anyhow::anyhow!("Error create RX: {:?}", e))?;
        rx_driver.rx_enable()?;

        let mut tx_driver = I2sDriver::new_std_tx(
            self.out_i2s,
            &i2s_config,
            self.out_clk,
            self.dout,
            self.out_mclk,
            self.out_ws,
        )
        .map_err(|e| anyhow::anyhow!("Error create TX: {:?}", e))?;
        tx_driver.tx_enable()?;

        let mut fn_read = |read_buffer: &mut [i16]| -> Result<usize, esp_idf_svc::sys::EspError> {
            let read_buffer_ = unsafe {
                std::slice::from_raw_parts_mut(
                    read_buffer.as_mut_ptr() as *mut u8,
                    read_buffer.len() * std::mem::size_of::<i16>(),
                )
            };

            rx_driver.read(
                read_buffer_,
                esp_idf_svc::hal::delay::TickType::new_millis(50).0,
            )
        };
        let mut fn_write = |play_data: &[i16]| -> Result<usize, esp_idf_svc::sys::EspError> {
            tx_driver.write(
                unsafe {
                    std::slice::from_raw_parts(
                        play_data.as_ptr() as *const u8,
                        play_data.len() * std::mem::size_of::<i16>(),
                    )
                },
                esp_idf_svc::hal::delay::TickType::new_millis(50).0,
            )
        };

        let afe_handle = Arc::new(AFE::new());
        let afe_handle_ = afe_handle.clone();

        let _afe_r = std::thread::Builder::new()
            .stack_size(8 * 1024)
            .spawn(|| afe_worker(afe_handle_, tx, 500.0))?;

        audio_task_run(&mut rx, &mut fn_read, &mut fn_write, &afe_handle)
    }
}

pub fn echo_test(mut rx: MicRx, mut tx: PlayerTx) -> anyhow::Result<()> {
    let mut record_sample = Vec::with_capacity(1024);

    loop {
        match rx.blocking_recv() {
            Some(crate::app::Event::MicAudioChunk(data)) => {
                record_sample.extend_from_slice(&data);
            }
            Some(crate::app::Event::MicAudioEnd) => {
                let len = record_sample.len() as f32;
                let mean = record_sample
                    .iter()
                    .map(|x| x.abs() as f32 / len)
                    .sum::<f32>();

                log::info!(
                    "MicAudioEnd, sending back {} bytes mean:{mean}",
                    len / 16000.0
                );
                let (sender, receiver) = tokio::sync::oneshot::channel();
                tx.send(AudioEvent::StartSpeech)?;
                tx.send(AudioEvent::SpeechChunki16(record_sample.clone()))?;
                tx.send(AudioEvent::EndSpeech(sender))?;
                let _ = receiver.blocking_recv();
                record_sample.clear();
            }
            Some(_) => {}
            None => break,
        }
    }
    log::warn!("Echo test exited");
    Ok(())
}
