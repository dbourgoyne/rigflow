#[derive(Debug, Clone, Copy)]
pub struct RadioBand {
    pub name: &'static str,
    pub preferred_demod: &'static str,
    pub start_hz: f32,
    pub end_hz: f32,
    pub color: u32,
}

pub const RADIO_BANDS: &[RadioBand] = &[
    RadioBand {
        name: "AM Broadcast",
        preferred_demod: "AM",
        start_hz: 530_000.0,
        end_hz: 1_700_000.0,
        color: 0x804000,
    },
    RadioBand {
        name: "Shortwave",
        preferred_demod: "AM",
        start_hz: 2_300_000.0,
        end_hz: 26_100_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "FM Broadcast",
        preferred_demod: "WFM",
        start_hz: 88_000_000.0,
        end_hz: 108_000_000.0,
        color: 0x205020,
    },
    RadioBand {
        name: "Air Band",
        preferred_demod: "AM",
        start_hz: 118_000_000.0,
        end_hz: 137_000_000.0,
        color: 0x505020,
    },
    RadioBand {
        name: "2m Amateur",
        preferred_demod: "NFM",
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        color: 0x204060,
    },
    RadioBand {
        name: "NOAA Weather",
        preferred_demod: "NFM",
        start_hz: 162_400_000.0,
        end_hz: 162_550_000.0,
        color: 0x206060,
    },
    RadioBand {
        name: "Military Air",
        preferred_demod: "NFM",
        start_hz: 225_000_000.0,
        end_hz: 400_000_000.0,
        color: 0x900000,
    },
    RadioBand {
        name: "70cm Amateur",
        preferred_demod: "NFM",
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        color: 0x402060,
    },
];

/*
pub fn band_for_frequency(freq_hz: f32) -> Option<&'static RadioBand> {
    RADIO_BANDS
        .iter()
        .find(|band| freq_hz >= band.start_hz && freq_hz <= band.end_hz)
}
*/
