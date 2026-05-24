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
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmKind {
    RttyData,
    PhoneImage,
    CwOnly,
    SsbPhone,
    UsbPhoneCwRttyData,
    FixedDigitalMessages,
}

/// Raw segment definition (full band allocation).
#[derive(Debug, Clone, Copy)]
pub struct OmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    pub kind: OmKind,
}

/// Visible (clipped) segment for rendering.
#[derive(Debug, Clone, Copy)]
pub struct VisibleOmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    pub kind: OmKind,
}

/// Amateur Extra privileges.
const AMATEUR_EXTRA_SEGMENTS: &[OmSegment] = &[
    // 10 meters
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
    },
    // VHF/UHF
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
    },
];

/// Advanced privileges (currently same as Extra for implemented bands).
const ADVANCED_SEGMENTS: &[OmSegment] = &[
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
    },
];

/// General privileges (currently same as Advanced in this simplified model).
const GENERAL_SEGMENTS: &[OmSegment] = &[
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
    },
];

/// Technician privileges (10m limited).
const TECHNICIAN_SEGMENTS: &[OmSegment] = &[
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 28_500_000.0,
        kind: OmKind::SsbPhone,
    },
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
    },
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
    },
];

/// Novice privileges (very limited HF).
const NOVICE_SEGMENTS: &[OmSegment] = &[
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 28_500_000.0,
        kind: OmKind::SsbPhone,
    },
];

/// Get all segments for a given license class.
pub fn om_segments_for_license(license: LicenseClass) -> &'static [OmSegment] {
    match license {
        LicenseClass::AmateurExtra => AMATEUR_EXTRA_SEGMENTS,
        LicenseClass::Advanced => ADVANCED_SEGMENTS,
        LicenseClass::General => GENERAL_SEGMENTS,
        LicenseClass::Technician => TECHNICIAN_SEGMENTS,
        LicenseClass::Novice => NOVICE_SEGMENTS,
    }
}

/// Compute visible OM segments clipped to the current spectrum view.
pub fn visible_om_segments(
    left_hz: f32,
    right_hz: f32,
    license: LicenseClass,
) -> Vec<VisibleOmSegment> {
    if right_hz <= left_hz {
        return Vec::new();
    }

    om_segments_for_license(license)
        .iter()
        .filter_map(|seg| {
            let start_hz = seg.start_hz.max(left_hz);
            let end_hz = seg.end_hz.min(right_hz);

            if end_hz <= start_hz {
                None
            } else {
                Some(VisibleOmSegment {
                    start_hz,
                    end_hz,
                    kind: seg.kind,
                })
            }
        })
        .collect()
}
