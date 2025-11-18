use esp_idf_svc::sys::TickType_t;

use super::{
    consts::{PCDErrorCode, PCDRegister, PICCCommand, Uid},
    tif, MfrcDriver, MFRC522,
};

impl<D> MFRC522<D>
where
    D: MfrcDriver,
{
    pub fn picc_is_new_card_present(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        let mut buffer_atqa = [0; 2];
        let mut buffer_size = 2;

        self.write_reg(PCDRegister::TxModeReg, 0x00, timeout)?;
        self.write_reg(PCDRegister::RxModeReg, 0x00, timeout)?;
        self.write_reg(PCDRegister::ModWidthReg, 0x26, timeout)?;

        self.picc_request_a(&mut buffer_atqa, &mut buffer_size, timeout)?;

        Ok(())
    }

    pub fn picc_halta(&mut self, timeout: TickType_t) -> Result<(), PCDErrorCode> {
        let mut buff = [0; 4];
        buff[0] = PICCCommand::PICC_CMD_HLTA;
        buff[1] = 0;

        self.pcd_calc_crc_single_buf(&mut buff, 2, 2, timeout)?;
        let res = self.pcd_transceive_data(&buff, 4, &mut [], None, None, 0, false, timeout);

        // yes error timeout here is only Ok here
        match res {
            Ok(_) => Err(PCDErrorCode::Error),
            Err(PCDErrorCode::Timeout) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn picc_select(
        &mut self,
        uid: &mut Uid,
        valid_bits: u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        let mut uid_complete = false;
        let mut use_casdcade_tag;
        let mut cascade_level = 1u8;
        let mut count: u8;
        let mut check_bit: u8;
        let mut index: u8;
        let mut uid_index: u8;
        let mut current_level_known_bits: i8;
        let mut buff = [0; 9];
        let mut buffer_used: u8;
        let mut rx_align: u8;
        let mut tx_last_bits = 0u8;
        let mut response_buff_ptr = 0;
        let mut response_length = 0u8;

        if valid_bits > 80 {
            return Err(PCDErrorCode::Invalid);
        }

        self.pcd_clear_register_bit_mask(PCDRegister::CollReg, 0x80, timeout)?;

        while !uid_complete {
            match cascade_level {
                1 => {
                    buff[0] = PICCCommand::PICC_CMD_SEL_CL1;
                    uid_index = 0;
                    use_casdcade_tag = valid_bits != 0 && (uid.size > 4);
                }
                2 => {
                    buff[0] = PICCCommand::PICC_CMD_SEL_CL2;
                    uid_index = 3;
                    use_casdcade_tag = valid_bits != 0 && (uid.size > 7);
                }
                3 => {
                    buff[0] = PICCCommand::PICC_CMD_SEL_CL3;
                    uid_index = 6;
                    use_casdcade_tag = false;
                }
                _ => {
                    return Err(PCDErrorCode::InternalError);
                }
            }

            current_level_known_bits = valid_bits as i8 - (8i8 * uid_index as i8);
            if current_level_known_bits < 0 {
                current_level_known_bits = 0;
            }

            index = 2;
            if use_casdcade_tag {
                buff[index as usize] = PICCCommand::PICC_CMD_CT;
                index += 1;
            }

            let mut bytes_to_copy =
                current_level_known_bits / 8 + tif(current_level_known_bits % 8 != 0, 1, 0);

            if bytes_to_copy != 0 {
                let max_bytes = if use_casdcade_tag { 3 } else { 4 };
                if bytes_to_copy > max_bytes {
                    bytes_to_copy = max_bytes;
                }

                for count in 0..bytes_to_copy as usize {
                    buff[index as usize] = uid.uid_bytes[uid_index as usize + count];
                    index += 1;
                }
            }

            if use_casdcade_tag {
                current_level_known_bits += 8;
            }

            let mut select_done = false;
            while !select_done {
                if current_level_known_bits >= 32 {
                    buff[1] = 0x70;
                    buff[6] = buff[2] ^ buff[3] ^ buff[4] ^ buff[5];

                    self.pcd_calc_crc_single_buf(&mut buff, 7, 7, timeout)?;

                    tx_last_bits = 0;
                    buffer_used = 9;
                    response_buff_ptr = 6;
                    response_length = 3;
                } else {
                    tx_last_bits = (current_level_known_bits % 8) as u8;
                    count = (current_level_known_bits / 8) as u8;
                    index = 2 + count;
                    buff[1] = (index << 4) + tx_last_bits;
                    buffer_used = index + tif(tx_last_bits != 0, 1, 0);

                    response_length = 9 - index;
                    response_buff_ptr = index;
                }

                rx_align = tx_last_bits;
                self.write_reg(
                    PCDRegister::BitFramingReg,
                    (rx_align << 4) + tx_last_bits,
                    timeout,
                )?;

                let res = self.pcd_transceive_data(
                    &buff.clone(),
                    buffer_used,
                    &mut buff[response_buff_ptr as usize..],
                    Some(&mut response_length),
                    Some(&mut tx_last_bits),
                    rx_align,
                    false,
                    timeout,
                );

                match res {
                    Ok(_) => {
                        if current_level_known_bits >= 32 {
                            select_done = true;
                        } else {
                            current_level_known_bits = 32;
                        }
                    }
                    Err(PCDErrorCode::Collision) => {
                        let value_of_coll_reg = self.read_reg(PCDRegister::CollReg, timeout)?;
                        if value_of_coll_reg & 0x20 != 0 {
                            return Err(PCDErrorCode::Collision);
                        }

                        let mut collision_pos = value_of_coll_reg & 0x1F;
                        if collision_pos == 0 {
                            collision_pos = 32;
                        }

                        if collision_pos as i8 <= current_level_known_bits {
                            return Err(PCDErrorCode::InternalError);
                        }

                        current_level_known_bits = collision_pos as i8;
                        count = (current_level_known_bits % 8) as u8;
                        check_bit = ((current_level_known_bits - 1) % 8) as u8;
                        index = 1 + (current_level_known_bits / 8) as u8 + tif(count != 0, 1, 0);

                        buff[index as usize] |= 1 << check_bit;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            index = tif(buff[2] == PICCCommand::PICC_CMD_CT, 3, 2);
            let bytes_to_copy = tif(buff[2] == PICCCommand::PICC_CMD_CT, 3, 4);

            for i in 0..bytes_to_copy {
                uid.uid_bytes[uid_index as usize + i] = buff[index as usize];
                index += 1;
            }

            if response_length != 3 || tx_last_bits != 0 {
                return Err(PCDErrorCode::Error);
            }

            self.pcd_calc_crc(
                &[buff[response_buff_ptr as usize]],
                1,
                &mut buff[2..],
                timeout,
            )?;

            if (buff[2] != buff[response_buff_ptr as usize + 1])
                || (buff[3] != buff[response_buff_ptr as usize + 2])
            {
                return Err(PCDErrorCode::CrcWrong);
            }

            if buff[response_buff_ptr as usize] & 0x04 != 0 {
                cascade_level += 1;
            } else {
                uid_complete = true;
                uid.sak = buff[response_buff_ptr as usize];
            }
        }

        Ok(())
    }

    pub fn picc_wakeup_a(
        &mut self,
        buffer_atqa: &mut [u8],
        buffer_size: &mut u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.picc_reqa_or_wupa(
            PICCCommand::PICC_CMD_WUPA,
            buffer_atqa,
            buffer_size,
            timeout,
        )
    }

    pub fn picc_request_a(
        &mut self,
        buffer_atqa: &mut [u8],
        buffer_size: &mut u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        self.picc_reqa_or_wupa(
            PICCCommand::PICC_CMD_REQA,
            buffer_atqa,
            buffer_size,
            timeout,
        )
    }

    pub fn picc_reqa_or_wupa(
        &mut self,
        cmd: u8,
        buffer_atqa: &mut [u8],
        buffer_size: &mut u8,
        timeout: TickType_t,
    ) -> Result<(), PCDErrorCode> {
        if *buffer_size < 2 {
            return Err(PCDErrorCode::NoRoom);
        }

        self.pcd_clear_register_bit_mask(PCDRegister::CollReg, 0x80, timeout)?;

        let mut valid_bits = 7;
        self.pcd_transceive_data(
            &[cmd],
            1,
            buffer_atqa,
            Some(buffer_size),
            Some(&mut valid_bits),
            0,
            false,
            timeout,
        )?;

        if *buffer_size != 2 || valid_bits != 0 {
            return Err(PCDErrorCode::Error);
        }

        Ok(())
    }
}
