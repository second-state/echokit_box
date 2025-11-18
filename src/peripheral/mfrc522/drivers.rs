use esp_idf_svc::hal::i2c::Operation;
use esp_idf_svc::sys::TickType_t;

use super::{consts::PCDErrorCode, MfrcDriver};

pub struct I2CDriver<'d> {
    address: u8,
    i2c: esp_idf_svc::hal::i2c::I2cDriver<'d>,
}

impl<'d> I2CDriver<'d> {
    pub fn new(i2c: esp_idf_svc::hal::i2c::I2cDriver<'d>, addr: u8) -> Self {
        Self { address: addr, i2c }
    }
}

impl<'d> MfrcDriver for I2CDriver<'d> {
    fn write_reg(&mut self, reg: u8, val: u8, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        self.i2c
            .transaction(self.address, &mut [Operation::Write(&[reg, val])], timeout)
            .map_err(PCDErrorCode::from_i2c_error)?;

        Ok(())
    }

    fn write_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        values: &[u8],
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.i2c
            .transaction(
                self.address,
                &mut [Operation::Write(&[reg]), Operation::Write(&values[..count])],
                timeout,
            )
            .map_err(PCDErrorCode::from_i2c_error)?;

        Ok(())
    }

    fn read_reg(&mut self, reg: u8, timeout: TickType_t) -> Result<u8, PCDErrorCode> {
        let mut read = [0; 1];
        self.i2c
            .transaction(
                self.address,
                &mut [Operation::Write(&[reg]), Operation::Read(&mut read)],
                timeout,
            )
            .map_err(PCDErrorCode::from_i2c_error)?;

        Ok(read[0])
    }

    fn read_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        output_buff: &mut [u8],
        rx_align: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        if count == 0 {
            return Ok(());
        }

        let first_out_byte = output_buff[0];
        self.i2c
            .transaction(
                self.address,
                &mut [
                    Operation::Write(&[reg]),
                    Operation::Read(&mut output_buff[..count]),
                ],
                timeout,
            )
            .map_err(PCDErrorCode::from_i2c_error)?;

        if rx_align > 0 {
            let mask = 0xFF << rx_align;
            output_buff[0] = (first_out_byte & !mask) | (output_buff[0] & mask);
        }

        Ok(())
    }
}
