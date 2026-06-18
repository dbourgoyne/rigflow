/// Color constants for OM segment visualization (0xRRGGBB).
pub const COLOR_OM_RTTY_DATA: u32 = 0x00c00000; // red
pub const COLOR_OM_PHONE_IMAGE: u32 = 0x0000a000; // green
pub const COLOR_OM_CW_ONLY: u32 = 0x00f0f0f0; // white
pub const COLOR_OM_SSB_PHONE: u32 = 0x00d0c000; // yellow
pub const COLOR_OM_USB_PHONE_CW_RTTY_DATA: u32 = 0x0040b0ff; // light blue
pub const COLOR_OM_FIXED_DIGITAL: u32 = 0x00ff9000; // orange

/// Classification of operator privileges.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LicenseClass {
    AmateurExtra,
    Advanced,
    General,
    Technician,
    Novice,
}

// License bitmask used by the band-plan table so one flat list can carry the
// per-license rows from ARRL_Band_Plan.csv (e.g. a sub-band that is RTTY/data
// for E,A,G but CW-only for N,T).
const LIC_E: u8 = 1 << 0; // Amateur Extra
const LIC_A: u8 = 1 << 1; // Advanced
const LIC_G: u8 = 1 << 2; // General
const LIC_T: u8 = 1 << 3; // Technician
const LIC_N: u8 = 1 << 4; // Novice

/// Single-bit mask for one license class (for filtering the band plan).
fn license_bit(license: LicenseClass) -> u8 {
    match license {
        LicenseClass::AmateurExtra => LIC_E,
        LicenseClass::Advanced => LIC_A,
        LicenseClass::General => LIC_G,
        LicenseClass::Technician => LIC_T,
        LicenseClass::Novice => LIC_N,
    }
}

#[allow(dead_code)]
/// Cycle forward through license classes (used for UI toggling).
pub fn next_license(license: LicenseClass) -> LicenseClass {
    match license {
        LicenseClass::AmateurExtra => LicenseClass::Advanced,
        LicenseClass::Advanced => LicenseClass::General,
        LicenseClass::General => LicenseClass::Technician,
        LicenseClass::Technician => LicenseClass::Novice,
        LicenseClass::Novice => LicenseClass::AmateurExtra,
    }
}

#[allow(dead_code)]
/// Cycle backward through license classes.
pub fn prev_license(license: LicenseClass) -> LicenseClass {
    match license {
        LicenseClass::AmateurExtra => LicenseClass::Novice,
        LicenseClass::Advanced => LicenseClass::AmateurExtra,
        LicenseClass::General => LicenseClass::Advanced,
        LicenseClass::Technician => LicenseClass::General,
        LicenseClass::Novice => LicenseClass::Technician,
    }
}

/// OM segment type (used for coloring + meaning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmKind {
    RttyData,
    PhoneImage,
    CwOnly,
    SsbPhone,
    UsbPhoneCwRttyData,
    FixedDigitalMessages,
}

/// Raw segment definition (full band allocation, one row per CSV line).
#[derive(Debug, Clone, Copy)]
pub struct OmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    /// Bitmask of the license classes this allocation applies to (`LIC_*`).
    pub licenses: u8,
    pub kind: OmKind,
}

/// Visible segment for rendering. `start_hz`/`end_hz` are clipped to the current
/// view (used to draw the bar); `band_start_hz`/`band_end_hz` are the full
/// allocation edges (used for the hover tooltip).
#[derive(Debug, Clone, Copy)]
pub struct VisibleOmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    pub band_start_hz: f32,
    pub band_end_hz: f32,
    pub kind: OmKind,
}

/// Complete US amateur band plan, transcribed from `reference/ARRL_Band_Plan.csv`
/// (kept in the repo as the source of truth). Each row carries the license
/// classes it applies to, so overlapping per-license allocations coexist and
/// are filtered at render time. Colors map to `OmKind`:
/// Red=RttyData, Green=PhoneImage, White=CwOnly, Yellow=SsbPhone,
/// Blue=UsbPhoneCwRttyData, Orange=FixedDigitalMessages.
#[rustfmt::skip]
const BAND_PLAN: &[OmSegment] = &[
    // 2200 m / 630 m / 160 m
    OmSegment { start_hz: 135_700.0,       end_hz: 137_800.0,       licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 472_000.0,       end_hz: 479_000.0,       licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 1_800_000.0,     end_hz: 2_000_000.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    // 80 m
    OmSegment { start_hz: 3_500_000.0,     end_hz: 3_525_000.0,     licenses: LIC_E,                         kind: OmKind::RttyData },
    OmSegment { start_hz: 3_525_000.0,     end_hz: 3_600_000.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 3_500_000.0,     end_hz: 3_525_000.0,     licenses: LIC_N | LIC_T,                 kind: OmKind::CwOnly },
    OmSegment { start_hz: 3_600_000.0,     end_hz: 4_000_000.0,     licenses: LIC_E,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 3_700_000.0,     end_hz: 4_000_000.0,     licenses: LIC_A,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 3_800_000.0,     end_hz: 4_000_000.0,     licenses: LIC_G,                         kind: OmKind::PhoneImage },
    // 60 m (channelized)
    OmSegment { start_hz: 5_330_500.0,     end_hz: 5_333_300.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 5_346_500.0,     end_hz: 5_349_300.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 5_371_500.0,     end_hz: 5_374_300.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 5_403_500.0,     end_hz: 5_406_300.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 5_351_500.0,     end_hz: 5_366_500.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::UsbPhoneCwRttyData },
    // 40 m
    OmSegment { start_hz: 7_000_000.0,     end_hz: 7_025_000.0,     licenses: LIC_E,                         kind: OmKind::RttyData },
    OmSegment { start_hz: 7_025_000.0,     end_hz: 7_125_000.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::CwOnly },
    OmSegment { start_hz: 7_125_000.0,     end_hz: 7_175_000.0,     licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 7_175_000.0,     end_hz: 7_300_000.0,     licenses: LIC_E,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 7_225_000.0,     end_hz: 7_300_000.0,     licenses: LIC_A,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 7_275_000.0,     end_hz: 7_300_000.0,     licenses: LIC_G,                         kind: OmKind::PhoneImage },
    // 30 m
    OmSegment { start_hz: 10_100_000.0,    end_hz: 10_150_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    // 20 m
    OmSegment { start_hz: 14_000_000.0,    end_hz: 14_025_000.0,    licenses: LIC_E,                         kind: OmKind::RttyData },
    OmSegment { start_hz: 14_025_000.0,    end_hz: 14_150_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 14_150_000.0,    end_hz: 14_350_000.0,    licenses: LIC_E,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 14_175_000.0,    end_hz: 14_350_000.0,    licenses: LIC_A,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 14_225_000.0,    end_hz: 14_350_000.0,    licenses: LIC_G,                         kind: OmKind::PhoneImage },
    // 17 m
    OmSegment { start_hz: 18_068_000.0,    end_hz: 18_110_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 18_110_000.0,    end_hz: 18_168_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    // 15 m
    OmSegment { start_hz: 21_000_000.0,    end_hz: 21_025_000.0,    licenses: LIC_E,                         kind: OmKind::RttyData },
    OmSegment { start_hz: 21_025_000.0,    end_hz: 21_200_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 21_000_000.0,    end_hz: 21_200_000.0,    licenses: LIC_N | LIC_T,                 kind: OmKind::CwOnly },
    OmSegment { start_hz: 21_200_000.0,    end_hz: 21_450_000.0,    licenses: LIC_E,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 21_225_000.0,    end_hz: 21_450_000.0,    licenses: LIC_A,                         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 21_275_000.0,    end_hz: 21_450_000.0,    licenses: LIC_G,                         kind: OmKind::PhoneImage },
    // 12 m
    OmSegment { start_hz: 24_890_000.0,    end_hz: 24_930_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 24_930_000.0,    end_hz: 24_990_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    // 10 m
    OmSegment { start_hz: 28_000_000.0,    end_hz: 28_300_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::RttyData },
    OmSegment { start_hz: 28_000_000.0,    end_hz: 28_300_000.0,    licenses: LIC_N | LIC_T,                 kind: OmKind::CwOnly },
    OmSegment { start_hz: 28_300_000.0,    end_hz: 28_500_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    OmSegment { start_hz: 28_300_000.0,    end_hz: 28_500_000.0,    licenses: LIC_N | LIC_T,                 kind: OmKind::SsbPhone },
    OmSegment { start_hz: 28_500_000.0,    end_hz: 29_700_000.0,    licenses: LIC_E | LIC_A | LIC_G,         kind: OmKind::PhoneImage },
    // 6 m
    OmSegment { start_hz: 50_000_000.0,    end_hz: 50_100_000.0,    licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 50_100_000.0,    end_hz: 54_000_000.0,    licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    // 2 m
    OmSegment { start_hz: 144_000_000.0,   end_hz: 144_100_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::UsbPhoneCwRttyData },
    OmSegment { start_hz: 144_100_000.0,   end_hz: 148_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    // 1.25 m
    OmSegment { start_hz: 219_000_000.0,   end_hz: 220_000_000.0,   licenses: LIC_N,                         kind: OmKind::FixedDigitalMessages },
    OmSegment { start_hz: 222_000_000.0,   end_hz: 225_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    OmSegment { start_hz: 222_000_000.0,   end_hz: 225_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData },
    // 70 cm
    OmSegment { start_hz: 420_000_000.0,   end_hz: 450_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    OmSegment { start_hz: 420_000_000.0,   end_hz: 450_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData },
    // 33 cm
    OmSegment { start_hz: 902_000_000.0,   end_hz: 928_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    OmSegment { start_hz: 902_000_000.0,   end_hz: 928_000_000.0,   licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData },
    // 23 cm
    OmSegment { start_hz: 1_240_000_000.0, end_hz: 1_300_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage },
    OmSegment { start_hz: 1_270_000_000.0, end_hz: 1_295_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData },
    OmSegment { start_hz: 1_240_000_000.0, end_hz: 1_270_000_000.0, licenses: LIC_N,                         kind: OmKind::PhoneImage }, // Novice: 5 W limit
];

/// Compute visible OM segments clipped to the current spectrum view, filtered to
/// the operator's license class.
pub fn visible_om_segments(
    left_hz: f32,
    right_hz: f32,
    license: LicenseClass,
) -> Vec<VisibleOmSegment> {
    if right_hz <= left_hz {
        return Vec::new();
    }

    let bit = license_bit(license);

    BAND_PLAN
        .iter()
        .filter(|seg| seg.licenses & bit != 0)
        .filter_map(|seg| {
            let start_hz = seg.start_hz.max(left_hz);
            let end_hz = seg.end_hz.min(right_hz);

            if end_hz <= start_hz {
                None
            } else {
                Some(VisibleOmSegment {
                    start_hz,
                    end_hz,
                    band_start_hz: seg.start_hz,
                    band_end_hz: seg.end_hz,
                    kind: seg.kind,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: collect the kinds visible at a single frequency for a license.
    // Use a ±100 Hz window: a tighter one can collapse to zero width once f32
    // rounding kicks in above ~16 MHz (step ≥ 2 Hz), which is irrelevant in
    // practice where the view span is many kHz wide.
    fn kinds_at(freq_hz: f32, license: LicenseClass) -> Vec<OmKind> {
        visible_om_segments(freq_hz - 100.0, freq_hz + 100.0, license)
            .into_iter()
            .map(|s| s.kind)
            .collect()
    }

    #[test]
    fn license_filtering_selects_the_right_row() {
        // 28.300–28.500 MHz is Phone/Image for E,A,G but SSB-phone for N,T.
        assert_eq!(
            kinds_at(28_400_000.0, LicenseClass::General),
            vec![OmKind::PhoneImage]
        );
        assert_eq!(
            kinds_at(28_400_000.0, LicenseClass::Novice),
            vec![OmKind::SsbPhone]
        );
        assert_eq!(
            kinds_at(28_400_000.0, LicenseClass::Technician),
            vec![OmKind::SsbPhone]
        );
    }

    #[test]
    fn extra_only_subband_excluded_for_general() {
        // 3.500–3.525 MHz is RTTY/data for Extra only (General has nothing here;
        // Novice/Tech get CW-only).
        assert_eq!(
            kinds_at(3_510_000.0, LicenseClass::AmateurExtra),
            vec![OmKind::RttyData]
        );
        assert!(kinds_at(3_510_000.0, LicenseClass::General).is_empty());
        assert_eq!(
            kinds_at(3_510_000.0, LicenseClass::Novice),
            vec![OmKind::CwOnly]
        );
    }

    #[test]
    fn lower_hf_bands_are_present() {
        // Regression: the old hard-coded table omitted everything below 10 m.
        assert!(!kinds_at(14_200_000.0, LicenseClass::AmateurExtra).is_empty());
        assert!(!kinds_at(7_050_000.0, LicenseClass::General).is_empty());
        assert!(!kinds_at(1_900_000.0, LicenseClass::Advanced).is_empty());
    }

    #[test]
    fn clipping_to_view_window_works() {
        // A window covering only part of the 20 m phone band still returns it,
        // clipped to the window.
        let segs = visible_om_segments(14_300_000.0, 14_400_000.0, LicenseClass::AmateurExtra);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].kind, OmKind::PhoneImage);
        assert_eq!(segs[0].start_hz, 14_300_000.0);
        assert_eq!(segs[0].end_hz, 14_350_000.0); // clipped to the band edge
    }
}
