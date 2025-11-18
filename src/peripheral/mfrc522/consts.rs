// FROM: https://github.com/OSSLibraries/Arduino_MFRC522v2/blob/master/src/MFRC522Constants.h

use esp_idf_svc::sys::EspError;

#[derive(Debug, Clone)]
pub struct Uid {
    pub size: u8,
    pub uid_bytes: [u8; 10],
    pub sak: u8,
}

impl Uid {
    pub fn get_number(&self) -> u128 {
        match self.size {
            4 => u32::from_le_bytes(self.uid_bytes[..4].try_into().unwrap_or([0, 0, 0, 0])) as u128,
            7 => {
                let mut bytes = [0; 8];
                bytes[..7].copy_from_slice(&self.uid_bytes[..7]);
                u64::from_le_bytes(bytes) as u128
            }
            10 => {
                let mut bytes = [0; 16];
                bytes[..10].copy_from_slice(&self.uid_bytes[..10]);
                u128::from_le_bytes(bytes)
            }
            _ => {
                log::error!("Wrong bytes count!");
                unreachable!() // i dont think that this case is reachable
            }
        }
    }
}

pub struct PCDRegister;
pub struct PCDCommand;
pub struct PICCCommand;

#[allow(dead_code, non_upper_case_globals)]
impl PCDRegister {
    // Page 0: Command and status
    //              0x00                    // reserved for future use
    pub const CommandReg: u8 = 0x01; // starts and stops command execution
    pub const ComIEnReg: u8 = 0x02; // enable and disable interrupt request control bits
    pub const DivIEnReg: u8 = 0x03; // enable and disable interrupt request control bits
    pub const ComIrqReg: u8 = 0x04; // interrupt request bits
    pub const DivIrqReg: u8 = 0x05; // interrupt request bits
    pub const ErrorReg: u8 = 0x06; // error bits showing the error status of the last command executed
    pub const Status1Reg: u8 = 0x07; // communication status bits
    pub const Status2Reg: u8 = 0x08; // receiver and transmitter status bits
    pub const FIFODataReg: u8 = 0x09; // input and output of 64 byte FIFO buffer
    pub const FIFOLevelReg: u8 = 0x0A; // number of bytes stored in the FIFO buffer
    pub const WaterLevelReg: u8 = 0x0B; // level for FIFO underflow and overflow warning
    pub const ControlReg: u8 = 0x0C; // miscellaneous control registers
    pub const BitFramingReg: u8 = 0x0D; // adjustments for bit-oriented frames
    pub const CollReg: u8 = 0x0E; // bit position of the first bit-collision detected on the RF interface
                                  //              0x0F     // reserved for future use

    // Page 1: Command
    //               0x10     // reserved for future use
    pub const ModeReg: u8 = 0x11; // defines general modes for transmitting and receiving
    pub const TxModeReg: u8 = 0x12; // defines transmission data rate and framing
    pub const RxModeReg: u8 = 0x13; // defines reception data rate and framing
    pub const TxControlReg: u8 = 0x14; // controls the logical behavior of the antenna driver pins TX1 and TX2
    pub const TxASKReg: u8 = 0x15; // controls the setting of the transmission modulation
    pub const TxSelReg: u8 = 0x16; // selects the internal sources for the antenna driver
    pub const RxSelReg: u8 = 0x17; // selects internal receiver settings
    pub const RxThresholdReg: u8 = 0x18; // selects thresholds for the bit decoder
    pub const DemodReg: u8 = 0x19; // defines demodulator settings
                                   //               0x1A     // reserved for future use
                                   //               0x1B     // reserved for future use
    pub const MfTxReg: u8 = 0x1C; // controls some MIFARE communication transmit parameters
    pub const MfRxReg: u8 = 0x1D; // controls some MIFARE communication receive parameters
                                  //               0x1E     // reserved for future use
    pub const SerialSpeedReg: u8 = 0x1F; // selects the speed of the serial UART interface

    // Page 2: Configuration
    //               0x20        // reserved for future use
    pub const CRCResultRegH: u8 = 0x21; // shows the MSB and LSB values of the CRC calculation
    pub const CRCResultRegL: u8 = 0x22;
    //               0x23        // reserved for future use
    pub const ModWidthReg: u8 = 0x24; // controls the ModWidth setting?
                                      //               0x25        // reserved for future use
    pub const RFCfgReg: u8 = 0x26; // configures the receiver gain
    pub const GsNReg: u8 = 0x27; // selects the conductance of the antenna driver pins TX1 and TX2 for modulation
    pub const CWGsPReg: u8 = 0x28; // defines the conductance of the p-driver output during periods of no modulation
    pub const ModGsPReg: u8 = 0x29; // defines the conductance of the p-driver output during periods of modulation
    pub const TModeReg: u8 = 0x2A; // defines settings for the internal timer
    pub const TPrescalerReg: u8 = 0x2B; // the lower 8 bits of the TPrescaler value. The 4 high bits are in TModeReg.
    pub const TReloadRegH: u8 = 0x2C; // defines the 16-bit timer reload value
    pub const TReloadRegL: u8 = 0x2D;
    pub const TCounterValueRegH: u8 = 0x2E; // shows the 16-bit timer value
    pub const TCounterValueRegL: u8 = 0x2F;

    // Page 3: Test Registers
    //               0x30      // reserved for future use
    pub const TestSel1Reg: u8 = 0x31; // general test signal configuration
    pub const TestSel2Reg: u8 = 0x32; // general test signal configuration
    pub const TestPinEnReg: u8 = 0x33; // enables pin output driver on pins D1 to D7
    pub const TestPinValueReg: u8 = 0x34; // defines the values for D1 to D7 when it is used as an I/O bus
    pub const TestBusReg: u8 = 0x35; // shows the status of the internal test bus
    pub const AutoTestReg: u8 = 0x36; // controls the digital self-test
    pub const VersionReg: u8 = 0x37; // shows the software version
    pub const AnalogTestReg: u8 = 0x38; // controls the pins AUX1 and AUX2
    pub const TestDAC1Reg: u8 = 0x39; // defines the test value for TestDAC1
    pub const TestDAC2Reg: u8 = 0x3A; // defines the test value for TestDAC2
    pub const TestADCReg: u8 = 0x3B; // shows the value of ADC I and Q channels
                                     //               0x3C      // reserved for production tests
                                     //               0x3D      // reserved for production tests
                                     //               0x3E      // reserved for production tests
                                     //               0x3F      // reserved for production tests
}

#[allow(dead_code, non_upper_case_globals)]
impl PCDCommand {
    pub const Idle: u8 = 0x00; // no action, cancels current command execution
    pub const Mem: u8 = 0x01; // stores 25 bytes into the internal buffer
    pub const GenerateRandomID: u8 = 0x02; // generates a 10-byte random ID number
    pub const CalcCRC: u8 = 0x03; // activates the CRC coprocessor or performs a self-test
    pub const Transmit: u8 = 0x04; // transmits data from the FIFO buffer
    pub const NoCmdChange: u8 = 0x07; // no command change, can be used to modify the CommandReg register bits without affecting the command, for example, the PowerDown bit
    pub const Receive: u8 = 0x08; // activates the receiver circuits
    pub const Transceive: u8 = 0x0C; // transmits data from FIFO buffer to antenna and automatically activates the receiver after transmission
    pub const MFAuthent: u8 = 0x0E; // performs the MIFARE standard authentication as a reader
    pub const SoftReset: u8 = 0x0F; // resets the MFRC522
}

#[allow(dead_code, non_upper_case_globals)]
impl PICCCommand {
    pub const PICC_CMD_REQA: u8 = 0x26; // REQuest command, Type A. Invites PICCs in state IDLE to go to READY and prepare for anticollision or selection. 7 bit frame.
    pub const PICC_CMD_WUPA: u8 = 0x52; // Wake-UP command, Type A. Invites PICCs in state IDLE and HALT to go to READY(*) and prepare for anticollision or selection. 7 bit frame.
    pub const PICC_CMD_CT: u8 = 0x88; // Cascade Tag. Not really a command, but used during anti collision.
    pub const PICC_CMD_SEL_CL1: u8 = 0x93; // Anti collision/Select, Cascade Level 1
    pub const PICC_CMD_SEL_CL2: u8 = 0x95; // Anti collision/Select, Cascade Level 2
    pub const PICC_CMD_SEL_CL3: u8 = 0x97; // Anti collision/Select, Cascade Level 3
    pub const PICC_CMD_HLTA: u8 = 0x50; // HaLT command, Type A. Instructs an ACTIVE PICC to go to state HALT.
    pub const PICC_CMD_RATS: u8 = 0xE0; // Request command for Answer To Reset.
                                        // The commands used for MIFARE Classic (from http://www.mouser.com/ds/2/302/MF1S503x-89574.pdf, Section 9)
                                        // Use PCD_MFAuthent to authenticate access to a sector, then use these commands to read/write/modify the blocks on the sector.
                                        // The read/write commands can also be used for MIFARE Ultralight.
    pub const PICC_CMD_MF_AUTH_KEY_A: u8 = 0x60; // Perform authentication with Key A
    pub const PICC_CMD_MF_AUTH_KEY_B: u8 = 0x61; // Perform authentication with Key B
    pub const PICC_CMD_MF_READ: u8 = 0x30; // Reads one 16 byte block from the authenticated sector of the PICC. Also used for MIFARE Ultralight.
    pub const PICC_CMD_MF_WRITE: u8 = 0xA0; // Writes one 16 byte block to the authenticated sector of the PICC. Called "COMPATIBILITY WRITE" for MIFARE Ultralight.
    pub const PICC_CMD_MF_DECREMENT: u8 = 0xC0; // Decrements the contents of a block and stores the result in the internal data register.
    pub const PICC_CMD_MF_INCREMENT: u8 = 0xC1; // Increments the contents of a block and stores the result in the internal data register.
    pub const PICC_CMD_MF_RESTORE: u8 = 0xC2; // Reads the contents of a block into the internal data register.
    pub const PICC_CMD_MF_TRANSFER: u8 = 0xB0; // Writes the contents of the internal data register to a block.
                                               // The commands used for MIFARE Ultralight (from http://www.nxp.com/documents/data_sheet/MF0ICU1.pdf, Section 8.6)
                                               // The PICC_CMD_MF_READ and PICC_CMD_MF_WRITE can also be used for MIFARE Ultralight.
    pub const PICC_CMD_UL_WRITE: u8 = 0xA2; // Writes one 4 byte page to the PICC.
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum PCDVersion {
    Counterfeit = 0x12,
    FM17522 = 0x88,
    FM17522_1 = 0xb2,
    FM17522E = 0x89,
    Version0_0 = 0x90,
    Version1_0 = 0x91,
    Version2_0 = 0x92,
    VersionUnknown = 0xff,
}

#[allow(dead_code)]
impl PCDVersion {
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x12 => PCDVersion::Counterfeit,
            0x88 => PCDVersion::FM17522,
            0xb2 => PCDVersion::FM17522_1,
            0x89 => PCDVersion::FM17522E,
            0x90 => PCDVersion::Version0_0,
            0x91 => PCDVersion::Version1_0,
            0x92 => PCDVersion::Version2_0,
            _ => PCDVersion::VersionUnknown,
        }
    }
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum PICCType {
    PiccTypeUnknown = 0xff,
    PiccTypeIso14443_4 = 0x20, // PICC compliant with ISO/IEC 14443-4.
    PiccTypeIso18092 = 0x40,   // PICC compliant with ISO/IEC 18092 (NFC).
    PiccTypeMifareMini = 0x09, // MIFARE Classic protocol, 320 bytes.
    PiccTypeMifare1K = 0x08,   // MIFARE Classic protocol, 1KB.
    PiccTypeMifare4K = 0x18,   // MIFARE Classic protocol, 4KB.
    PiccTypeMifareUL = 0x00,   // MIFARE Ultralight or Ultralight C.
    PiccTypeMifarePlus = 0x10 | 0x11, // MIFARE Plus.
    PiccTypeMifareDesfire,     // MIFARE DESFire.
    PiccTypeTnp3XXX = 0x01, // Only mentioned in NXP AN 10833 MIFARE Type Identification Procedure.
    PiccTypeNotComplete = 0x04, // SAK indicates UID is not complete.
}

impl PICCType {
    pub fn from_sak(sak: u8) -> Self {
        let sak = sak & 0x7F;

        match sak {
            0x20 => PICCType::PiccTypeIso14443_4,
            0x40 => PICCType::PiccTypeIso18092,
            0x09 => PICCType::PiccTypeMifareMini,
            0x08 => PICCType::PiccTypeMifare1K,
            0x18 => PICCType::PiccTypeMifare4K,
            0x00 => PICCType::PiccTypeMifareUL,
            0x10 | 0x11 => PICCType::PiccTypeMifarePlus,
            0x01 => PICCType::PiccTypeTnp3XXX,
            0x04 => PICCType::PiccTypeNotComplete,
            _ => PICCType::PiccTypeUnknown,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PCDErrorCode {
    /// Error in communication
    Error,

    /// Collision detected
    Collision,

    /// Timeout in communication
    Timeout,

    /// A buffer is not big enough
    NoRoom,

    /// Internal error in code.
    InternalError,

    /// Invalid argument
    Invalid,

    /// The CRC_A does not match
    CrcWrong,

    /// Unspecified error
    Unknown,

    /// MIFARE PICC responded with NAK
    MifareNack,

    /// SPI error
    SpiError(EspError),

    /// I2C error
    I2cError(EspError),

    /// Any driver error
    DriverError,
}

impl PCDErrorCode {
    pub fn from_spi_error(error: EspError) -> Self {
        PCDErrorCode::SpiError(error)
    }

    pub fn from_i2c_error(error: EspError) -> Self {
        PCDErrorCode::I2cError(error)
    }
}

pub enum UidSize {
    Four,
    Seven,
    Ten,
}

impl UidSize {
    pub fn to_byte(&self) -> u8 {
        match self {
            Self::Four => 4,
            Self::Seven => 7,
            Self::Ten => 10,
        }
    }
}
