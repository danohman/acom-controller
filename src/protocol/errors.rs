//! Alarm / status code catalogue (message 0x21 bit map, message 0x2F code byte).
//!
//! The wire code is `word * 16 + bit` where `word` is Error Codes 0..=A.
//! Severity is a presentation-level classification derived from the wording,
//! it is not transmitted by the amplifier.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Informational message (e.g. ATU tuning progress).
    Info,
    Warning,
    SoftFault,
    HardFault,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::SoftFault => "SOFT FAULT",
            Severity::HardFault => "HARD FAULT",
        }
    }
}

fn bias_name(i: u8) -> String {
    let pam = if i < 4 { 1 } else { 2 };
    let ch = (b'A' + (i % 4)) as char;
    format!("BIAS_{pam}{ch}")
}

/// Human readable text for an alarm code. `None` for unassigned bits.
pub fn text(code: u8) -> Option<String> {
    let s: String = match code {
        0x00 | 0x08 => "HOT SWITCHING ATTEMPT".into(),
        0x01 => "OUTPUT RELAY CLOSED, IT SHOULD BE OPEN".into(),
        0x02 => "OUTPUT RELAY OPEN, IT SHOULD BE CLOSED".into(),
        0x03 => "DRIVE POWER DETECTED AT WRONG TIME".into(),
        0x04 => "REFLECTED POWER WARNING".into(),
        0x05 => "EXCESSIVE REFLECTED POWER".into(),
        0x06 => "DRIVE POWER TOO HIGH".into(),
        0x07 => "EXCESSIVE DRIVE POWER".into(),
        0x09 => "DRIVE FREQUENCY OUT OF RANGE".into(),
        0x0A => "FREQUENCY VIOLATION".into(),
        0x0B => "OUTPUT DISBALANCE".into(),
        0x0C => "RF POWER DETECTED AT WRONG TIME".into(),
        0x0D => "PA LOAD SWR TOO HIGH".into(),
        0x0E => "STOP TRANSMISSION FIRST".into(),
        0x0F => "REMOVE DRIVE POWER IMMEDIATELY".into(),
        0x10 => "5 V TOO LOW".into(),
        0x11 => "5 V TOO HIGH".into(),
        0x12 => "26 V TOO LOW".into(),
        0x13 => "26 V TOO HIGH".into(),
        0x14 => "TIMEOUT EXPIRED: INCOMPLETE PROCESS".into(),
        0x15 => "PAM1 FAN SPEED TOO LOW".into(),
        0x16 => "PAM2 FAN SPEED TOO LOW".into(),
        0x17 => "LPF FAN SPEED TOO LOW".into(),
        0x18 => "PAM1 DISSIPATION POWER TOO HIGH".into(),
        0x19 => "PAM2 DISSIPATION POWER TOO HIGH".into(),
        0x1A => "PAM1 DISSIPATION POWER WARNING".into(),
        0x1B => "PAM2 DISSIPATION POWER WARNING".into(),
        0x1C => "PAM1 TEMPERATURE TOO HIGH".into(),
        0x1D => "PAM2 TEMPERATURE TOO HIGH".into(),
        0x1E => "PAM1 EXCESSIVE TEMPERATURE".into(),
        0x1F => "PAM2 EXCESSIVE TEMPERATURE".into(),
        // PAM1 block (0x20..=0x3F) and PAM2 block (0x40..=0x5F) mirror each other.
        0x20..=0x5F => {
            let pam = if code < 0x40 { 1 } else { 2 };
            let o = code & 0x1F;
            match o {
                0x00 => format!("PAM{pam} HV TOO LOW"),
                0x01 => format!("PAM{pam} HV TOO HIGH"),
                0x02 => format!("PAM{pam} CURRENT NOT ZERO WHEN IT SHOULD BE"),
                0x03 => format!("PAM{pam} IDLE CURRENT TOO LOW"),
                0x04 => format!("PAM{pam} CURRENT WARNING"),
                0x05 => format!("PAM{pam} EXCESSIVE CURRENT"),
                0x06..=0x09 => {
                    format!("{} VOLTAGE ERROR", bias_name((o - 6) + (pam - 1) * 4))
                }
                0x0A..=0x0D => format!(
                    "{} NOT ZERO WHEN IT SHOULD BE",
                    bias_name((o - 0x0A) + (pam - 1) * 4)
                ),
                0x0E => format!("PAM{pam} GAIN TOO LOW"),
                0x0F => format!("PAM{pam} GAIN TOO HIGH"),
                0x10 => format!("PAM{pam} HV NOT ZERO WHEN IT SHOULD BE"),
                0x11 => format!("PAM{pam} CURRENT NOT ZERO WHEN IT SHOULD BE"),
                0x12 => format!("PAM{pam} EXCESSIVE TEMPERATURE"),
                0x13 => format!("PAM{pam} TEMPERATURE TOO HIGH"),
                0x14..=0x17 => format!(
                    "{} NOT ZERO WHEN IT SHOULD BE",
                    bias_name((o - 0x14) + (pam - 1) * 4)
                ),
                0x18 => format!("PSU{pam} EXCESSIVE TEMPERATURE"),
                0x19 => format!("PAM{pam} EXCESSIVE CURRENT: CHECK SWR OR REDUCE DRIVE POWER"),
                _ => return None,
            }
        }
        0x60 => "PSU1/CONTROL MALFUNCTION".into(),
        0x61 => "PSU2/CONTROL MALFUNCTION".into(),
        0x62 => "PSU1 EXCESSIVE TEMPERATURE".into(),
        0x63 => "PSU2 EXCESSIVE TEMPERATURE".into(),
        0x64 => "DISPLAY UNIT COMMUNICATION ERROR".into(),
        0x65 => "MODEM EXCESSIVE TEMPERATURE".into(),
        0x66 => "ALARM BY DC POWER SWITCH TO ATU/ASEL".into(),
        0x67 => "COMMUNICATION TO ATU/ASEL FAILING".into(),
        0x68 => "ATU/ASEL NOT RESPONDING".into(),
        0x69 => "TUNING COMPLETED: STOP DRIVE POWER".into(),
        0x6F => "LOSS OF EEPROM DATA".into(),
        0x70 => "CAT ERROR".into(),
        0x71 => "ATU/ASEL UNASSIGNED AUTOMATICALLY AND OUT OF SERVICE".into(),
        0x72 => "ATU WILL BE IN BYPASS / ANT. 1 UNTIL ANTENNA AND FREQUENCY ARE SELECTED".into(),
        0x73 => "MODEM INITIALIZATION FAULT".into(),
        0x80 => "UNEXPECTED ATU RESPONSE".into(),
        0x81 => "A1 SELECTED BY DEFAULT - USE ANTENNA ASSIGNMENTS MENU TO CHANGE".into(),
        0x82 => "ATU TUNING OUTDATED - YOU MAY REFRESH IT".into(),
        0x83 => "FREQUENCY CHANGED DURING TUNE".into(),
        0x84 => "NO MATCH: CHECK THE ANTENNA SWR".into(),
        0x85 => "ATU MEMORY WRITE ERROR - CHECK BATTERY PLEASE".into(),
        0x86 => "ATU FULL TUNING REQUIRED".into(),
        0x87 => "RF POWER DETECTED. INCOMPLETE ATU/ASEL COMMUTATION".into(),
        0x88 => "FIXED-FREQ. MEMORY FULL: ERASE OBSOLETE RECORDS".into(),
        0x89 => "RF POWER DETECTED AT WRONG TIME: ATU DAMAGE RISK".into(),
        0x8A => "TUNING RF POWER TOO LOW - TRY AGAIN".into(),
        0x8B => "TUNING RF POWER UNSTABLE - TRY AGAIN".into(),
        0x8C => "TUNING RF POWER TOO HIGH: ATU DAMAGE RISK".into(),
        0x8D => "TUNING IN PROGRESS".into(),
        0x8E => "TUNED: SWR < 1.3".into(),
        0x8F => "TUNED: SWR < 1.6".into(),
        0x90..=0x97 => format!(
            "{} OUT OF WINDOW. IF OK SELECT OFF",
            bias_name(code - 0x90)
        ),
        0x98..=0x9F => format!("{} SHOULD BE BELOW HIGH THRESHOLD", bias_name(code - 0x98)),
        0xA0 => "ATU/ASEL DC-INPUT VOLTAGE TOO HIGH".into(),
        0xA1 => "ATU/ASEL DC-INPUT VOLTAGE TOO LOW".into(),
        0xA2 => "ATU/ASEL INTERNAL +5 V TOO LOW".into(),
        0xA3 => "ATU/ASEL INTERNAL +5 V TOO HIGH".into(),
        0xA4 => "ANTENNA VOLTAGE TOO HIGH: REDUCE POWER".into(),
        0xA5 => "ANTENNA VOLTAGE TOO HIGH: ATU/ASEL IS OFF DUE TO DAMAGE RISK".into(),
        0xA6 => "ANTENNA CURRENT TOO HIGH: REDUCE POWER".into(),
        0xA7 => "ANTENNA CURRENT TOO HIGH: ATU/ASEL IS OFF DUE TO DAMAGE RISK".into(),
        0xA8 => "ANTENNA REFLECTED POWER TOO HIGH: ATU/ASEL DAMAGE RISK".into(),
        0xA9 => "ANTENNA REFLECTED POWER TOO HIGH: ATU/ASEL IS OFF DUE TO DAMAGE RISK".into(),
        0xAA => "ATU/ASEL RF-INPUT POWER TOO HIGH: DAMAGE RISK".into(),
        0xAB => "ATU/ASEL RF-INPUT POWER TOO HIGH: TURNED OFF DUE TO DAMAGE RISK".into(),
        0xAC => "ANTENNA SWR TOO HIGH: ATU/ASEL DAMAGE RISK".into(),
        0xAD => "ANTENNA SWR TOO HIGH: ATU/ASEL DAMAGE RISK (WARNING)".into(),
        0xAE => "ATU/ASEL TEMPERATURE TOO HIGH: TURNED OFF DUE TO DAMAGE RISK".into(),
        0xAF => "ATU/ASEL TEMPERATURE TOO LOW: TURNED OFF DUE TO DAMAGE RISK".into(),
        _ => return None,
    };
    Some(s)
}

/// Classify a code for display purposes.
pub fn severity(code: u8) -> Severity {
    let t = match text(code) {
        Some(t) => t,
        None => return Severity::Info,
    };
    // ATU progress / result messages.
    if matches!(code, 0x69 | 0x72 | 0x81 | 0x8D | 0x8E | 0x8F) {
        return Severity::Info;
    }
    if t.contains("WARNING") || t.contains("OUTDATED") || t.contains("OUT OF WINDOW")
        || t.contains("BELOW HIGH THRESHOLD") || t.contains("TRY AGAIN")
        || t.contains("FREQUENCY CHANGED") || t.contains("CAT ERROR")
        || t.contains("REDUCE POWER") || t.contains("MEMORY WRITE")
        || t.contains("MEMORY FULL") || t.contains("NO MATCH")
    {
        return Severity::Warning;
    }
    if t.contains("NOT ZERO") || t.contains("TIMEOUT") || t.contains("FAN SPEED")
        || t.contains("EXCESSIVE REFLECTED") || t.contains("EXCESSIVE TEMPERATURE")
        || t.contains("MALFUNCTION") || t.contains("TURNED OFF") || t.contains("IS OFF")
        || t.contains("UNEXPECTED") || t.contains("INITIALIZATION FAULT")
        || t.contains("DC POWER SWITCH") || t.contains("INTERNAL +5") || t.contains("DC-INPUT")
        || (t.contains("EXCESSIVE CURRENT") && !t.contains("CHECK SWR"))
    {
        return Severity::HardFault;
    }
    Severity::SoftFault
}

/// Expand the eleven 16-bit "Error Codes 0..A" words into the list of active codes.
pub fn active_codes(words: &[u16]) -> Vec<u8> {
    let mut v = Vec::new();
    for (w, word) in words.iter().enumerate().take(11) {
        for bit in 0..16u8 {
            if word & (1 << bit) != 0 {
                v.push((w as u8) * 16 + bit);
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_bits() {
        let mut w = [0u16; 11];
        w[0] = 1 << 5; // EXCESSIVE REFLECTED POWER
        w[1] = 1 << 0x0C; // word 1, bit 12 = code 0x1C
        assert_eq!(active_codes(&w), vec![0x05, 0x1C]);
        assert_eq!(text(0x1C).unwrap(), "PAM1 TEMPERATURE TOO HIGH");
        assert_eq!(severity(0x05), Severity::HardFault);
    }

    #[test]
    fn bias_naming() {
        assert_eq!(text(0x26).unwrap(), "BIAS_1A VOLTAGE ERROR");
        assert_eq!(text(0x47).unwrap(), "BIAS_2B VOLTAGE ERROR");
        assert_eq!(text(0x4D).unwrap(), "BIAS_2D NOT ZERO WHEN IT SHOULD BE");
    }
}
