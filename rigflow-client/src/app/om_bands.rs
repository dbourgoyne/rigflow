const AMATEUR_EXTRA_SEGMENTS: &[OmSegment] = &[
    // 10 meters
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "10m",
    },

    // 6 meters
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "6m",
    },

    // 2 meters
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "2m",
    },

    // 1.25 meters
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "1.25m",
    },

    // 70 cm
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "70cm",
    },
];

const ADVANCED_SEGMENTS: &[OmSegment] = &[
    // Same starter VHF/UHF treatment as Extra
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "6m",
    },
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "2m",
    },
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "1.25m",
    },
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "70cm",
    },
];

const GENERAL_SEGMENTS: &[OmSegment] = &[
    // 10 meters
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 29_700_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "10m",
    },

    // 6 meters
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "6m",
    },

    // 2 meters
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "2m",
    },

    // 1.25 meters
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "1.25m",
    },

    // 70 cm
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "70cm",
    },
];

const TECHNICIAN_SEGMENTS: &[OmSegment] = &[
    // 10 meters: Novice/Technician privileges
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 28_500_000.0,
        kind: OmKind::SsbPhone,
        amateur_band_name: "10m",
    },

    // 6 meters
    OmSegment {
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "6m",
    },

    // 2 meters
    OmSegment {
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "2m",
    },

    // 1.25 meters
    OmSegment {
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "1.25m",
    },

    // 70 cm
    OmSegment {
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        kind: OmKind::PhoneImage,
        amateur_band_name: "70cm",
    },
];

const NOVICE_SEGMENTS: &[OmSegment] = &[
    // 10 meters: Novice privileges
    OmSegment {
        start_hz: 28_000_000.0,
        end_hz: 28_300_000.0,
        kind: OmKind::RttyData,
        amateur_band_name: "10m",
    },
    OmSegment {
        start_hz: 28_300_000.0,
        end_hz: 28_500_000.0,
        kind: OmKind::SsbPhone,
        amateur_band_name: "10m",
    },

    // 23 cm / 1.25 m differences exist for Novice, but as a starter we keep this conservative.
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseClass {
    AmateurExtra,
    Advanced,
    General,
    Technician,
    Novice,
}

pub fn next_license(license: LicenseClass) -> LicenseClass {
    match license {
        LicenseClass::AmateurExtra => LicenseClass::Advanced,
        LicenseClass::Advanced => LicenseClass::General,
        LicenseClass::General => LicenseClass::Technician,
        LicenseClass::Technician => LicenseClass::Novice,
        LicenseClass::Novice => LicenseClass::AmateurExtra,
    }
}

pub fn prev_license(license: LicenseClass) -> LicenseClass {
    match license {
        LicenseClass::AmateurExtra => LicenseClass::Novice,
        LicenseClass::Advanced => LicenseClass::AmateurExtra,
        LicenseClass::General => LicenseClass::Advanced,
        LicenseClass::Technician => LicenseClass::General,
        LicenseClass::Novice => LicenseClass::Technician,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmKind {
    RttyData,
    PhoneImage,
    CwOnly,
    SsbPhone,
    UsbPhoneCwRttyData,
    FixedDigitalMessages,
}

#[derive(Debug, Clone, Copy)]
pub struct OmSegment {
    pub start_hz: f32,
    pub end_hz: f32,
    pub kind: OmKind,
    pub amateur_band_name: &'static str,
}

pub fn om_segments_for_license(license: LicenseClass) -> &'static [OmSegment] {
    match license {
        LicenseClass::AmateurExtra => AMATEUR_EXTRA_SEGMENTS,
        LicenseClass::Advanced => ADVANCED_SEGMENTS,
        LicenseClass::General => GENERAL_SEGMENTS,
        LicenseClass::Technician => TECHNICIAN_SEGMENTS,
        LicenseClass::Novice => NOVICE_SEGMENTS,
    }
}
