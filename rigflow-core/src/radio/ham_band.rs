//! Amateur (ham) HF band definitions, band detection, and N2ADR filter mapping.
//!
//! Shared by the client (band buttons, band detection display) and the server
//! (N2ADR filter programming for the Hermes Lite 2).  No 6m / transverter /
//! editable-table support by design.
//!
//! ## N2ADR filter values
//!
//! The per-band filter values match Quisk's HL2 `Hermes_BandDict`
//! (`quisk_conf_defaults.py`): the 7-bit value selects the J16 filter outputs.
//! Quisk transmits it as the address-0 C&C byte `C2 = value << 1` (value in
//! C2[7:1]); see [`crate::radio::ham_band`] consumers.  The value here is the
//! raw 7-bit number (NOT pre-shifted, NOT bit-reversed).

use serde::{Deserialize, Serialize};

use crate::dsp::modes::DemodMode;

/// A supported amateur HF band (160m–10m, no 6m).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HamBand {
    B160,
    B80,
    B60,
    B40,
    B30,
    B20,
    B17,
    B15,
    B12,
    B10,
}

impl HamBand {
    /// All bands in display order (160m → 10m).
    pub const ALL: [HamBand; 10] = [
        HamBand::B160,
        HamBand::B80,
        HamBand::B60,
        HamBand::B40,
        HamBand::B30,
        HamBand::B20,
        HamBand::B17,
        HamBand::B15,
        HamBand::B12,
        HamBand::B10,
    ];

    /// Human-readable label, e.g. "20m".
    pub fn label(self) -> &'static str {
        match self {
            HamBand::B160 => "160m",
            HamBand::B80 => "80m",
            HamBand::B60 => "60m",
            HamBand::B40 => "40m",
            HamBand::B30 => "30m",
            HamBand::B20 => "20m",
            HamBand::B17 => "17m",
            HamBand::B15 => "15m",
            HamBand::B12 => "12m",
            HamBand::B10 => "10m",
        }
    }

    /// Inclusive RF frequency range (Hz) that identifies this band.
    pub fn range_hz(self) -> (u64, u64) {
        match self {
            HamBand::B160 => (1_800_000, 2_000_000),
            HamBand::B80 => (3_500_000, 4_000_000),
            HamBand::B60 => (5_330_500, 5_405_000),
            HamBand::B40 => (7_000_000, 7_300_000),
            HamBand::B30 => (10_100_000, 10_150_000),
            HamBand::B20 => (14_000_000, 14_350_000),
            HamBand::B17 => (18_068_000, 18_168_000),
            HamBand::B15 => (21_000_000, 21_450_000),
            HamBand::B12 => (24_890_000, 24_990_000),
            HamBand::B10 => (28_000_000, 29_700_000),
        }
    }

    /// Sensible default operating frequency (Hz) for the band button.
    pub fn default_frequency_hz(self) -> u64 {
        match self {
            HamBand::B160 => 1_900_000,
            HamBand::B80 => 3_900_000,
            HamBand::B60 => 5_357_000,
            HamBand::B40 => 7_200_000,
            HamBand::B30 => 10_120_000,
            HamBand::B20 => 14_200_000,
            HamBand::B17 => 18_130_000,
            HamBand::B15 => 21_300_000,
            HamBand::B12 => 24_950_000,
            HamBand::B10 => 28_400_000,
        }
    }

    /// Conventional default demod mode (USB above ~10 MHz, LSB below).
    pub fn default_mode(self) -> DemodMode {
        match self {
            HamBand::B160 | HamBand::B80 | HamBand::B40 => DemodMode::Lsb,
            HamBand::B60
            | HamBand::B30
            | HamBand::B20
            | HamBand::B17
            | HamBand::B15
            | HamBand::B12
            | HamBand::B10 => DemodMode::Usb,
        }
    }

    /// N2ADR filter board value (7-bit) for this band, matching Quisk's HL2
    /// `Hermes_BandDict`.  Transmit as `C2 = value << 1` on the address-0 frame.
    pub fn n2adr_filter_value(self) -> u8 {
        match self {
            HamBand::B160 => 1, // 0b0000001
            HamBand::B80 => 66, // 0b1000010
            HamBand::B60 => 68, // 0b1000100 (shares the 40m filter, per Quisk)
            HamBand::B40 => 68, // 0b1000100
            HamBand::B30 => 72, // 0b1001000
            HamBand::B20 => 72, // 0b1001000
            HamBand::B17 => 80, // 0b1010000
            HamBand::B15 => 80, // 0b1010000
            HamBand::B12 => 96, // 0b1100000
            HamBand::B10 => 96, // 0b1100000
        }
    }
}

/// Detect the band containing `freq_hz`, or `None` if outside all supported
/// bands (caller should then leave any N2ADR filter unchanged).
pub fn band_from_frequency(freq_hz: u64) -> Option<HamBand> {
    HamBand::ALL.into_iter().find(|band| {
        let (lo, hi) = band.range_hz();
        freq_hz >= lo && freq_hz <= hi
    })
}

/// Default operating frequency (Hz) for a band button.
pub fn default_frequency_for_band(band: HamBand) -> u64 {
    band.default_frequency_hz()
}

/// Default demod mode for a band button.
pub fn default_mode_for_band(band: HamBand) -> DemodMode {
    band.default_mode()
}

/// N2ADR 7-bit filter value for a band (transmit as `value << 1` in C2).
pub fn n2adr_filter_value_for_band(band: HamBand) -> u8 {
    band.n2adr_filter_value()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_detection_inside_and_outside() {
        assert_eq!(band_from_frequency(14_200_000), Some(HamBand::B20));
        assert_eq!(band_from_frequency(7_200_000), Some(HamBand::B40));
        assert_eq!(band_from_frequency(1_900_000), Some(HamBand::B160));
        assert_eq!(band_from_frequency(28_400_000), Some(HamBand::B10));
        // Edges (inclusive).
        assert_eq!(band_from_frequency(14_000_000), Some(HamBand::B20));
        assert_eq!(band_from_frequency(14_350_000), Some(HamBand::B20));
        // Gaps between bands → None.
        assert_eq!(band_from_frequency(5_000_000), None); // between 80m and 60m
        assert_eq!(band_from_frequency(30_000_000), None); // above 10m
        assert_eq!(band_from_frequency(100_000), None); // below 160m
    }

    #[test]
    fn each_band_default_freq_is_inside_its_range() {
        for band in HamBand::ALL {
            assert_eq!(
                band_from_frequency(band.default_frequency_hz()),
                Some(band),
                "default freq for {} not inside its own range",
                band.label()
            );
        }
    }

    #[test]
    fn default_modes_match_table() {
        use DemodMode::{Lsb, Usb};
        assert_eq!(default_mode_for_band(HamBand::B160), Lsb);
        assert_eq!(default_mode_for_band(HamBand::B80), Lsb);
        assert_eq!(default_mode_for_band(HamBand::B40), Lsb);
        assert_eq!(default_mode_for_band(HamBand::B60), Usb);
        assert_eq!(default_mode_for_band(HamBand::B30), Usb);
        assert_eq!(default_mode_for_band(HamBand::B20), Usb);
        assert_eq!(default_mode_for_band(HamBand::B10), Usb);
    }

    #[test]
    fn n2adr_values_match_quisk() {
        // Quisk HL2 Hermes_BandDict (quisk_conf_defaults.py).
        assert_eq!(n2adr_filter_value_for_band(HamBand::B160), 0b0000001);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B80), 0b1000010);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B60), 0b1000100);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B40), 0b1000100);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B30), 0b1001000);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B20), 0b1001000);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B17), 0b1010000);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B15), 0b1010000);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B12), 0b1100000);
        assert_eq!(n2adr_filter_value_for_band(HamBand::B10), 0b1100000);
        // The C2 byte sent on the wire is value << 1, all <= 255.
        for band in HamBand::ALL {
            assert!((band.n2adr_filter_value() as u16) << 1 <= 0xFF);
        }
    }
}
