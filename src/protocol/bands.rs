//! Band plan used by the amplifier.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// Band number as used on the wire (1-based).
    pub number: u8,
    pub name: &'static str,
    pub low_khz: u32,
    pub high_khz: u32,
}

/// Bands 1..=11 are common to every model; band 12 (4 m) exists on the 500S.
pub const BANDS: [Band; 12] = [
    Band { number: 1, name: "160 m", low_khz: 1800, high_khz: 2000 },
    Band { number: 2, name: "80 m", low_khz: 3500, high_khz: 4000 },
    Band { number: 3, name: "60 m", low_khz: 5060, high_khz: 5450 },
    Band { number: 4, name: "40 m", low_khz: 7000, high_khz: 7300 },
    Band { number: 5, name: "30 m", low_khz: 10100, high_khz: 10150 },
    Band { number: 6, name: "20 m", low_khz: 14000, high_khz: 14350 },
    Band { number: 7, name: "17 m", low_khz: 18068, high_khz: 18168 },
    Band { number: 8, name: "15 m", low_khz: 21000, high_khz: 21450 },
    Band { number: 9, name: "12 m", low_khz: 24890, high_khz: 24990 },
    Band { number: 10, name: "10 m", low_khz: 28000, high_khz: 29700 },
    Band { number: 11, name: "6 m", low_khz: 50000, high_khz: 54000 },
    Band { number: 12, name: "4 m", low_khz: 69900, high_khz: 70500 },
];

pub fn band_by_number(n: u8) -> Option<&'static Band> {
    BANDS.iter().find(|b| b.number == n)
}

/// Find the band containing `khz`, restricted to the bands a model supports.
pub fn band_for_frequency(khz: u32, max_band: u8) -> Option<&'static Band> {
    BANDS
        .iter()
        .filter(|b| b.number <= max_band)
        .find(|b| khz >= b.low_khz && khz <= b.high_khz)
}

/// The spec transmits 4 m frequencies 10 MHz low so they fit in 16 bits;
/// any raw value above 54005 kHz must have 10000 kHz added back.
pub fn fix_frequency(raw: u16) -> u32 {
    let raw = raw as u32;
    if raw > 54005 { raw + 10000 } else { raw }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_frequencies() {
        assert_eq!(band_for_frequency(14074, 11).unwrap().name, "20 m");
        assert_eq!(band_for_frequency(7100, 11).unwrap().number, 4);
        assert!(band_for_frequency(70200, 11).is_none());
        assert_eq!(band_for_frequency(70200, 12).unwrap().name, "4 m");
        assert!(band_for_frequency(12000, 12).is_none());
    }

    #[test]
    fn four_meter_fixup() {
        assert_eq!(fix_frequency(14074), 14074);
        assert_eq!(fix_frequency(60200), 70200);
    }
}
