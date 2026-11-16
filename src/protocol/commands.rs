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
    pub const BAND_ANTENNA_SEGMENT: u8 = 0x09;
    pub const BUZZER: u8 = 0x0A;
    pub const FACTORY_RESTORE: u8 = 0x0D;
    pub const ATU: u8 = 0x0E;
    pub const ATU_SERVICE: u8 = 0x10;
    pub const ATU_VALIDITY: u8 = 0x11;
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

/// Manual band / antenna / segment selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select {
    Band(u8),
    NextBand,
    PrevBand,
    Antenna(u8),
    NextAntenna,
    Segment(u8),
    NextSegment,
    PrevSegment,
}

pub fn select(s: Select) -> Vec<u8> {
    let (arg, sub) = match s {
        Select::Band(n) => (n, 0x8F),
        Select::NextBand => (0, 0x80),
        Select::PrevBand => (0, 0x40),
        Select::Antenna(n) => (n, 0x3F),
        Select::NextAntenna => (0, 0x30),
        Select::Segment(n) => (n, 0x2F),
        Select::NextSegment => (0, 0x20),
        Select::PrevSegment => (0, 0x10),
    };
    management(cmd::BAND_ANTENNA_SEGMENT, arg, sub)
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
    AntennaAssignments = 5,
}

pub fn factory_restore(what: FactoryRestore) -> Vec<u8> {
    management(cmd::FACTORY_RESTORE, 0, what as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtuCommand {
    FullTune,
    Bypass(u8),
    StopTune,
    /// 0x00 = unassigned, 0x21 = ATU assigned.
    Assign(bool),
}

pub fn atu(c: AtuCommand) -> Vec<u8> {
    let (arg, sub) = match c {
        AtuCommand::FullTune => (0, 0x01),
        AtuCommand::Bypass(ant) => (ant, 0x02),
        AtuCommand::StopTune => (0, 0x03),
        AtuCommand::Assign(on) => (if on { 0x21 } else { 0x00 }, 0xFF),
    };
    management(cmd::ATU, arg, sub)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtuService {
    EraseCurrentTuning,
    EraseAntennaTunings(u8),
    EraseAllTunings,
    EraseWholeMemory,
    Exit,
}

pub fn atu_service(s: AtuService) -> Vec<u8> {
    let (kind, ant) = match s {
        AtuService::EraseCurrentTuning => (0x01, 0),
        AtuService::EraseAntennaTunings(a) => (0x02, a),
        AtuService::EraseAllTunings => (0x04, 0),
        AtuService::EraseWholeMemory => (0x08, 0),
        AtuService::Exit => (0xFF, 0),
    };
    management(cmd::ATU_SERVICE, kind, ant)
}

pub fn set_atu_validity_hours(hours: u16) -> Vec<u8> {
    management(cmd::ATU_VALIDITY, (hours >> 8) as u8, hours as u8)
}

pub fn telemetry_enable(on: bool) -> Vec<u8> {
    encode(if on { addr::TELEMETRY_ON } else { addr::TELEMETRY_OFF }, &[])
}

/// Change which message the amplifier sends cyclically (e.g. 0x2F).
pub fn set_cyclic_message(address: u8) -> Vec<u8> {
    encode(addr::CYCLIC_SET, &[address])
}

/// Write the four antenna-type / band-mask assignments.
pub fn antenna_assignments(types: [u8; 4], band_masks: [u16; 4]) -> Vec<u8> {
    let mut d = Vec::with_capacity(12);
    d.extend_from_slice(&types.map(|t| t & 0x07));
    for m in band_masks {
        d.push((m >> 8) as u8 & 0x07);
        d.push(m as u8);
    }
    encode(addr::ANT_ASSIGN_SET, &d)
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

/// Write 4 antenna display names (12 characters each).
pub fn antenna_names(names: &[String; 4]) -> Vec<u8> {
    let mut d = Vec::with_capacity(48);
    for n in names {
        let mut b = [b' '; 12];
        for (i, c) in n.bytes().filter(u8::is_ascii).take(12).enumerate() {
            b[i] = c;
        }
        d.extend_from_slice(&b);
    }
    encode(addr::ANT_NAMES_SET, &d)
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
            atu(AtuCommand::FullTune),
            clear_soft_faults(),
            hello_message("N0CALL")[0].clone(),
            antenna_names(&["A".into(), "B".into(), "C".into(), "D".into()]),
            antenna_assignments([1, 1, 2, 3], [0x7FF, 0, 1, 2]),
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
