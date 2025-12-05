use esp_idf_svc::{
    hal::{gpio::*, i2c::I2C0, i2s::I2S0},
    sys::EspError,
};

const AUDIO_STACK_SIZE: usize = 15 * 1024;
pub const AFE_AEC_OFFSET: usize = 512;

pub fn afe_config(afe_config: &mut esp_idf_svc::sys::esp_sr::afe_config_t) {
    afe_config.agc_init = true;
    afe_config.agc_mode = esp_idf_svc::sys::esp_sr::afe_agc_mode_t_AFE_AGC_MODE_WEBRTC;
    afe_config.afe_linear_gain = 1.0;
    afe_config.ns_init = false;
}

pub fn audio_init(_i2c: I2C0, _sda: Gpio48, _scl: Gpio45) {
    const SAMPLE_RATE: u32 = 16000;

    unsafe {
        use esp_idf_svc::sys::hal_driver;

        hal_driver::myiic_init();
        hal_driver::xl9555_init();
        hal_driver::es8311_init(SAMPLE_RATE as i32);
        hal_driver::xl9555_pin_write(hal_driver::SPK_CTRL_IO as _, 1);
        hal_driver::es8311_set_voice_volume(70);
        hal_driver::es8311_set_mic_gain(hal_driver::es8311_mic_gain_t_ES8311_MIC_GAIN_24DB);
        hal_driver::es8311_set_voice_mute(0); /* 打开DAC */
    }
}

pub fn start_audio_workers(
    i2s: I2S0,
    bclk: Gpio21,
    din: Gpio47,
    dout: Gpio14,
    ws: Gpio13,

    rx: crate::audio::PlayerRx,
    tx: crate::audio::EventTx,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    let worker = crate::audio::BoxAudioWorker {
        i2s,
        bclk: bclk.into(),
        din: din.into(),
        dout: dout.into(),
        ws: ws.into(),
        mclk: None,
    };

    let r = std::thread::Builder::new()
        .stack_size(AUDIO_STACK_SIZE)
        .spawn(move || {
            log::info!(
                "Starting audio worker thread in core {:?}",
                esp_idf_svc::hal::cpu::core()
            );
            let r = worker.run(rx, tx);
            if let Err(e) = r {
                log::error!("Audio worker error: {:?}", e);
            }
        })
        .map_err(|e| anyhow::anyhow!("Failed to spawn audio worker thread: {:?}", e))?;

    Ok(r)
}

pub fn start_btn_worker(
    rt: &tokio::runtime::Runtime,
    int_gpio: Gpio3,
    evt_tx: crate::audio::EventTx,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let mut int_gpio = esp_idf_svc::hal::gpio::PinDriver::input(int_gpio)?;
    int_gpio.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    int_gpio.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::NegEdge)?;

    Ok(rt.spawn(async move {
        loop {
            let r = int_gpio.wait_for_falling_edge().await;
            if let Err(e) = r {
                log::error!("Volume button wait error: {:?}", e);
                continue;
            }

            unsafe {
                let k = esp_idf_svc::sys::hal_driver::xl9555_key_scan(0) as u32;
                match k {
                    esp_idf_svc::sys::hal_driver::KEY0_PRES => {
                        log::info!("Volume up button pressed");
                        let r = evt_tx
                            .send(crate::app::Event::Event(crate::app::Event::VOL_UP))
                            .await;
                        if r.is_err() {
                            log::error!("Failed to send volume up event: {:?}", r.err());
                        }
                    }
                    esp_idf_svc::sys::hal_driver::KEY1_PRES => {
                        log::info!("Volume down button pressed");
                        let r = evt_tx
                            .send(crate::app::Event::Event(crate::app::Event::VOL_DOWN))
                            .await;
                        if r.is_err() {
                            log::error!("Failed to send volume down event: {:?}", r.err());
                        }
                    }
                    _ => {
                        log::debug!("Unknown key code: {}", k);
                    }
                }
            }
        }
    }))
}

pub fn set_volum(vol: u8) {
    let v = match vol {
        0..5 => vol as i32 * 50 / 5 + 20,
        _ => 70,
    };

    unsafe {
        esp_idf_svc::sys::hal_driver::es8311_set_voice_volume(v);
    }
}

pub const DISPLAY_WIDTH: usize = 320;
pub const DISPLAY_HEIGHT: usize = 240;

pub fn lcd_init(
    _cs: Gpio1,
    _dc: Gpio2,
    _rd: Gpio41,
    _wr: Gpio42,
    _lcd_data: (
        Gpio40,
        Gpio39,
        Gpio38,
        Gpio12,
        Gpio11,
        Gpio10,
        Gpio9,
        Gpio46,
    ),
) -> Result<(), EspError> {
    use esp_idf_svc::sys::hal_driver;
    unsafe {
        let config: hal_driver::lcd_cfg_t = std::mem::zeroed();
        hal_driver::lcd_init(config);
    }
    Ok(())
}

pub fn flush_display(color_data: &[u8], x_start: i32, y_start: i32, x_end: i32, y_end: i32) -> i32 {
    debug_assert_eq!(
        x_end - x_start,
        DISPLAY_WIDTH as i32,
        "x_end - x_start must be equal to DISPLAY_WIDTH"
    );
    unsafe {
        esp_idf_svc::sys::hal_driver::lcd_color_fill(
            x_start as u16,
            y_start as u16,
            x_end as u16,
            y_end as u16,
            color_data.as_ptr() as _,
        );
        0
    }
}

#[macro_export]
macro_rules! start_hal {
    ($peripherals:ident, $evt_tx:ident) => {{
        crate::boards::atom_box::audio_init(
            $peripherals.i2c0,
            $peripherals.pins.gpio48,
            $peripherals.pins.gpio45,
        );
        crate::boards::atom_box::lcd_init(
            $peripherals.pins.gpio1,
            $peripherals.pins.gpio2,
            $peripherals.pins.gpio41,
            $peripherals.pins.gpio42,
            (
                $peripherals.pins.gpio40,
                $peripherals.pins.gpio39,
                $peripherals.pins.gpio38,
                $peripherals.pins.gpio12,
                $peripherals.pins.gpio11,
                $peripherals.pins.gpio10,
                $peripherals.pins.gpio9,
                $peripherals.pins.gpio46,
            ),
        )?;
    }};
}

#[macro_export]
macro_rules! start_audio_workers {
    ($peripherals:ident, $rx:expr, $evt_tx:expr, $tokio_rt:expr) => {{
        crate::boards::atom_box::start_audio_workers(
            $peripherals.i2s0,
            $peripherals.pins.gpio21,
            $peripherals.pins.gpio47,
            $peripherals.pins.gpio14,
            $peripherals.pins.gpio13,
            $rx,
            $evt_tx,
        )?;
        crate::boards::atom_box::start_btn_worker($tokio_rt, $peripherals.pins.gpio3, $evt_tx)?;
    }};
}
