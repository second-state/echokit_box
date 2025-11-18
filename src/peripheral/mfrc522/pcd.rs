use esp_idf_svc::sys::TickType_t;

use super::{
    consts::{PCDCommand, PCDErrorCode, PCDRegister, PCDVersion, Uid},
    MfrcDriver, MFRC522,
};

/// assert return boolean (false)
macro_rules! assert_rb {
    ($expr:expr, $expected:expr) => {
        if $expr != $expected {
            return false;
        }
    };
}

impl<D> MFRC522<D>
where
    D: MfrcDriver,
{
    pub fn pcd_init(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        log::info!("Initializing PCD...\n");
        self.pcd_reset(timeout)?;
        log::info!("PCD Reset complete.\n");

        self.write_reg(PCDRegister::TxModeReg, 0x00, timeout)?;
        self.write_reg(PCDRegister::RxModeReg, 0x00, timeout)?;
        self.write_reg(PCDRegister::ModWidthReg, 0x26, timeout)?;

        self.write_reg(PCDRegister::TModeReg, 0x80, timeout)?;
        self.write_reg(PCDRegister::TPrescalerReg, 0xA9, timeout)?;
        self.write_reg(PCDRegister::TReloadRegH, 0x03, timeout)?;
        self.write_reg(PCDRegister::TReloadRegL, 0xE8, timeout)?;

        self.write_reg(PCDRegister::TxASKReg, 0x40, timeout)?;
        self.write_reg(PCDRegister::ModeReg, 0x3D, timeout)?;

        self.pcd_antenna_on(timeout)?;

        self.sleep(4);
        Ok(())
    }

    pub fn pcd_is_init(&mut self, timeout: TickType_t) -> bool {
        assert_rb!(self.read_reg(PCDRegister::TxModeReg, timeout), Ok(0x00));
        assert_rb!(self.read_reg(PCDRegister::RxModeReg, timeout), Ok(0x00));
        assert_rb!(self.read_reg(PCDRegister::ModWidthReg, timeout), Ok(0x26));

        assert_rb!(self.read_reg(PCDRegister::TModeReg, timeout), Ok(0x80));
        assert_rb!(self.read_reg(PCDRegister::TPrescalerReg, timeout), Ok(0xA9));
        assert_rb!(self.read_reg(PCDRegister::TReloadRegH, timeout), Ok(0x03));
        assert_rb!(self.read_reg(PCDRegister::TReloadRegL, timeout), Ok(0xE8));

        assert_rb!(self.read_reg(PCDRegister::TxASKReg, timeout), Ok(0x40));
        assert_rb!(self.read_reg(PCDRegister::ModeReg, timeout), Ok(0x3D));
        true
    }

    pub fn pcd_reset(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        // self.spi.flush().await.map_err(|_| PCDErrorCode::Unknown)?;
        self.write_reg(PCDRegister::CommandReg, PCDCommand::SoftReset, timeout)?;

        // max 3 tries
        for _ in 0..3 {
            let out = self.read_reg(PCDRegister::CommandReg, timeout);
            if let Ok(out) = out {
                let out = out & (1 << 4);
                if out == 0 {
                    break;
                }
            }

            self.sleep(50);
        }

        Ok(())
    }

    pub fn pcd_antenna_on(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        let val = self.read_reg(PCDRegister::TxControlReg, timeout)?;
        if (val & 0x03) != 0x03 {
            self.write_reg(PCDRegister::TxControlReg, val | 0x03, timeout)?;
        }

        Ok(())
    }

    pub fn pcd_antenna_off(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        self.pcd_clear_register_bit_mask(PCDRegister::TxControlReg, 0x03, timeout)
    }

    pub fn pcd_get_antenna_gain(&mut self, timeout: TickType_t) -> Result<u8, PCDErrorCode> {
        let res = self.read_reg(PCDRegister::RFCfgReg, timeout)?;
        Ok(res & (0x07 << 4))
    }

    pub fn pcd_set_antenna_gain(
        &mut self,
        mask: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        if self.pcd_get_antenna_gain(timeout)? != mask {
            self.pcd_clear_register_bit_mask(PCDRegister::RFCfgReg, 0x07 << 4, timeout)?;

            self.pcd_set_register_bit_mask(PCDRegister::RFCfgReg, mask & (0x07 << 4), timeout)?;
        }

        Ok(())
    }

    pub fn pcd_get_version(&mut self, timeout: TickType_t) -> Result<PCDVersion, PCDErrorCode> {
        Ok(PCDVersion::from_byte(
            self.read_reg(PCDRegister::VersionReg, timeout)?,
        ))
    }

    pub fn pcd_soft_power_down(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        let mut val = self.read_reg(PCDRegister::CommandReg, timeout)?;
        val |= 1 << 4;
        self.write_reg(PCDRegister::CommandReg, val, timeout)?;

        Ok(())
    }

    pub fn pcd_soft_power_up(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        let mut val = self.read_reg(PCDRegister::CommandReg, timeout)?;
        val &= !(1 << 4);
        self.write_reg(PCDRegister::CommandReg, val, timeout)?;

        let start_time = std::time::Instant::now();
        while start_time.elapsed().as_micros() < 500_000 {
            let val = self.read_reg(PCDRegister::CommandReg, timeout)?;
            if val & (1 << 4) == 0 {
                return Ok(());
            }
        }

        Err(PCDErrorCode::Timeout)
    }

    pub fn pcd_stop_crypto1(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        self.pcd_clear_register_bit_mask(PCDRegister::Status2Reg, 0x08, timeout)
    }

    /// Key - 6 bytes
    pub fn pcd_authenticate(
        &mut self,
        cmd: u8,
        block_addr: u8,
        key: &[u8],
        uid: &Uid,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        if key.len() != 6 && key.len() != 0xA {
            return Err(PCDErrorCode::Invalid);
        }

        let wait_irq = 0x10;
        let mut send_data = [0; 12];
        send_data[0] = cmd;
        send_data[1] = block_addr;
        send_data[2..8].copy_from_slice(key);
        send_data[8..12]
            .copy_from_slice(&uid.uid_bytes[(uid.size as usize - 4)..(uid.size as usize)]);

        self.pcd_communicate_with_picc(
            PCDCommand::MFAuthent,
            wait_irq,
            &send_data,
            12,
            &mut [],
            None,
            None,
            0,
            false,
            timeout,
        )
    }

    pub fn pcd_mifare_transceive(
        &mut self,
        send_data: &[u8],
        mut send_len: u8,
        accept_timeout: bool,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let mut cmd_buff = [0; 18];
        if send_len > 16 {
            return Err(PCDErrorCode::Invalid);
        }

        cmd_buff[..send_len as usize].copy_from_slice(&send_data[..send_len as usize]);
        self.pcd_calc_crc(
            &send_data[..send_len as usize],
            send_len,
            &mut cmd_buff[send_len as usize..],
            timeout,
        )?;

        send_len += 2;

        let wait_irq = 0x30;
        let mut cmd_buff_size = 18;
        let mut valid_bits = 0;

        let res = self.pcd_communicate_with_picc(
            PCDCommand::Transceive,
            wait_irq,
            &cmd_buff.clone(),
            send_len,
            &mut cmd_buff,
            Some(&mut cmd_buff_size),
            Some(&mut valid_bits),
            0,
            false,
            timeout,
        );

        match res {
            Err(PCDErrorCode::Timeout) => {
                if accept_timeout {
                    return Ok(());
                }
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }

        if cmd_buff_size != 1 || valid_bits != 4 {
            return Err(PCDErrorCode::Error);
        }

        if cmd_buff[0] != 0xA {
            // MIFARE_Misc::MF_ACK type
            return Err(PCDErrorCode::MifareNack);
        }

        Ok(())
    }

    pub fn pcd_ntag216_auth(
        &mut self,
        password: [u8; 4],
        timeout: TickType_t,
    ) -> Result<[u8; 2], PCDErrorCode> {
        let mut cmd_buff = [0; 18];
        cmd_buff[0] = 0x1B;
        cmd_buff[1..5].copy_from_slice(&password);

        self.pcd_calc_crc_single_buf(&mut cmd_buff, 5, 5, timeout)?;

        let wait_irq = 0x30;
        let mut valid_bits = 0;
        let mut rx_length = 5;

        self.pcd_communicate_with_picc(
            PCDCommand::Transceive,
            wait_irq,
            &cmd_buff.clone(),
            7,
            &mut cmd_buff,
            Some(&mut rx_length),
            Some(&mut valid_bits),
            0,
            false,
            timeout,
        )?;

        Ok([cmd_buff[0], cmd_buff[1]])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pcd_transceive_data(
        &mut self,
        send_data: &[u8],
        send_len: u8,
        back_data: &mut [u8],
        back_len: Option<&mut u8>,
        valid_bits: Option<&mut u8>,
        rx_align: u8,
        check_crc: bool,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let wait_irq = 0x30;
        self.pcd_communicate_with_picc(
            PCDCommand::Transceive,
            wait_irq,
            send_data,
            send_len,
            back_data,
            back_len,
            valid_bits,
            rx_align,
            check_crc,
            timeout,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pcd_communicate_with_picc(
        &mut self,
        cmd: u8,
        wait_irq: u8,
        send_data: &[u8],
        send_len: u8,
        back_data: &mut [u8],
        mut back_len: Option<&mut u8>,
        valid_bits: Option<&mut u8>,
        rx_align: u8,
        check_crc: bool,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let tx_last_bits = if let Some(ref valid_bits) = valid_bits {
            **valid_bits
        } else {
            0
        };

        let bit_framing = (rx_align << 4) + tx_last_bits;

        self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

        self.write_reg(PCDRegister::ComIrqReg, 0x7F, timeout)?;
        self.write_reg(PCDRegister::FIFOLevelReg, 0x80, timeout)?;
        self.write_reg_buff(
            PCDRegister::FIFODataReg,
            send_len as usize,
            send_data,
            timeout,
        )?;

        self.write_reg(PCDRegister::BitFramingReg, bit_framing, timeout)?;

        self.write_reg(PCDRegister::CommandReg, cmd, timeout)?;

        if cmd == PCDCommand::Transceive {
            self.pcd_set_register_bit_mask(PCDRegister::BitFramingReg, 0x80, timeout)?;
        }

        let start_time = std::time::Instant::now();
        loop {
            let n = self.read_reg(PCDRegister::ComIrqReg, timeout)?;
            if n & wait_irq != 0 {
                break;
            }

            if n & 0x01 != 0 || start_time.elapsed().as_micros() >= 36_000 {
                return Err(PCDErrorCode::Timeout);
            }
        }

        let error_reg_value = self.read_reg(PCDRegister::ErrorReg, timeout)?;
        if error_reg_value & 0x13 != 0 {
            return Err(PCDErrorCode::Error);
        }

        let mut _valid_bits = 0;
        if let Some(back_len) = back_len.as_mut() {
            let n = self.read_reg(PCDRegister::FIFOLevelReg, timeout)?;
            if n > **back_len {
                return Err(PCDErrorCode::NoRoom);
            }

            **back_len = n;
            self.read_reg_buff(
                PCDRegister::FIFODataReg,
                n as usize,
                back_data,
                rx_align,
                timeout,
            )?;

            _valid_bits = self.read_reg(PCDRegister::ControlReg, timeout)? & 0x07;
            if let Some(valid_bits) = valid_bits {
                *valid_bits = _valid_bits;
            }
        }

        if error_reg_value & 0x08 != 0 {
            return Err(PCDErrorCode::Collision);
        }

        if let Some(back_len) = back_len {
            if check_crc {
                if *back_len == 1 && _valid_bits == 4 {
                    return Err(PCDErrorCode::MifareNack);
                }

                if *back_len < 2 || _valid_bits != 0 {
                    return Err(PCDErrorCode::CrcWrong);
                }

                let mut control_buff = [0; 2];
                self.pcd_calc_crc(back_data, *back_len - 2, &mut control_buff, timeout)?;

                if (back_data[*back_len as usize - 2] != control_buff[0])
                    || (back_data[*back_len as usize - 1] != control_buff[1])
                {
                    return Err(PCDErrorCode::CrcWrong);
                }
            }
        }

        Ok(())
    }

    /// Now it prints data to console, TODO: change this
    /// Always returns false (for now)
    pub async fn pcd_selftest(&mut self, timeout: TickType_t) -> Result<bool, PCDErrorCode> {
        log::debug!("Running PCD_Selftest!\n");

        self.write_reg(PCDRegister::FIFOLevelReg, 0x80, timeout)?;
        self.write_reg_buff(PCDRegister::FIFODataReg, 25, &[0; 25], timeout)?;

        self.write_reg(PCDRegister::CommandReg, PCDCommand::Mem, timeout)?;

        self.write_reg(PCDRegister::AutoTestReg, 0x09, timeout)?;
        self.write_reg(PCDRegister::FIFODataReg, 0x00, timeout)?;
        self.write_reg(PCDRegister::CommandReg, PCDCommand::CalcCRC, timeout)?;

        for _ in 0..0xFF {
            let n = self.read_reg(PCDRegister::FIFOLevelReg, timeout)?;
            if n >= 64 {
                break;
            }
        }

        self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

        let mut res = [0; 64];
        self.read_reg_buff(PCDRegister::FIFODataReg, 64, &mut res, 0, timeout)?;

        self.write_reg(PCDRegister::AutoTestReg, 0x00, timeout)?;

        let mut str = String::new();

        for (i, resi) in res.iter().enumerate() {
            if i % 8 == 0 && !str.is_empty() {
                log::debug!("{}", str);
                str.clear();
            }

            _ = core::fmt::write(&mut str, format_args!("{:#04x} ", resi));
        }
        log::debug!("{}", str);

        log::debug!("PCD_Selftest Done!\n");
        self.pcd_init(timeout)?;
        Ok(false)
    }

    pub fn pcd_clear_register_bit_mask(
        &mut self,
        reg: u8,
        mask: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let tmp = self.read_reg(reg, timeout)?;
        self.write_reg(reg, tmp & (!mask), timeout)?;

        Ok(())
    }

    pub fn pcd_set_register_bit_mask(
        &mut self,
        reg: u8,
        mask: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let tmp = self.read_reg(reg, timeout)?;
        self.write_reg(reg, tmp | mask, timeout)?;

        Ok(())
    }

    pub fn pcd_calc_crc(
        &mut self,
        data: &[u8],
        length: u8,
        res: &mut [u8],
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

        self.write_reg(PCDRegister::DivIrqReg, 0x04, timeout)?;
        self.write_reg(PCDRegister::FIFOLevelReg, 0x80, timeout)?;
        self.write_reg_buff(PCDRegister::FIFODataReg, length as usize, data, timeout)?;

        self.write_reg(PCDRegister::CommandReg, PCDCommand::CalcCRC, timeout)?;

        let start_time = std::time::Instant::now();
        while start_time.elapsed().as_micros() < 89_000 {
            let n = self.read_reg(PCDRegister::DivIrqReg, timeout)?;
            if n & 0x04 != 0 {
                self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

                res[0] = self.read_reg(PCDRegister::CRCResultRegL, timeout)?;
                res[1] = self.read_reg(PCDRegister::CRCResultRegH, timeout)?;
                return Ok(());
            }
        }

        Err(PCDErrorCode::Timeout)
    }

    /// This function is to prevent unnesecary clones
    pub fn pcd_calc_crc_single_buf(
        &mut self,
        data: &mut [u8],
        length: u8,
        out_offset: usize,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

        self.write_reg(PCDRegister::DivIrqReg, 0x04, timeout)?;
        self.write_reg(PCDRegister::FIFOLevelReg, 0x80, timeout)?;
        self.write_reg_buff(PCDRegister::FIFODataReg, length as usize, data, timeout)?;

        self.write_reg(PCDRegister::CommandReg, PCDCommand::CalcCRC, timeout)?;

        let start_time = std::time::Instant::now();
        while start_time.elapsed().as_micros() < 89_000 {
            let n = self.read_reg(PCDRegister::DivIrqReg, timeout)?;
            if n & 0x04 != 0 {
                self.write_reg(PCDRegister::CommandReg, PCDCommand::Idle, timeout)?;

                data[out_offset] = self.read_reg(PCDRegister::CRCResultRegL, timeout)?;
                data[out_offset + 1] = self.read_reg(PCDRegister::CRCResultRegH, timeout)?;
                return Ok(());
            }
        }

        Err(PCDErrorCode::Timeout)
    }
}
