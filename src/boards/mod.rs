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
pub fn init_mfrc522<
    I2C: esp_idf_svc::hal::i2c::I2c,
    P: esp_idf_svc::hal::peripheral::Peripheral<P = I2C> + 'static,
>(
    i2c: P,
    sda: esp_idf_svc::hal::gpio::AnyIOPin,
    scl: esp_idf_svc::hal::gpio::AnyIOPin,
    evt_tx: crate::audio::EventTx,
) {
    let config = esp_idf_svc::hal::i2c::config::Config::default()
        .baudrate(esp_idf_svc::hal::units::Hertz(40_000));
    let i2c_d = esp_idf_svc::hal::i2c::I2cDriver::new(i2c, sda, scl, &config).unwrap();
    let timeout = esp_idf_svc::hal::delay::TickType::new_millis(1000).0;

    let d = crate::peripheral::mfrc522::drivers::I2CDriver::new(i2c_d, 0x28);
    let mfrc522 = crate::peripheral::mfrc522::MFRC522::new(d);

    let _ = std::thread::Builder::new()
        .stack_size(8 * 1024)
        .spawn(move || {
            log::info!(
                "Starting MFRC522 worker thread in core {:?}",
                esp_idf_svc::hal::cpu::core()
            );
            let mut mfrc522 = mfrc522;
            if let Err(e) = mfrc522.pcd_init(timeout) {
                log::error!("Error initializing MFRC522: {:?}", e);
                return;
            }

            if mfrc522.pcd_is_init(timeout) {
                log::info!("MFRC522 initialized successfully");
            } else {
                log::error!("MFRC522 initialization failed");
                return;
            }

            loop {
                match mfrc522.picc_is_new_card_present(timeout) {
                    Ok(_) => {
                        match mfrc522
                            .get_card(crate::peripheral::mfrc522::consts::UidSize::Four, timeout)
                        {
                            Ok(card) => {
                                log::info!("Card UID: {}", card.get_number());
                                let picc_type =
                                    crate::peripheral::mfrc522::consts::PICCType::from_sak(
                                        card.sak,
                                    );

                                log::info!("PICC Type: {:?}", picc_type);

                                if picc_type !=  crate::peripheral::mfrc522::consts::PICCType::PiccTypeMifareUL
                                {
                                                    std::thread::sleep(std::time::Duration::from_millis(1000));

                                    continue;
                                }

                                match decode_ndef_in_mifare_ultralight(&mut mfrc522, timeout) {
                                    Ok(texts) => {
                                        for text in texts {
                                            log::info!("NDEF Text Record: {}", text);
                                            evt_tx.blocking_send(
                                                crate::app::Event::ServerUrl(text),
                                            ).unwrap_or_else(|e| {
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
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        });
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
