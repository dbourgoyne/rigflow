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

/// OM segment type — maps to the bar color and the in-bar abbreviation. The full
/// hover text comes from the CSV `Key` column (`OmSegment::key`), not from here.
/// The discriminant order is also the vertical stacking tiebreak (RttyData stacks
/// above PhoneImage), so do not reorder casually.
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
    /// Allowed-operations text from the CSV `Key` column (shown on hover).
    pub key: &'static str,
}

/// Visible segment for rendering. `start_hz`/`end_hz` are clipped to the current
/// view (used to draw the bar); `band_start_hz`/`band_end_hz` are the full
/// allocation edges (used for the hover tooltip). `lane`/`lanes` describe vertical
/// stacking: `lane` is this segment's index and `lanes` the number of stacked
/// lanes in its overlap cluster (`lanes == 1` when nothing overlaps it).
#[derive(Debug, Clone, Copy)]
pub struct VisibleOmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    pub band_start_hz: f32,
    pub band_end_hz: f32,
    pub kind: OmKind,
    pub key: &'static str,
    pub lane: u8,
    pub lanes: u8,
}

/// Complete US amateur band plan, transcribed 1:1 from `reference/ARRL_Band_Plan.csv`
/// (the source of truth). Each row carries the license classes it applies to (so
/// per-license and overlapping allocations coexist and are filtered/stacked at
/// render time), the color via `OmKind`, and the hover `key` text. Color → kind:
/// Red=RttyData, Green=PhoneImage, White=CwOnly, Yellow=SsbPhone,
/// Blue=UsbPhoneCwRttyData, Orange=FixedDigitalMessages.
#[rustfmt::skip]
const BAND_PLAN: &[OmSegment] = &[
    // 2200 m
    OmSegment { start_hz: 135_700.0, end_hz: 137_800.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 135_700.0, end_hz: 137_800.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    // 630 m
    OmSegment { start_hz: 472_000.0, end_hz: 479_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 472_000.0, end_hz: 479_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    // 160 m
    OmSegment { start_hz: 1_800_000.0, end_hz: 2_000_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 1_800_000.0, end_hz: 2_000_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    // 80 m
    OmSegment { start_hz: 3_500_000.0, end_hz: 3_600_000.0, licenses: LIC_E, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 3_600_000.0, end_hz: 4_000_000.0, licenses: LIC_E, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 3_525_000.0, end_hz: 3_600_000.0, licenses: LIC_A, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 3_700_000.0, end_hz: 4_000_000.0, licenses: LIC_A, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 3_525_000.0, end_hz: 3_600_000.0, licenses: LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 3_800_000.0, end_hz: 4_000_000.0, licenses: LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 3_525_000.0, end_hz: 3_600_000.0, licenses: LIC_N | LIC_T, kind: OmKind::CwOnly, key: "CW only" },
    // 60 m
    OmSegment { start_hz: 5_332_000.0, end_hz: 5_405_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::UsbPhoneCwRttyData, key: "USB, CW and digital modes" },
    // 40 m
    OmSegment { start_hz: 7_000_000.0, end_hz: 7_125_000.0, licenses: LIC_E, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 7_125_000.0, end_hz: 7_300_000.0, licenses: LIC_E, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 7_025_000.0, end_hz: 7_125_000.0, licenses: LIC_A, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 7_125_000.0, end_hz: 7_300_000.0, licenses: LIC_A, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 7_025_000.0, end_hz: 7_125_000.0, licenses: LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 7_175_000.0, end_hz: 7_300_000.0, licenses: LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 7_025_000.0, end_hz: 7_125_000.0, licenses: LIC_N | LIC_T, kind: OmKind::CwOnly, key: "CW only" },
    // 30 m
    OmSegment { start_hz: 10_100_000.0, end_hz: 10_150_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    // 20 m
    OmSegment { start_hz: 14_000_000.0, end_hz: 14_150_000.0, licenses: LIC_E, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 14_150_000.0, end_hz: 14_350_000.0, licenses: LIC_E, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 14_025_000.0, end_hz: 14_150_000.0, licenses: LIC_A, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 14_175_000.0, end_hz: 14_350_000.0, licenses: LIC_A, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 14_025_000.0, end_hz: 14_150_000.0, licenses: LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 14_225_000.0, end_hz: 14_350_000.0, licenses: LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    // 17 m
    OmSegment { start_hz: 18_068_000.0, end_hz: 18_110_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 18_110_000.0, end_hz: 18_168_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    // 15 m
    OmSegment { start_hz: 21_000_000.0, end_hz: 21_200_000.0, licenses: LIC_E, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 21_200_000.0, end_hz: 21_450_000.0, licenses: LIC_E, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 21_025_000.0, end_hz: 21_200_000.0, licenses: LIC_A, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 21_225_000.0, end_hz: 21_450_000.0, licenses: LIC_A, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 21_025_000.0, end_hz: 21_200_000.0, licenses: LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 21_275_000.0, end_hz: 21_450_000.0, licenses: LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 21_025_000.0, end_hz: 21_200_000.0, licenses: LIC_N | LIC_T, kind: OmKind::CwOnly, key: "CW only" },
    // 12 m
    OmSegment { start_hz: 24_890_000.0, end_hz: 24_930_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 24_930_000.0, end_hz: 24_990_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    // 10 m
    OmSegment { start_hz: 28_000_000.0, end_hz: 28_300_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 28_300_000.0, end_hz: 29_700_000.0, licenses: LIC_E | LIC_A | LIC_G, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 28_000_000.0, end_hz: 28_300_000.0, licenses: LIC_N | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 28_300_000.0, end_hz: 28_500_000.0, licenses: LIC_N | LIC_T, kind: OmKind::SsbPhone, key: "SSB phone" },
    // 6 m
    OmSegment { start_hz: 50_000_000.0, end_hz: 50_100_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::CwOnly, key: "CW only" },
    OmSegment { start_hz: 50_100_000.0, end_hz: 54_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 50_100_000.0, end_hz: 54_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    // 2 m
    OmSegment { start_hz: 144_000_000.0, end_hz: 144_100_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::CwOnly, key: "CW only" },
    OmSegment { start_hz: 144_100_000.0, end_hz: 148_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 144_100_000.0, end_hz: 148_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    // 1.25 m
    OmSegment { start_hz: 219_000_000.0, end_hz: 220_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::FixedDigitalMessages, key: "Fixed digital message forwarding systems only" },
    OmSegment { start_hz: 222_000_000.0, end_hz: 225_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T | LIC_N, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 222_000_000.0, end_hz: 225_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T | LIC_N, kind: OmKind::RttyData, key: "RTTY and data" },
    // 70 cm
    OmSegment { start_hz: 420_000_000.0, end_hz: 450_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 420_000_000.0, end_hz: 450_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    // 33 cm
    OmSegment { start_hz: 902_000_000.0, end_hz: 928_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 902_000_000.0, end_hz: 928_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    // 23 cm
    OmSegment { start_hz: 1_240_000_000.0, end_hz: 1_300_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 1_240_000_000.0, end_hz: 1_300_000_000.0, licenses: LIC_E | LIC_A | LIC_G | LIC_T, kind: OmKind::RttyData, key: "RTTY and data" },
    OmSegment { start_hz: 1_270_000_000.0, end_hz: 1_295_000_000.0, licenses: LIC_N, kind: OmKind::PhoneImage, key: "phone and image" },
    OmSegment { start_hz: 1_270_000_000.0, end_hz: 1_295_000_000.0, licenses: LIC_N, kind: OmKind::RttyData, key: "RTTY and data" },
];

/// Compute visible OM segments clipped to the current spectrum view, filtered to
/// the operator's license class, with vertical lanes assigned for overlaps.
pub fn visible_om_segments(
    left_hz: f32,
    right_hz: f32,
    license: LicenseClass,
) -> Vec<VisibleOmSegment> {
    if right_hz <= left_hz {
        return Vec::new();
    }

    let bit = license_bit(license);

    let mut visible: Vec<VisibleOmSegment> = BAND_PLAN
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
                    key: seg.key,
                    lane: 0,
                    lanes: 1,
                })
            }
        })
        .collect();

    assign_lanes(&mut visible);
    visible
}

/// Assign vertical stacking lanes so segments that overlap in frequency render as
/// stacked bars instead of painting over each other. Segments are grouped into
/// clusters of transitively-overlapping ranges; within each cluster, each segment
/// takes the lowest lane not already occupied at its start, and every member of the
/// cluster is told the cluster's total lane count. Non-overlapping segments end up
/// with `lane = 0`, `lanes = 1` (full height).
fn assign_lanes(segs: &mut [VisibleOmSegment]) {
    // Sort by start, then by kind discriminant for a deterministic stack order
    // (RttyData above PhoneImage).
    segs.sort_by(|a, b| {
        a.start_hz
            .partial_cmp(&b.start_hz)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((a.kind as u8).cmp(&(b.kind as u8)))
    });

    let mut i = 0;
    while i < segs.len() {
        // Grow a cluster [i, j) of transitively overlapping segments.
        let mut j = i + 1;
        let mut cluster_end = segs[i].end_hz;
        while j < segs.len() && segs[j].start_hz < cluster_end {
            cluster_end = cluster_end.max(segs[j].end_hz);
            j += 1;
        }

        // Greedy lane assignment within the cluster.
        let mut lane_end: Vec<f32> = Vec::new();
        for seg in segs[i..j].iter_mut() {
            let start = seg.start_hz;
            let mut placed = None;
            for (lane, end) in lane_end.iter_mut().enumerate() {
                if *end <= start {
                    *end = seg.end_hz;
                    placed = Some(lane);
                    break;
                }
            }
            seg.lane = placed.unwrap_or_else(|| {
                lane_end.push(seg.end_hz);
                lane_end.len() - 1
            }) as u8;
        }

        let lanes = lane_end.len() as u8;
        for seg in segs[i..j].iter_mut() {
            seg.lanes = lanes;
        }

        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Collect (kind, lane, lanes) visible at a single frequency for a license.
    // Use a ±100 Hz window: a tighter one can collapse to zero width once f32
    // rounding kicks in above ~16 MHz (step ≥ 2 Hz), irrelevant in practice where
    // the view span is many kHz wide.
    fn at(freq_hz: f32, license: LicenseClass) -> Vec<(OmKind, u8, u8)> {
        visible_om_segments(freq_hz - 100.0, freq_hz + 100.0, license)
            .into_iter()
            .map(|s| (s.kind, s.lane, s.lanes))
            .collect()
    }

    #[test]
    fn overlapping_red_green_stacks_into_two_lanes() {
        // 222–225 MHz (Extra): RTTY/data + phone/image overlap → 2 lanes, RTTY on top.
        let mut v = at(223_000_000.0, LicenseClass::AmateurExtra);
        v.sort_by_key(|(_, lane, _)| *lane);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], (OmKind::RttyData, 0, 2));
        assert_eq!(v[1], (OmKind::PhoneImage, 1, 2));
    }

    #[test]
    fn non_overlapping_segment_is_full_height() {
        // 20 m phone (Extra) does not overlap anything → single full-height lane.
        let v = at(14_250_000.0, LicenseClass::AmateurExtra);
        assert_eq!(v, vec![(OmKind::PhoneImage, 0, 1)]);
    }

    #[test]
    fn license_filtering_still_selects_the_right_rows() {
        // 28.300–28.500 MHz: phone/image for E,A,G but SSB phone for N,T.
        assert_eq!(
            at(28_400_000.0, LicenseClass::General),
            vec![(OmKind::PhoneImage, 0, 1)]
        );
        assert_eq!(
            at(28_400_000.0, LicenseClass::Novice),
            vec![(OmKind::SsbPhone, 0, 1)]
        );
    }

    #[test]
    fn hover_key_is_carried_through() {
        let v = visible_om_segments(14_240_000.0, 14_260_000.0, LicenseClass::General);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key, "phone and image");
    }

    #[test]
    fn clipping_carries_full_band_edges() {
        // A window inside 20 m phone returns it clipped, keeping the true edges.
        let v = visible_om_segments(14_200_000.0, 14_300_000.0, LicenseClass::AmateurExtra);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, OmKind::PhoneImage);
        assert_eq!(v[0].start_hz, 14_200_000.0);
        assert_eq!(v[0].end_hz, 14_300_000.0);
        assert_eq!(v[0].band_start_hz, 14_150_000.0);
        assert_eq!(v[0].band_end_hz, 14_350_000.0);
    }
}
