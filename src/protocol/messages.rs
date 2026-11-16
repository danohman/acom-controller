//! Decoding of amplifier -> computer messages into typed values.

use super::bands::fix_frequency;
use super::frame::Frame;

pub mod addr {
    pub const BAD_MESSAGE: u8 = 0x01;
    pub const REQUEST: u8 = 0x02;
    pub const OK: u8 = 0x03;
    pub const ERR: u8 = 0x04;
    pub const SYSTEM_INFO: u8 = 0x11;
    pub const SETTINGS: u8 = 0x12;
    pub const SERIAL: u8 = 0x13;
    pub const HELLO_1: u8 = 0x14;
    pub const HELLO_2: u8 = 0x15;
    pub const ATU_INFO: u8 = 0x16;
    pub const ANT_ASSIGN: u8 = 0x18;
    pub const SEGMENT: u8 = 0x1A;
    pub const ANT_NAMES: u8 = 0x1B;
    pub const ERROR_CODES: u8 = 0x21;
    pub const BAND_INFO: u8 = 0x27;
    pub const CAT_FROM_AMP: u8 = 0x2C;
    pub const RF_POWERS: u8 = 0x2D;
    pub const ATU_MEASURE: u8 = 0x2E;
    pub const TELEMETRY: u8 = 0x2F;
    pub const HARD_FAULT_1: u8 = 0x31;
    pub const HARD_FAULT_2: u8 = 0x32;
    // computer -> amplifier
    pub const COMMAND: u8 = 0x81;
    pub const HELLO_SET_1: u8 = 0x83;
    pub const HELLO_SET_2: u8 = 0x84;
    pub const ANT_NAMES_SET: u8 = 0x85;
    pub const ACK: u8 = 0x86;
    pub const TELEMETRY_OFF: u8 = 0x91;
    pub const TELEMETRY_ON: u8 = 0x92;
    pub const ANT_ASSIGN_SET: u8 = 0xA0;
    pub const CYCLIC_SET: u8 = 0xAA;
    pub const CAT_TO_AMP: u8 = 0xB0;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AmpMode {
    Reset,
    Init,
    Debug,
    Service,
    #[default]
    Standby,
    OperateRx,
    OperateTx,
    /// ATU tune / antenna change procedure.
    Atac,
    SetParameters,
    TurnOff,
    Unknown(u8),
}

impl AmpMode {
    pub fn from_nibble(n: u8) -> Self {
        match n {
            1 => Self::Reset,
            2 => Self::Init,
            3 => Self::Debug,
            4 => Self::Service,
            5 => Self::Standby,
            6 => Self::OperateRx,
            7 => Self::OperateTx,
            8 => Self::Atac,
            9 => Self::SetParameters,
            0xA => Self::TurnOff,
            x => Self::Unknown(x),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Reset => "RESET",
            Self::Init => "STARTING",
            Self::Debug => "DEBUG",
            Self::Service => "SERVICE",
            Self::Standby => "STANDBY",
            Self::OperateRx => "OPERATE · RX",
            Self::OperateTx => "OPERATE · TX",
            Self::Atac => "ATU TUNING",
            Self::SetParameters => "SETUP MENU",
            Self::TurnOff => "POWERING OFF",
            Self::Unknown(_) => "UNKNOWN",
        }
    }

    pub fn is_operate(self) -> bool {
        matches!(self, Self::OperateRx | Self::OperateTx)
    }
}

/// Sub-state of the ATAC (ATU tune) procedure, from the mode byte 0x8X.
pub fn atac_text(mode_byte: u8) -> Option<&'static str> {
    if mode_byte >> 4 != 8 {
        return None;
    }
    Some(match mode_byte & 0x0F {
        0 => "Apply transceiver power to tune",
        1 => "Apply transceiver power (CAT)",
        2 => "Full tune in progress",
        3 => "Remove drive power (CAT)",
        4 => "Remove drive power",
        5 => "Tuned: excellent (SWR < 1.3)",
        6 => "Tuned: good (SWR < 1.6)",
        7 => "Tuned: bad (SWR ≥ 1.6)",
        8 => "Full tune failed",
        9 => "Tune interrupted by operator",
        _ => "ATU procedure",
    })
}

/// The 72-byte cyclic status message (0x2F): the primary telemetry source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Telemetry {
    pub mode_byte: u8,
    pub mode: AmpMode,
    pub flags1: u16,
    pub flags2: u16,
    /// Seconds since amplifier start.
    pub clock_s: u32,
    pub temp1_k: u16,
    pub temp2_k: u16,
    pub input_w: f32,
    pub forward_w: f32,
    pub reflected_w: f32,
    pub swr: f32,
    pub pam1_dissipation_w: f32,
    pub pam2_dissipation_w: f32,
    pub band_data_mv: u16,
    pub vcc5_v: f32,
    pub vcc26_v: f32,
    pub hv1_v: f32,
    pub hv2_v: f32,
    /// PAM1 drain current in amperes.
    pub id1_a: f32,
    pub id2_a: f32,
    pub frequency_khz: u32,
    pub bias_mv: [u16; 8],
    pub alarm_code: u8,
    pub alarm_param: u16,
    /// 0..=4
    pub fan_speed: u8,
    /// Active low-pass filter channel (0 = none).
    pub lpf_channel: u8,
    pub sys_flags: u8,
}

impl Telemetry {
    pub fn temp1_c(&self) -> f32 {
        self.temp1_k as f32 - 273.15
    }
    pub fn temp2_c(&self) -> f32 {
        self.temp2_k as f32 - 273.15
    }
    pub fn tx_access_enabled(&self) -> bool {
        self.flags2 & (1 << 4) != 0
    }
    pub fn output_relay(&self) -> bool {
        self.flags2 & (1 << 6) != 0
    }
    pub fn input_relay(&self) -> bool {
        self.flags2 & (1 << 3) != 0
    }
    pub fn key_in(&self) -> bool {
        self.flags2 & (1 << 1) != 0
    }
    pub fn hv_on(&self) -> bool {
        self.flags1 & (1 << 12) != 0
    }
    pub fn bias_on(&self) -> bool {
        self.flags1 & (1 << 9) != 0
    }
    pub fn lpf_tuned(&self) -> bool {
        self.flags2 & (1 << 9) != 0
    }
    pub fn last_command_remote(&self) -> bool {
        self.flags1 & (1 << 15) != 0
    }
    pub fn default_antenna(&self) -> bool {
        self.sys_flags & (1 << 3) != 0
    }
    pub fn atu_assigned(&self) -> bool {
        self.sys_flags & (1 << 1) != 0
    }
    /// Efficiency estimate: forward power vs. PAM dissipation.
    pub fn dissipation_w(&self) -> f32 {
        self.pam1_dissipation_w + self.pam2_dissipation_w
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AmpModel {
    #[default]
    Acom600S,
    Acom1200S,
    Acom700S,
    Acom600SNewLpf,
    Acom2020S,
    Acom500S,
    Acom1400S,
    Unknown(u8),
}

impl AmpModel {
    pub const ALL: [AmpModel; 7] = [
        Self::Acom600S,
        Self::Acom1200S,
        Self::Acom700S,
        Self::Acom600SNewLpf,
        Self::Acom2020S,
        Self::Acom500S,
        Self::Acom1400S,
    ];

    pub fn from_code(c: u8) -> Self {
        match c {
            1 => Self::Acom600S,
            2 => Self::Acom1200S,
            3 => Self::Acom700S,
            4 => Self::Acom600SNewLpf,
            5 => Self::Acom2020S,
            6 => Self::Acom500S,
            7 => Self::Acom1400S,
            x => Self::Unknown(x),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Acom600S => "ACOM 600S",
            Self::Acom1200S => "ACOM 1200S",
            Self::Acom700S => "ACOM 700S",
            Self::Acom600SNewLpf => "ACOM 600S (new LPFs)",
            Self::Acom2020S => "ACOM 2020S",
            Self::Acom500S => "ACOM 500S",
            Self::Acom1400S => "ACOM 1400S",
            Self::Unknown(_) => "Unknown model",
        }
    }

    /// Highest band number the model supports.
    pub fn max_band(self) -> u8 {
        if self == Self::Acom500S { 12 } else { 11 }
    }

    /// Nominal full-scale forward power for meter ranges, in watts.
    pub fn full_scale_w(self) -> f32 {
        match self {
            Self::Acom600S | Self::Acom600SNewLpf | Self::Acom700S => 700.0,
            Self::Acom500S => 600.0,
            Self::Acom1200S | Self::Acom1400S => 1500.0,
            Self::Acom2020S => 2200.0,
            Self::Unknown(_) => 1500.0,
        }
    }

    /// The 2020S reports PAM1 current in units of 10 mA.
    pub fn current_scale(self) -> f32 {
        if self == Self::Acom2020S { 0.01 } else { 0.001 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemInfo {
    pub model: AmpModel,
    pub firmware: String,
    pub cpu_pcb: String,
    pub interface_a: String,
    pub interface_b: String,
    pub modem: String,
    pub bootloader: String,
    pub serial: String,
    pub hard_fault_count: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub cat_baud: u8,
    pub cat_interface: u8,
    pub cat_command_set: u8,
    pub cat_byte_spacing_us: u16,
    pub cat_polling_ms: u16,
    pub user_flags: u16,
    pub buzzer_volume: u8,
}

impl Settings {
    pub fn extended_protocol(&self) -> bool {
        self.user_flags & (1 << 7) != 0
    }
    pub fn fahrenheit(&self) -> bool {
        self.user_flags & (1 << 2) != 0
    }
    pub fn auto_operate(&self) -> bool {
        self.user_flags & (1 << 1) != 0
    }
    pub fn extra_cooling(&self) -> bool {
        self.user_flags & 1 != 0
    }
    pub fn buzzer(&self) -> bool {
        self.user_flags & (1 << 3) != 0
    }
    pub fn operate_access(&self) -> bool {
        self.user_flags & (1 << 6) != 0
    }
    pub fn atac_auto(&self) -> bool {
        self.user_flags & (1 << 4) != 0
    }
    pub fn on_button_short(&self) -> bool {
        self.user_flags & (1 << 5) != 0
    }
}

pub const BAUD_TABLE: [(u8, &str); 7] = [
    (0, "None"),
    (1, "1200"),
    (2, "4800"),
    (3, "9600"),
    (4, "19200"),
    (5, "38400"),
    (6, "57600"),
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BandInfo {
    pub antenna: u8,
    pub antenna_type: u8,
    pub antenna_is_default: bool,
    pub band: u8,
    pub segment_count: u8,
    pub segment: u8,
    pub low_khz: u32,
    pub high_khz: u32,
    /// Per-segment tuned flags (up to 56 in the extended message).
    pub segments_tuned: Vec<bool>,
    pub atu_label: u8,
}

pub fn atu_label_text(n: u8) -> &'static str {
    match n {
        1 => "ATU initialising",
        2 => "Recalling settings",
        3 => "Matching cycle…",
        4 => "ATU memory transfer",
        5 => "Completed",
        6 => "ATU in bypass",
        7 => "ATU matched",
        8 => "Coarse ATU match",
        9 => "No match found",
        10 => "Settings outdated",
        11 => "Memory settings",
        _ => "",
    }
}

pub fn antenna_type_text(t: u8) -> &'static str {
    match t & 0x7F {
        0 => "Sharp",
        1 => "Regular",
        2 => "Wide",
        3 => "Fixed",
        _ => "?",
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AtuMeasure {
    pub temp1_k: u16,
    pub temp2_k: u16,
    pub input_w: f32,
    pub forward_w: f32,
    pub reflected_w: f32,
    pub atu_supply_v: f32,
    pub antenna_v: f32,
    pub antenna_a: f32,
    pub antenna_fwd_w: f32,
    pub antenna_refl_w: f32,
    pub atu_swr: f32,
    pub atu_temp_k: u16,
    pub atu_hours: u16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardFaultRecord {
    pub total_count: u8,
    pub index: u8,
    pub error_words: [u16; 10],
    pub mode: u16,
    pub clock_s: u32,
}

/// Operating conditions captured with a hard fault (record part 2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardFaultSnapshot {
    pub index: u8,
    pub frequency_khz: u32,
    pub forward_w: u16,
    pub reflected_w: u16,
    pub input_w: f32,
    pub vcc26_v: f32,
    pub vcc5_v: f32,
    pub hv1_v: f32,
    pub id1_raw: u16,
    pub temp1_k: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    BadMessage,
    Ok(u16),
    Err(u16),
    SystemInfo(SystemInfo),
    Settings(Settings),
    Serial(String),
    Hello { part: u8, text: String },
    AtuInfo { atu_type: u8, hw: String, fw: String, serial: String, tune_validity_h: Option<u16> },
    AntennaNames([String; 4]),
    ErrorCodes(Vec<u8>),
    BandInfo(BandInfo),
    RfPowers { input_w: f32, forward_w: f32, reflected_w: f32, mode_byte: u8 },
    AtuMeasure(AtuMeasure),
    Telemetry(Telemetry),
    HardFault(HardFaultRecord),
    HardFaultSnapshot(HardFaultSnapshot),
    CatRequest { id: u8, value: u32 },
    Unknown { address: u8, len: usize },
}

fn u16le(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}

fn ver(d: &[u8], i: usize) -> String {
    format!("{}.{}", d[i], d[i + 1])
}

fn ascii(d: &[u8]) -> String {
    d.iter()
        .take_while(|&&b| b != 0)
        .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '?' })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Decode one validated frame. Frames that are too short for their address are
/// reported as [`Message::Unknown`] rather than panicking.
pub fn decode(f: &Frame) -> Message {
    let d = f.data.as_slice();
    let unknown = || Message::Unknown { address: f.address, len: d.len() + 4 };
    match f.address {
        addr::BAD_MESSAGE => Message::BadMessage,
        addr::OK if d.len() >= 2 => Message::Ok(u16::from_be_bytes([d[0], d[1]])),
        addr::ERR if d.len() >= 2 => Message::Err(u16::from_be_bytes([d[0], d[1]])),
        addr::SYSTEM_INFO if d.len() >= 26 => Message::SystemInfo(SystemInfo {
            model: AmpModel::from_code(d[0]),
            firmware: ver(d, 1),
            cpu_pcb: ver(d, 3),
            interface_a: ver(d, 5),
            interface_b: ver(d, 7),
            modem: ver(d, 9),
            bootloader: ver(d, 11),
            serial: ascii(&d[13..25]),
            hard_fault_count: d[25],
        }),
        addr::SETTINGS if d.len() >= 9 => Message::Settings(Settings {
            cat_baud: d[0] >> 4,
            cat_command_set: d[1] & 0x0F,
            cat_interface: d[1] >> 4,
            cat_byte_spacing_us: u16le(d, 2),
            cat_polling_ms: u16le(d, 4),
            user_flags: u16le(d, 6),
            buzzer_volume: d[8],
        }),
        addr::SERIAL if d.len() >= 12 => Message::Serial(ascii(&d[..12])),
        addr::HELLO_1 if d.len() >= 15 => Message::Hello { part: 0, text: ascii(&d[..15]) },
        addr::HELLO_2 if d.len() >= 15 => Message::Hello { part: 1, text: ascii(&d[..15]) },
        addr::ATU_INFO if d.len() >= 15 => Message::AtuInfo {
            atu_type: d[0],
            hw: ver(d, 1),
            fw: ver(d, 3),
            serial: ascii(&d[5..15]),
            tune_validity_h: (d.len() >= 17).then(|| u16::from_be_bytes([d[15], d[16]])),
        },
        addr::ANT_NAMES if d.len() >= 48 => Message::AntennaNames([
            ascii(&d[0..12]),
            ascii(&d[12..24]),
            ascii(&d[24..36]),
            ascii(&d[36..48]),
        ]),
        addr::ERROR_CODES if d.len() >= 22 => {
            let words: Vec<u16> = (0..11).map(|i| u16le(d, i * 2)).collect();
            Message::ErrorCodes(super::errors::active_codes(&words))
        }
        addr::BAND_INFO if d.len() >= 17 => {
            // 7 bytes of per-segment "tuned" flags cover up to 56 segments.
            let seg_stat: Vec<bool> = d[9..16]
                .iter()
                .flat_map(|b| (0..8).map(move |i| b & (1 << i) != 0))
                .collect();
            let (low, high) = (fix_frequency(u16le(d, 5)), fix_frequency(u16le(d, 7)));
            Message::BandInfo(BandInfo {
                antenna: d[0],
                antenna_type: d[1] & 0x7F,
                antenna_is_default: d[1] & 0x80 != 0,
                band: d[2],
                segment_count: d[3],
                segment: d[4],
                low_khz: low,
                high_khz: high,
                segments_tuned: seg_stat,
                atu_label: d[16],
            })
        }
        addr::RF_POWERS if d.len() >= 7 => Message::RfPowers {
            input_w: u16le(d, 0) as f32 / 10.0,
            forward_w: u16le(d, 2) as f32,
            reflected_w: u16le(d, 4) as f32,
            mode_byte: d[6],
        },
        addr::ATU_MEASURE if d.len() >= 27 => Message::AtuMeasure(AtuMeasure {
            temp1_k: u16le(d, 0),
            temp2_k: u16le(d, 2),
            input_w: u16le(d, 4) as f32 / 10.0,
            forward_w: u16le(d, 6) as f32,
            reflected_w: u16le(d, 8) as f32,
            atu_supply_v: u16le(d, 10) as f32 / 10.0,
            antenna_v: u16le(d, 12) as f32,
            antenna_a: u16le(d, 14) as f32 / 10.0,
            antenna_fwd_w: u16le(d, 16) as f32,
            antenna_refl_w: u16le(d, 18) as f32,
            atu_swr: u16le(d, 20) as f32 / 100.0,
            atu_temp_k: u16le(d, 22),
            atu_hours: u16le(d, 24),
        }),
        addr::TELEMETRY if d.len() >= 68 => Message::Telemetry(decode_telemetry(d)),
        addr::HARD_FAULT_1 if d.len() >= 32 => {
            let mut words = [0u16; 10];
            for (i, w) in words.iter_mut().enumerate() {
                *w = u16le(d, 2 + i * 2);
            }
            Message::HardFault(HardFaultRecord {
                total_count: d[0],
                index: d[1],
                error_words: words,
                mode: u16le(d, 24),
                clock_s: ((u16le(d, 28) as u32) << 16) | u16le(d, 30) as u32,
            })
        }
        addr::HARD_FAULT_2 if d.len() >= 32 => Message::HardFaultSnapshot(HardFaultSnapshot {
            index: d[1],
            frequency_khz: fix_frequency(u16le(d, 2)),
            forward_w: u16le(d, 4),
            reflected_w: u16le(d, 6),
            input_w: u16le(d, 8) as f32 / 10.0,
            vcc26_v: u16le(d, 12) as f32 / 10.0,
            vcc5_v: u16le(d, 14) as f32 / 1000.0,
            hv1_v: u16le(d, 16) as f32 / 10.0,
            id1_raw: u16le(d, 20),
            temp1_k: u16le(d, 24),
        }),
        addr::CAT_FROM_AMP if d.len() >= 5 => Message::CatRequest {
            id: d[0],
            value: u32::from_be_bytes([d[1], d[2], d[3], d[4]]),
        },
        _ => unknown(),
    }
}

/// `d` is the payload (frame bytes 3..N-1), so payload offset = frame byte - 3.
fn decode_telemetry(d: &[u8]) -> Telemetry {
    let mode_byte = d[0];
    let mut bias = [0u16; 8];
    for (i, b) in bias.iter_mut().enumerate() {
        *b = u16le(d, 47 + i * 2);
    }
    Telemetry {
        mode_byte,
        mode: AmpMode::from_nibble(mode_byte >> 4),
        flags1: u16le(d, 1),
        flags2: u16le(d, 3),
        clock_s: ((u16le(d, 9) as u32) << 16) | u16le(d, 11) as u32,
        temp1_k: u16le(d, 13),
        temp2_k: u16le(d, 15),
        input_w: u16le(d, 17) as f32 / 10.0,
        forward_w: u16le(d, 19) as f32,
        reflected_w: u16le(d, 21) as f32,
        swr: u16le(d, 23) as f32 / 100.0,
        pam1_dissipation_w: u16le(d, 25) as f32 / 10.0,
        pam2_dissipation_w: u16le(d, 27) as f32 / 10.0,
        band_data_mv: u16le(d, 29),
        vcc5_v: u16le(d, 33) as f32 / 1000.0,
        vcc26_v: u16le(d, 35) as f32 / 10.0,
        hv1_v: u16le(d, 37) as f32 / 10.0,
        hv2_v: u16le(d, 39) as f32 / 10.0,
        // Raw current; the model-specific scale is applied by the consumer.
        id1_a: u16le(d, 41) as f32,
        id2_a: u16le(d, 43) as f32,
        frequency_khz: fix_frequency(u16le(d, 45)),
        bias_mv: bias,
        alarm_code: d[63],
        alarm_param: u16le(d, 64),
        fan_speed: d[66] >> 4,
        lpf_channel: d[66] & 0x0F,
        sys_flags: d[67],
    }
}

/// Serialise a [`Telemetry`] back into a payload. Used by the built-in amplifier
/// simulator and by tests to prove decode/encode symmetry.
pub fn encode_telemetry(t: &Telemetry) -> Vec<u8> {
    let mut d = vec![0u8; 68];
    let mut put = |i: usize, v: u16| {
        d[i..i + 2].copy_from_slice(&v.to_le_bytes());
    };
    // Written through a closure borrowing `d`; do the byte fields afterwards.
    put(1, t.flags1);
    put(3, t.flags2);
    put(9, (t.clock_s >> 16) as u16);
    put(11, t.clock_s as u16);
    put(13, t.temp1_k);
    put(15, t.temp2_k);
    put(17, (t.input_w * 10.0).round() as u16);
    put(19, t.forward_w.round() as u16);
    put(21, t.reflected_w.round() as u16);
    put(23, (t.swr * 100.0).round() as u16);
    put(25, (t.pam1_dissipation_w * 10.0).round() as u16);
    put(27, (t.pam2_dissipation_w * 10.0).round() as u16);
    put(29, t.band_data_mv);
    put(33, (t.vcc5_v * 1000.0).round() as u16);
    put(35, (t.vcc26_v * 10.0).round() as u16);
    put(37, (t.hv1_v * 10.0).round() as u16);
    put(39, (t.hv2_v * 10.0).round() as u16);
    put(41, t.id1_a as u16);
    put(43, t.id2_a as u16);
    let wire_freq = if t.frequency_khz > 54005 { t.frequency_khz - 10000 } else { t.frequency_khz };
    put(45, wire_freq as u16);
    for (i, b) in t.bias_mv.iter().enumerate() {
        put(47 + i * 2, *b);
    }
    put(64, t.alarm_param);
    d[0] = t.mode_byte;
    d[63] = t.alarm_code;
    d[66] = (t.fan_speed << 4) | (t.lpf_channel & 0x0F);
    d[67] = t.sys_flags;
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::frame::{Decoder, encode};

    #[test]
    fn telemetry_roundtrip() {
        let t = Telemetry {
            mode_byte: 0x70,
            mode: AmpMode::OperateTx,
            flags1: 0x1200,
            flags2: 0x0258,
            clock_s: 70_000,
            temp1_k: 320,
            temp2_k: 0,
            input_w: 12.3,
            forward_w: 612.0,
            reflected_w: 14.0,
            swr: 1.35,
            pam1_dissipation_w: 150.5,
            pam2_dissipation_w: 0.0,
            band_data_mv: 1200,
            vcc5_v: 5.02,
            vcc26_v: 26.1,
            hv1_v: 49.8,
            hv2_v: 0.0,
            id1_a: 24000.0,
            id2_a: 0.0,
            frequency_khz: 14074,
            bias_mv: [1, 2, 3, 4, 5, 6, 7, 8],
            alarm_code: 0x8E,
            alarm_param: 7,
            fan_speed: 2,
            lpf_channel: 6,
            sys_flags: 0x0A,
        };
        let wire = encode(addr::TELEMETRY, &encode_telemetry(&t));
        assert_eq!(wire.len(), 72, "0x2F is 72 bytes per spec");
        let frames = Decoder::new().push(&wire);
        let Message::Telemetry(back) = decode(&frames[0]) else { panic!("not telemetry") };
        assert_eq!(back.mode, AmpMode::OperateTx);
        assert_eq!(back.frequency_khz, 14074);
        assert!((back.swr - 1.35).abs() < 0.001);
        assert_eq!(back.forward_w, 612.0);
        assert_eq!(back.bias_mv, t.bias_mv);
        assert_eq!(back.fan_speed, 2);
        assert_eq!(back.lpf_channel, 6);
        assert!(back.hv_on());
        assert_eq!(back.clock_s, 70_000);
    }

    #[test]
    fn short_frames_do_not_panic() {
        for a in 0..=255u8 {
            let f = Frame { address: a, data: vec![1, 2] };
            let _ = decode(&f);
        }
    }

    #[test]
    fn hard_fault_parts_decode() {
        // Part 1: error words 0..9, mode, clock (38-byte frame -> 34-byte payload).
        let mut d = vec![0u8; 34];
        d[0] = 37;
        d[1] = 5;
        d[2 + 2 * 3] = 0x04; // word 3 bit 2 -> code 0x32
        d[24] = 0x30; // mode low
        d[28..30].copy_from_slice(&1u16.to_le_bytes()); // clock high word
        d[30..32].copy_from_slice(&2u16.to_le_bytes()); // clock low word
        let frames = Decoder::new().push(&encode(addr::HARD_FAULT_1, &d));
        let Message::HardFault(r) = decode(&frames[0]) else { panic!("not hard fault") };
        assert_eq!((r.total_count, r.index), (37, 5));
        assert_eq!(super::super::errors::active_codes(&r.error_words), vec![0x32]);
        assert_eq!(r.clock_s, 0x0001_0002);

        // Part 2: operating snapshot.
        let mut d = vec![0u8; 34];
        d[1] = 5;
        d[2..4].copy_from_slice(&14074u16.to_le_bytes());
        d[4..6].copy_from_slice(&600u16.to_le_bytes());
        d[6..8].copy_from_slice(&90u16.to_le_bytes());
        d[8..10].copy_from_slice(&123u16.to_le_bytes());
        d[12..14].copy_from_slice(&261u16.to_le_bytes());
        d[14..16].copy_from_slice(&5020u16.to_le_bytes());
        d[16..18].copy_from_slice(&498u16.to_le_bytes());
        d[24..26].copy_from_slice(&320u16.to_le_bytes());
        let frames = Decoder::new().push(&encode(addr::HARD_FAULT_2, &d));
        let Message::HardFaultSnapshot(x) = decode(&frames[0]) else { panic!("not snapshot") };
        assert_eq!((x.index, x.frequency_khz, x.forward_w, x.reflected_w), (5, 14074, 600, 90));
        assert!((x.input_w - 12.3).abs() < 0.01 && (x.vcc26_v - 26.1).abs() < 0.01);
        assert!((x.vcc5_v - 5.02).abs() < 0.001 && (x.hv1_v - 49.8).abs() < 0.01);
        assert_eq!(x.temp1_k, 320);
    }
}
