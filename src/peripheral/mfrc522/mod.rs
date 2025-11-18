use esp_idf_svc::sys::TickType_t;

pub mod consts;
// pub mod debug;
pub mod drivers;
pub mod mifare;
pub mod pcd;
pub mod picc;

use consts::{PCDErrorCode, Uid, UidSize};

pub trait MfrcDriver {
    fn write_reg(&mut self, reg: u8, val: u8, timeout: TickType_t) -> Result<(), PCDErrorCode>;
    fn write_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        values: &[u8],
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode>;
    fn read_reg(&mut self, reg: u8, timeout: TickType_t) -> Result<u8, PCDErrorCode>;
    fn read_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        output_buff: &mut [u8],
        rx_align: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode>;
}

pub struct MFRC522<D>
where
    D: MfrcDriver,
{
    driver: D,
}

impl<D> MFRC522<D>
where
    D: MfrcDriver,
{
    pub fn new(driver: D) -> Self {
        Self { driver }
    }

    pub fn sleep(&self, time_ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(time_ms));
    }

    pub fn get_card(&mut self, size: UidSize, timeout: TickType_t) -> Result<Uid, PCDErrorCode> {
        let mut uid = Uid {
            size: size.to_byte(),
            sak: 0,
            uid_bytes: [0; 10],
        };

        self.picc_select(&mut uid, 0, timeout)?;
        Ok(uid)
    }

    pub fn write_reg(&mut self, reg: u8, val: u8, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        self.driver.write_reg(reg, val, timeout)
    }

    pub fn write_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        values: &[u8],
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.driver.write_reg_buff(reg, count, values, timeout)
    }

    pub fn read_reg(&mut self, reg: u8, timeout: TickType_t) -> Result<u8, PCDErrorCode> {
        self.driver.read_reg(reg, timeout)
    }

    pub fn read_reg_buff(
        &mut self,
        reg: u8,
        count: usize,
        output_buff: &mut [u8],
        rx_align: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.driver
            .read_reg_buff(reg, count, output_buff, rx_align, timeout)
    }
}

#[inline(always)]
pub fn tif<T>(expr: bool, true_val: T, false_val: T) -> T {
    if expr {
        true_val
    } else {
        false_val
    }
}
