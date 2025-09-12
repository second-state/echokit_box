use std::sync::Arc;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2s::{config, I2sDriver, I2S0, I2S1};
use esp_idf_svc::sys::esp_sr;

const SAMPLE_RATE: u32 = 16000;
const PORT_TICK_PERIOD_MS: u32 = 1000 / esp_idf_svc::sys::configTICK_RATE_HZ;

#[allow(clippy::manual_c_str_literals)]
unsafe fn afe_init() -> (*mut esp_sr::esp_afe_sr_iface_t, *mut esp_sr::esp_afe_sr_data_t) {
    let models = esp_sr::esp_srmodel_init("model\0".as_ptr() as *const _);
    log::info!("models: {models:?}");
    let afe_config = esp_sr::afe_config_init(
        "M\0".as_ptr() as _,
        models,
        esp_sr::afe_type_t_AFE_TYPE_SR,
        esp_sr::afe_mode_t_AFE_MODE_HIGH_PERF,
    );

    let afe_config = afe_config.as_mut().unwrap();
    afe_config.pcm_config.total_ch_num = 1;
    afe_config.pcm_config.mic_num = 1;
    afe_config.pcm_config.ref_num = 1; // echo cancellation
    afe_config.pcm_config.sample_rate = 16000;
    afe_config.afe_ringbuf_size = 25;
    afe_config.vad_init = true;
    afe_config.vad_min_noise_ms = 500;
    afe_config.vad_min_speech_ms = 64;
    afe_config.vad_delay_ms = 128;
    afe_config.vad_mode = esp_sr::vad_mode_t_VAD_MODE_1;
    afe_config.agc_init = true;
    // wakeword
    afe_config.wakenet_init = true;
    afe_config.wakenet_mode = esp_sr::det_mode_t_DET_MODE_90;

    log::info!("afe ringbuf size: {}", afe_config.afe_ringbuf_size);

    log::info!("afe_config: {afe_config:#?}");

    let afe_handle = esp_sr::esp_afe_handle_from_config(afe_config);
    let afe_handle = afe_handle.as_mut().unwrap();
    let afe_data = (afe_handle.create_from_config.unwrap())(afe_config);
    let audio_chunksize = (afe_handle.get_feed_chunksize.unwrap())(afe_data);
    log::info!("audio chunksize: {audio_chunksize}");

    // 开启语音唤醒
    (afe_handle.enable_wakenet.unwrap())(afe_data);
    // (afe_handle.set_wakenet_threshold.unwrap())(afe_data, 1, 0.4);
    // (afe_handle.set_wakenet_threshold.unwrap())(afe_data, 2, 0.4);
    esp_sr::afe_config_free(afe_config);
    (afe_handle, afe_data)
}

#[allow(clippy::upper_case_acronyms)]
struct AFE {
    handle: *mut esp_sr::esp_afe_sr_iface_t,
    data: *mut esp_sr::esp_afe_sr_data_t,
    #[allow(unused)]
    feed_chunksize: usize,
}

unsafe impl Send for AFE {}
unsafe impl Sync for AFE {}

#[derive(Debug)]
struct AFEResult {
    data: Vec<u8>,
    speech: bool,
    wakeup_detected: bool,
    wake_word_index: i32,
    wakenet_model_index: i32,
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

    fn fetch(&self) -> Result<AFEResult, i32> {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            let result = (afe_handle.as_ref().unwrap().fetch.unwrap())(afe_data).as_mut().unwrap();

            if result.ret_value != 0 {
                return Err(result.ret_value);
            }

            let data_size = result.data_size;
            let vad_state = result.vad_state;
            let wakeup_state = result.wakeup_state;
            let wake_word_index = result.wake_word_index;
            let wakenet_model_index = result.wakenet_model_index;

            let mut data = Vec::with_capacity(data_size as usize + result.vad_cache_size as usize);
            if result.vad_cache_size > 0 {
                let data_ptr = result.vad_cache as *const u8;
                let data_ = std::slice::from_raw_parts(data_ptr, (result.vad_cache_size) as usize);
                data.extend_from_slice(data_);
            }
            if data_size > 0 {
                let data_ptr = result.data as *const u8;
                let data_ = std::slice::from_raw_parts(data_ptr, (data_size) as usize);
                data.extend_from_slice(data_);
            };

            let speech = vad_state == esp_sr::vad_state_t_VAD_SPEECH;
            let wakeup_detected = wakeup_state == esp_sr::wakenet_state_t_WAKENET_DETECTED;

            Ok(AFEResult {
                data,
                speech,
                wakeup_detected,
                wake_word_index,
                wakenet_model_index,
            })
        }
    }

    #[allow(dead_code)]
    fn get_feed_channel_num(&self) -> usize {
        unsafe { (self.handle.as_ref().unwrap().get_feed_channel_num.unwrap())(self.data) as usize }
    }
}

pub static WAKE_WAV: &[u8] = include_bytes!("../assets/hello.wav");

pub enum AudioData {
    Hello(tokio::sync::oneshot::Sender<()>),
    SetHelloStart,
    SetHelloChunk(Vec<u8>),
    SetHelloEnd,
    Start,
    Chunk(Vec<u8>),
    End(tokio::sync::oneshot::Sender<()>),
}

pub type PlayerTx = tokio::sync::mpsc::UnboundedSender<AudioData>;
pub type PlayerRx = tokio::sync::mpsc::UnboundedReceiver<AudioData>;
pub type MicTx = tokio::sync::mpsc::Sender<crate::app::Event>;

pub async fn i2s_task_(
    i2s: I2S0, ws: AnyIOPin, sck: AnyIOPin, din: AnyIOPin, i2s1: I2S1, bclk: AnyIOPin,
    lrclk: AnyIOPin, dout: AnyIOPin, (tx, rx): (MicTx, PlayerRx),
) {
    let afe_handle = Arc::new(AFE::new());
    let afe_handle_ = afe_handle.clone();
    let afe_r = std::thread::spawn(|| afe_worker(afe_handle_, tx));
    let r = i2s_player_(i2s, ws, sck, din, i2s1, bclk, lrclk, dout, afe_handle, rx).await;
    if let Err(e) = r {
        log::error!("Error: {e}");
    } else {
        log::info!("I2S test completed successfully");
    }
    let r = afe_r.join().unwrap();
    if let Err(e) = r {
        log::error!("Error: {e}");
    } else {
        log::info!("AFE worker completed successfully");
    }
}

async fn i2s_player_(
    i2s: I2S0, ws: AnyIOPin, sck: AnyIOPin, din: AnyIOPin, i2s1: I2S1, bclk: AnyIOPin,
    lrclk: AnyIOPin, dout: AnyIOPin, afe_handle: Arc<AFE>, mut rx: PlayerRx,
) -> anyhow::Result<()> {
    let i2s_config = config::StdConfig::new(
        config::Config::default().auto_clear(true),
        config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
        config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits16,
            config::SlotMode::Mono,
        ),
        config::StdGpioConfig::default(),
    );

    let mclk: Option<esp_idf_svc::hal::gpio::AnyIOPin> = None;
    let mut rx_driver = I2sDriver::new_std_rx(i2s, &i2s_config, sck, din, mclk, ws).unwrap();
    rx_driver.rx_enable()?;

    let mclk: Option<esp_idf_svc::hal::gpio::AnyIOPin> = None;
    let mut tx_driver = I2sDriver::new_std_tx(i2s1, &i2s_config, bclk, dout, mclk, lrclk).unwrap();
    tx_driver.tx_enable()?;

    // 10ms size of u16 = 2
    let mut buf = [0u8; 160 * 2 * 2];
    let feed_chunk_bytes: usize = afe_handle.feed_chunksize * 2;
    let mut mic_accumulator: Vec<u8> = Vec::with_capacity(4096);
    let mut speaking = false;
    log::info!("feed_chunksize: {}", afe_handle.feed_chunksize);

    let mut hello_audio = WAKE_WAV.to_vec();

    tx_driver.write_all(&hello_audio, 100 / PORT_TICK_PERIOD_MS)?;
    log::info!("Playing hello audio, waiting for response...");

    loop {
        let data = if speaking {
            rx.recv().await
        } else {
            tokio::select! {
                Some(data) = rx.recv() =>{
                    Some(data)
                }
                _ = async {} => {
                    for _ in 0..10{
                        let n = rx_driver.read(&mut buf, 100 / PORT_TICK_PERIOD_MS)?;
                        mic_accumulator.extend_from_slice(&buf[..n]);
                        while mic_accumulator.len() >= feed_chunk_bytes {
                            let chunk = mic_accumulator.drain(..feed_chunk_bytes).collect::<Vec<u8>>();
                            afe_handle.feed(&chunk);
                        }
                    }
                    None
                }
            }
        };
        if let Some(data) = data {
            match data {
                AudioData::Hello(tx) => {
                    log::info!("Received hello");
                    tx_driver
                        .write_all_async(&hello_audio)
                        .await
                        .map_err(|e| anyhow::anyhow!("Error play hello: {:?}", e))?;
                    let _ = tx.send(());
                    speaking = false;
                },
                AudioData::SetHelloStart => {
                    log::info!("Received set hello start");
                    hello_audio.clear();
                },
                AudioData::SetHelloChunk(data) => {
                    log::info!("Received set hello chunk");
                    hello_audio.extend(data);
                },
                AudioData::SetHelloEnd => {
                    log::info!("Received set hello end");
                    tx_driver
                        .write_all_async(&hello_audio)
                        .await
                        .map_err(|e| anyhow::anyhow!("Error play set hello: {:?}", e))?;
                },
                AudioData::Start => {
                    log::info!("Received start");
                    speaking = true;
                },
                AudioData::Chunk(data) => {
                    log::info!("Received audio chunk");
                    if speaking {
                        tx_driver
                            .write_all_async(&data)
                            .await
                            .map_err(|e| anyhow::anyhow!("Error play audio data: {:?}", e))?;
                    }
                },
                AudioData::End(tx) => {
                    log::info!("Received end");
                    let _ = tx.send(());
                    speaking = false;
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                },
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    // Ok(())
}

#[allow(unused)]
pub async fn i2s_task(
    i2s: I2S0, bclk: AnyIOPin, din: AnyIOPin, dout: AnyIOPin, ws: AnyIOPin,
    (tx, rx): (MicTx, PlayerRx),
) {
    let afe_handle = Arc::new(AFE::new());
    let afe_handle_ = afe_handle.clone();
    let afe_r = std::thread::spawn(|| afe_worker(afe_handle_, tx));
    let r = i2s_player(i2s, bclk, din, dout, ws, afe_handle, rx).await;
    if let Err(e) = r {
        log::error!("Error: {e}");
    } else {
        log::info!("I2S test completed successfully");
    }
    let r = afe_r.join().unwrap();
    if let Err(e) = r {
        log::error!("Error: {e}");
    } else {
        log::info!("AFE worker completed successfully");
    }
}

async fn i2s_player(
    i2s: I2S0, bclk: AnyIOPin, din: AnyIOPin, dout: AnyIOPin, ws: AnyIOPin, afe_handle: Arc<AFE>,
    mut rx: PlayerRx,
) -> anyhow::Result<()> {
    log::info!("PORT_TICK_PERIOD_MS = {PORT_TICK_PERIOD_MS}");
    let i2s_config = config::StdConfig::new(
        config::Config::default().auto_clear(true),
        config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
        config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits16,
            config::SlotMode::Mono,
        ),
        config::StdGpioConfig::default(),
    );

    let mclk: Option<esp_idf_svc::hal::gpio::AnyIOPin> = None;

    let mut driver = I2sDriver::new_std_bidir(i2s, &i2s_config, bclk, din, dout, mclk, ws).unwrap();
    driver.tx_enable()?;
    driver.rx_enable()?;

    let mut buf = [0u8; 2 * 160];
    let mut mic_accumulator: Vec<u8> = Vec::with_capacity(4096);
    let feed_chunk_bytes = afe_handle.feed_chunksize * core::mem::size_of::<i16>();
    let mut speaking = false;

    let mut hello_audio = WAKE_WAV.to_vec();

    driver.write_all(&hello_audio, 100 / PORT_TICK_PERIOD_MS)?;
    log::info!("Playing hello audio, waiting for response...");

    loop {
        let data = if speaking {
            rx.recv().await
        } else {
            tokio::select! {
                Some(data) = rx.recv() =>{
                    Some(data)
                }
                _ = async {} => {
                    let n = driver.read(&mut buf, 100 / PORT_TICK_PERIOD_MS)?;
                    mic_accumulator.extend_from_slice(&buf[..n]);
                    while mic_accumulator.len() >= feed_chunk_bytes {
                        let chunk = mic_accumulator.drain(..feed_chunk_bytes).collect::<Vec<u8>>();
                        afe_handle.feed(&chunk);
                    }
                    None
                }
            }
        };
        if let Some(data) = data {
            match data {
                AudioData::Hello(tx) => {
                    log::info!("Received hello");
                    driver
                        .write_all_async(&hello_audio)
                        .await
                        .map_err(|e| anyhow::anyhow!("Error play hello: {:?}", e))?;
                    log::info!("Hello audio sent, notifying");
                    let _ = tx.send(());
                    log::info!("Hello audio sent, notifying done");
                    speaking = false;
                },
                AudioData::SetHelloStart => {
                    log::info!("Received set hello start");
                    hello_audio.clear();
                },
                AudioData::SetHelloChunk(data) => {
                    log::info!("Received set hello chunk");
                    hello_audio.extend(data);
                },
                AudioData::SetHelloEnd => {
                    log::info!("Received set hello end");
                    driver
                        .write_all_async(&hello_audio)
                        .await
                        .map_err(|e| anyhow::anyhow!("Error play set hello: {:?}", e))?;
                },
                AudioData::Start => {
                    log::info!("Received start");
                    speaking = true;
                },
                AudioData::Chunk(data) => {
                    log::info!("Received audio chunk");
                    if speaking {
                        driver
                            .write_all_async(&data)
                            .await
                            .map_err(|e| anyhow::anyhow!("Error play audio data: {:?}", e))?;
                    }
                },
                AudioData::End(tx) => {
                    log::info!("Received end");
                    let _ = tx.send(());
                    speaking = false;
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                },
            }
        } else {
            tokio::task::yield_now().await;
        }
    }

    // Ok(())
}

fn afe_worker(afe_handle: Arc<AFE>, tx: MicTx) -> anyhow::Result<()> {
    let mut speech = false;
    let mut wake_word_detected = false;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(10));

        let result = afe_handle.fetch();
        if let Err(_e) = &result {
            continue;
        }
        let result = result.unwrap();

        // Check for wakeword detection first
        if result.wakeup_detected && !wake_word_detected {
            wake_word_detected = true;
            log::info!(
                "Wakeword detected: model_index={}, wake_word_index={}",
                result.wakenet_model_index,
                result.wake_word_index
            );
            tx.blocking_send(crate::app::Event::WakewordDetected {
                model_index: result.wakenet_model_index,
                wake_word_index: result.wake_word_index,
            })
            .map_err(|_| anyhow::anyhow!("Failed to send wakeword event"))?;
        }

        if result.data.is_empty() {
            continue;
        }

        if result.speech {
            speech = true;
            // no forward speech data when wake word detected
            if wake_word_detected {
                continue;
            }
            log::debug!("Speech detected, sending {} bytes", result.data.len());
            tx.blocking_send(crate::app::Event::MicAudioChunk(result.data))
                .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            continue;
        }

        if speech {
            log::info!("Speech ended");
            tx.blocking_send(crate::app::Event::MicAudioEnd)
                .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            speech = false;
            wake_word_detected = false;
        }
    }
}

const WELCOME_WAV: &[u8] = include_bytes!("../assets/hello.wav");

pub fn player_welcome(
    i2s: I2S0, bclk: AnyIOPin, dout: AnyIOPin, lrclk: AnyIOPin, mclk: Option<AnyIOPin>,
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
