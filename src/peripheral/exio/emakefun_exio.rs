use esp_idf_svc::hal::i2c::Operation;

pub enum GpioMode {
    None = 0,
    InputPullUp = 1 << 0,
    InputPullDown = 1 << 1,
    InputFloating = 1 << 2,
    Output = 1 << 3,
    Adc = 1 << 4,
    Pwm = 1 << 5,
}

pub enum GpioPin {
    E0 = 0,
    E1 = 1,
    E2 = 2,
    E3 = 3,
    E4 = 4,
    E5 = 5,
    E6 = 6,
    E7 = 7,
}

const ADDRESS_IO_MODE: u8 = 0x01;
const ADDRESS_ANALOG_VALUES: u8 = 0x10;
const ADDRESS_VOLTAGE_VALUES: u8 = 0x20;
const ADDRESS_RATIO_VOLTAGE: u8 = 0x30;
const ADDRESS_DIGITAL_VALUES: u8 = 0x40;
const ADDRESS_PWM_DUTY: u8 = 0x50;
const ADDRESS_PWM_FREQUENCY: u8 = 0x60;

pub fn set_gpio_mode(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    dev_i2c_address: u8,
    pin: GpioPin,
    mode: GpioMode,
) -> anyhow::Result<()> {
    i2c.transaction(
        dev_i2c_address,
        &mut [Operation::Write(&[ADDRESS_IO_MODE + pin as u8, mode as u8])],
        esp_idf_svc::hal::delay::TickType::new_millis(1000).0,
    )
    .map_err(|e| anyhow::anyhow!("I2C write error: {:?}", e))?;
    Ok(())
}

fn set_gpio_level(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    dev_i2c_address: u8,
    pin: GpioPin,
    level: u8,
) -> anyhow::Result<()> {
    i2c.transaction(
        dev_i2c_address,
        &mut [Operation::Write(&[
            ADDRESS_DIGITAL_VALUES + pin as u8,
            level,
        ])],
        esp_idf_svc::hal::delay::TickType::new_millis(1000).0,
    )
    .map_err(|e| anyhow::anyhow!("I2C write error: {:?}", e))?;
    Ok(())
}

pub fn set_gpio_level_high(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    dev_i2c_address: u8,
    pin: GpioPin,
) -> anyhow::Result<()> {
    set_gpio_level(i2c, dev_i2c_address, pin, 1)
}

pub fn set_gpio_level_low(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    dev_i2c_address: u8,
    pin: GpioPin,
) -> anyhow::Result<()> {
    set_gpio_level(i2c, dev_i2c_address, pin, 0)
}

/// Read GPIO level
///
/// Returns: 0:low or 1:high
pub fn read_gpio_level(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    dev_i2c_address: u8,
    pin: GpioPin,
) -> anyhow::Result<u8> {
    let mut read = [0; 1];
    i2c.transaction(
        dev_i2c_address,
        &mut [
            Operation::Write(&[ADDRESS_DIGITAL_VALUES + pin as u8]),
            Operation::Read(&mut read),
        ],
        esp_idf_svc::hal::delay::TickType::new_millis(1000).0,
    )
    .map_err(|e| anyhow::anyhow!("I2C transaction error: {:?}", e))?;
    Ok(read[0])
}
