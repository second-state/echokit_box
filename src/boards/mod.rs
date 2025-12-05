#[cfg(feature = "box")]
pub mod atom_box;
#[cfg(feature = "box")]
pub use atom_box::*;

#[cfg(all(feature = "boards", not(feature = "_no_default")))]
pub mod base;
#[cfg(all(feature = "boards", not(feature = "_no_default")))]
pub use base::*;

#[cfg(feature = "cube")]
pub mod cube;
#[cfg(feature = "cube")]
pub use cube::*;

#[cfg(feature = "cube2")]
pub mod cube2;
#[cfg(feature = "cube2")]
pub use cube2::*;

#[cfg(feature = "i2c")]
pub type I2CInitFn = fn(&mut esp_idf_svc::hal::i2c::I2cDriver<'static>) -> anyhow::Result<()>;
#[cfg(feature = "i2c")]
pub type I2CLoopFn = fn(
    &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    &crate::audio::EventTx,
) -> anyhow::Result<()>;

#[cfg(feature = "i2c")]
pub fn init_i2c<
    I2C: esp_idf_svc::hal::i2c::I2c,
    P: esp_idf_svc::hal::peripheral::Peripheral<P = I2C> + 'static,
>(
    config: esp_idf_svc::hal::i2c::config::Config,
    i2c: P,
    sda: esp_idf_svc::hal::gpio::AnyIOPin,
    scl: esp_idf_svc::hal::gpio::AnyIOPin,
    event_tx: crate::audio::EventTx,
    tasks: Vec<(I2CInitFn, I2CLoopFn)>,
    stack_size: usize,
    loop_timeout_ms: u32,
) -> anyhow::Result<()> {
    if tasks.is_empty() {
        log::warn!("No I2C tasks to run");
        return Ok(());
    }

    let i2c_driver = esp_idf_svc::hal::i2c::I2cDriver::new(i2c, sda, scl, &config)
        .map_err(|e| anyhow::anyhow!("Failed to create I2C driver: {:?}", e))?;

    _ = std::thread::Builder::new()
        .stack_size(stack_size)
        .spawn(move || {
            log::info!(
                "Starting I2C worker thread in core {:?}",
                esp_idf_svc::hal::cpu::core()
            );
            let mut i2c_driver = i2c_driver;
            for (init_fn, _) in &tasks {
                if let Err(e) = init_fn(&mut i2c_driver) {
                    log::error!("I2C init function error: {:?}", e);
                }
            }
            loop {
                let now = std::time::Instant::now();
                for (_, loop_fn) in &tasks {
                    if let Err(e) = loop_fn(&mut i2c_driver, &event_tx) {
                        log::error!("I2C loop function error: {:?}", e);
                    }
                }
                let elapsed = now.elapsed();
                if elapsed.as_millis() < loop_timeout_ms as u128 {
                    std::thread::sleep(std::time::Duration::from_millis(
                        loop_timeout_ms as u64 - elapsed.as_millis() as u64,
                    ));
                }
            }
        });

    Ok(())
}

#[cfg(feature = "mfrc522")]
fn decode_ndef_in_mifare_ultralight<D: crate::peripheral::mfrc522::MfrcDriver>(
    mfrc522: &mut crate::peripheral::mfrc522::MFRC522<D>,
    timeout: esp_idf_svc::hal::delay::TickType_t,
) -> Result<Vec<String>, crate::peripheral::mfrc522::consts::PCDErrorCode> {
    let mut buff = [0; 18];

    let mut ndef_buffer = vec![];

    for page in (0..16).step_by(4) {
        let mut bytes_count = 18;
        mfrc522.mifare_read(page, &mut buff, &mut bytes_count, timeout)?;
        ndef_buffer.extend_from_slice(&buff[..16]);
    }

    let n = ndef_buffer[22] as usize;

    let messages = ndef::Message::try_from(&ndef_buffer[23..23 + n]).map_err(|e| {
        log::error!("Error parsing NDEF message: {:?}", e);
        crate::peripheral::mfrc522::consts::PCDErrorCode::Error
    })?;

    let mut r = vec![];
    for record in messages.records {
        if let ndef::Payload::RTD(ndef::RecordType::Text { txt, .. }) = record.payload {
            r.push(txt);
        }
    }

    Ok(r)
}

#[cfg(feature = "mfrc522")]
pub fn init_mfrc522(i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>) -> anyhow::Result<()> {
    let d = crate::peripheral::mfrc522::drivers::I2CDriver::new(i2c, 0x28);
    let mut mfrc522 = crate::peripheral::mfrc522::MFRC522::new(d);
    if let Err(e) = mfrc522.pcd_init(esp_idf_svc::hal::delay::TickType::new_millis(1000).0) {
        log::error!("Error initializing MFRC522: {:?}", e);
        return Err(anyhow::anyhow!("Error initializing MFRC522: {:?}", e));
    }

    if mfrc522.pcd_is_init(esp_idf_svc::hal::delay::TickType::new_millis(1000).0) {
        log::info!("MFRC522 initialized successfully");
        Ok(())
    } else {
        log::error!("Error checking MFRC522 initialization");
        Err(anyhow::anyhow!("Error checking MFRC522 initialization"))
    }
}

#[cfg(feature = "mfrc522")]
pub fn mfrc522_loop(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    evt_tx: &crate::audio::EventTx,
) -> anyhow::Result<()> {
    use crate::peripheral::mfrc522::consts::PICCType;

    let timeout = esp_idf_svc::hal::delay::TickType::new_millis(1000).0;

    let d = crate::peripheral::mfrc522::drivers::I2CDriver::new(i2c, 0x28);
    let mut mfrc522 = crate::peripheral::mfrc522::MFRC522::new(d);

    match mfrc522.picc_is_new_card_present(timeout) {
        Ok(_) => {
            match mfrc522.get_card(crate::peripheral::mfrc522::consts::UidSize::Four, timeout) {
                Ok(card) => {
                    log::info!("Card UID: {}", card.get_number());
                    let picc_type = PICCType::from_sak(card.sak);

                    log::info!("PICC Type: {:?}", picc_type);

                    if !matches!(picc_type, PICCType::PiccTypeMifareUL) {
                        return Ok(());
                    }

                    match decode_ndef_in_mifare_ultralight(&mut mfrc522, timeout) {
                        Ok(texts) => {
                            for text in texts {
                                log::info!("NDEF Text Record: {}", text);
                                evt_tx
                                    .blocking_send(crate::app::Event::ServerUrl(text))
                                    .unwrap_or_else(|e| {
                                        log::error!("Failed to send ServerUrl event: {:?}", e);
                                    });
                            }
                        }
                        Err(e) => {
                            log::error!("Error decoding NDEF message: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Error getting card UID: {:?}", e);
                }
            }
            _ = mfrc522.picc_halta(timeout);
        }
        Err(crate::peripheral::mfrc522::consts::PCDErrorCode::Timeout) => {
            // No card present
        }
        Err(e) => {
            log::error!("Error checking for new card: {:?}", e);
        }
    }

    Ok(())
}

#[allow(unused)]
pub fn backlight_init(
    bl_pin: esp_idf_svc::hal::gpio::AnyIOPin,
) -> anyhow::Result<esp_idf_svc::hal::ledc::LedcDriver<'static>> {
    use esp_idf_svc::hal;
    let config = hal::ledc::config::TimerConfig::new()
        .resolution(hal::ledc::Resolution::Bits13)
        .frequency(hal::units::Hertz(5000));
    let time = unsafe { hal::ledc::TIMER0::new() };
    let timer_driver = hal::ledc::LedcTimerDriver::new(time, &config)?;

    let ledc_driver =
        hal::ledc::LedcDriver::new(unsafe { hal::ledc::CHANNEL0::new() }, timer_driver, bl_pin)?;

    Ok(ledc_driver)
}

const LEDC_MAX_DUTY: u32 = (1 << 13) - 1;
#[allow(unused)]
pub fn set_backlight<'d>(
    ledc_driver: &mut esp_idf_svc::hal::ledc::LedcDriver<'d>,
    light: u8,
) -> anyhow::Result<()> {
    let light = 100.min(light) as u32;
    let duty = LEDC_MAX_DUTY - (81 * (100 - light));
    let duty = if light == 0 { 0 } else { duty };
    ledc_driver.set_duty(duty)?;
    Ok(())
}
