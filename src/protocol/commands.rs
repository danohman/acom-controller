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
}

/// Target operating state for [`set_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRequest {
    Standby,
    OperateRx,
    TurnOff,
    ProhibitTx,
    AllowTx,
}

impl ModeRequest {
    fn code(self) -> u8 {
        match self {
            Self::Standby => 0x05,
            Self::OperateRx => 0x06,
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

/// Manually select band `n` (1-based, as on the wire).
pub fn select_band(n: u8) -> Vec<u8> {
    management(cmd::BAND_SELECT, n, 0x8F)
}

/// Make the amplifier beep once at `volume_percent` (used to preview the buzzer volume).
pub fn buzzer_pulse(volume_percent: u8) -> Vec<u8> {
    management(cmd::BUZZER, 1, volume_percent.min(100))
}

pub fn telemetry_enable(on: bool) -> Vec<u8> {
    encode(if on { addr::TELEMETRY_ON } else { addr::TELEMETRY_OFF }, &[])
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
            select_band(6),
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
        // 55 81 08 02 00 06 00 chk  -> set mode OPERATE/RX
        let w = set_mode(ModeRequest::OperateRx);
        assert_eq!(&w[..7], &[0x55, 0x81, 0x08, 0x02, 0x00, 0x06, 0x00]);
        assert_eq!(w.len(), 8);
    }
}
