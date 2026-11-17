//! Computer -> amplifier command builders. Every function returns a complete,
//! ready-to-send wire frame.

use super::frame::encode;
use super::messages::addr;

/// Command bytes inside a 0x81 "management" message.
mod cmd {
    pub const REQUEST_MESSAGE: u8 = 0x01;
    pub const MODE: u8 = 0x02;
    pub const HARD_FAULT_RECORD: u8 = 0x03;
    pub const USER_FLAGS: u8 = 0x04;
    pub const CAT_SETTINGS_1: u8 = 0x05;
    pub const CAT_SETTINGS_2: u8 = 0x06;
    pub const CAT_SETTINGS_3: u8 = 0x07;
    pub const CLEAR_SOFT_FAULTS: u8 = 0x08;
    pub const BAND_SELECT: u8 = 0x09;
    pub const BUZZER: u8 = 0x0A;
    pub const FACTORY_RESTORE: u8 = 0x0D;
}

/// Target operating state for [`set_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRequest {
    Restart,
    Debug,
    Service,
    Standby,
    OperateRx,
    OperateTx,
    SetupMenu,
    TurnOff,
    ProhibitTx,
    AllowTx,
}

impl ModeRequest {
    fn code(self) -> u8 {
        match self {
            Self::Restart => 0x01,
            Self::Debug => 0x03,
            Self::Service => 0x04,
            Self::Standby => 0x05,
            Self::OperateRx => 0x06,
            Self::OperateTx => 0x07,
            Self::SetupMenu => 0x09,
            Self::TurnOff => 0x0A,
            Self::ProhibitTx => 0x40,
            Self::AllowTx => 0x80,
        }
    }
}

fn management(command: u8, p_hi: u8, p_lo: u8) -> Vec<u8> {
    // buttons byte is obsolete -> always 0
    encode(addr::COMMAND, &[command, p_hi, p_lo, 0])
}

/// Acknowledge a message received from the amplifier (must be sent for every
/// amplifier-initiated message or it is re-sent up to five times).
pub fn ack(received_address: u8) -> Vec<u8> {
    encode(addr::ACK, &[received_address])
}

/// Ask the amplifier to send a specific message once (e.g. `0x11` system info).
pub fn request_message(address: u8) -> Vec<u8> {
    management(cmd::REQUEST_MESSAGE, 0, address)
}

pub fn set_mode(mode: ModeRequest) -> Vec<u8> {
    management(cmd::MODE, 0, mode.code())
}

pub fn request_hard_fault_record(n: u8) -> Vec<u8> {
    management(cmd::HARD_FAULT_RECORD, 0, n)
}

pub fn set_user_flags(flags: u16) -> Vec<u8> {
    management(cmd::USER_FLAGS, (flags >> 8) as u8, flags as u8)
}

pub fn set_cat_settings_1(value: u16) -> Vec<u8> {
    management(cmd::CAT_SETTINGS_1, (value >> 8) as u8, value as u8)
}

pub fn set_cat_byte_spacing_us(value: u16) -> Vec<u8> {
    management(cmd::CAT_SETTINGS_2, (value >> 8) as u8, value as u8)
}

pub fn set_cat_polling_ms(value: u16) -> Vec<u8> {
    management(cmd::CAT_SETTINGS_3, (value >> 8) as u8, value as u8)
}

pub fn clear_soft_faults() -> Vec<u8> {
    management(cmd::CLEAR_SOFT_FAULTS, 0, 0)
}

/// Manual band selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select {
    Band(u8),
    NextBand,
    PrevBand,
}

pub fn select(s: Select) -> Vec<u8> {
    let (arg, sub) = match s {
        Select::Band(n) => (n, 0x8F),
        Select::NextBand => (0, 0x80),
        Select::PrevBand => (0, 0x40),
    };
    management(cmd::BAND_SELECT, arg, sub)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beep {
    None = 0,
    Pulse = 1,
    Warning = 2,
    PowerOff = 3,
    SoftFault = 4,
    HardFault = 5,
    PowerOn = 6,
}

pub fn buzzer(sound: Beep, volume_percent: u8) -> Vec<u8> {
    management(cmd::BUZZER, sound as u8, volume_percent.min(100))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryRestore {
    UserPreferences = 1,
    WorkingHours = 2,
    ErrorLog = 3,
    Everything = 4,
}

pub fn factory_restore(what: FactoryRestore) -> Vec<u8> {
    management(cmd::FACTORY_RESTORE, 0, what as u8)
}

pub fn telemetry_enable(on: bool) -> Vec<u8> {
    encode(if on { addr::TELEMETRY_ON } else { addr::TELEMETRY_OFF }, &[])
}

/// Change which message the amplifier sends cyclically (e.g. 0x2F).
pub fn set_cyclic_message(address: u8) -> Vec<u8> {
    encode(addr::CYCLIC_SET, &[address])
}

/// Push a 30-character user text ("hello message", typically a callsign).
pub fn hello_message(text: &str) -> [Vec<u8>; 2] {
    let mut bytes = [b' '; 30];
    for (i, b) in text.bytes().filter(u8::is_ascii).take(30).enumerate() {
        bytes[i] = b;
    }
    [
        encode(addr::HELLO_SET_1, &bytes[..15]),
        encode(addr::HELLO_SET_2, &bytes[15..]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::frame::Decoder;

    #[test]
    fn commands_are_valid_frames() {
        let all = [
            ack(0x2F),
            request_message(0x11),
            set_mode(ModeRequest::OperateRx),
            select(Select::Band(6)),
            clear_soft_faults(),
            hello_message("N0CALL")[0].clone(),
        ];
        for wire in all {
            let frames = Decoder::new().push(&wire);
            assert_eq!(frames.len(), 1, "{wire:02X?}");
        }
    }

    #[test]
    fn management_layout() {
        // 55 81 08 02 00 07 00 chk  -> set mode OPERATE/TX
        let w = set_mode(ModeRequest::OperateTx);
        assert_eq!(&w[..7], &[0x55, 0x81, 0x08, 0x02, 0x00, 0x07, 0x00]);
        assert_eq!(w.len(), 8);
    }
}
