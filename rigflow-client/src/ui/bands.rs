/// Static description of a radio band.
///
/// This is used for:
/// - rendering band overlays on the spectrum
/// - suggesting a preferred demod mode (UI hint only)
use rigflow_core::dsp::modes::DemodMode;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct RadioBand {
    /// Display name (e.g. "40m Amateur")
    pub name: &'static str,

    /// Preferred demod mode (UI hint only; still string for now)
    pub preferred_demod: DemodMode,

    /// Start frequency (Hz)
    pub start_hz: f32,

    /// End frequency (Hz)
    pub end_hz: f32,

    /// RGB color (0xRRGGBB)
    pub color: u32,
}

/// Global list of known radio bands.
///
/// NOTE:
/// - Frequencies are approximate and for visualization
/// - preferred_demod is a UI hint only (not enforced)
pub const RADIO_BANDS: &[RadioBand] = &[
    RadioBand {
        name: "AM Broadcast",
        preferred_demod: DemodMode::Am,
        start_hz: 530_000.0,
        end_hz: 1_700_000.0,
        color: 0x804000,
    },
    RadioBand {
        name: "160m Amateur",
        preferred_demod: DemodMode::Lsb,
        start_hz: 1_800_000.0,
        end_hz: 2_000_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "80m Amateur",
        preferred_demod: DemodMode::Lsb,
        start_hz: 3_500_000.0,
        end_hz: 4_000_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "40m Amateur",
        preferred_demod: DemodMode::Lsb,
        start_hz: 7_000_000.0,
        end_hz: 7_300_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "30m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 10_100_000.0,
        end_hz: 10_150_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "20m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 14_000_000.0,
        end_hz: 14_350_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "17m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 18_068_000.0,
        end_hz: 18_168_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "15m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 21_000_000.0,
        end_hz: 21_450_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "12m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 24_890_000.0,
        end_hz: 24_990_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "10m Amateur",
        preferred_demod: DemodMode::Usb,
        start_hz: 28_000_000.0,
        end_hz: 29_700_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "6m Amateur",
        preferred_demod: DemodMode::Am,
        start_hz: 50_000_000.0,
        end_hz: 54_000_000.0,
        color: 0x305080,
    },
    RadioBand {
        name: "FM Broadcast",
        preferred_demod: DemodMode::Wfm,
        start_hz: 88_000_000.0,
        end_hz: 108_000_000.0,
        color: 0x205020,
    },
    RadioBand {
        name: "Air Band",
        preferred_demod: DemodMode::Am,
        start_hz: 118_000_000.0,
        end_hz: 137_000_000.0,
        color: 0x505020,
    },
    RadioBand {
        name: "2m Amateur",
        preferred_demod: DemodMode::Nfm,
        start_hz: 144_000_000.0,
        end_hz: 148_000_000.0,
        color: 0x204060,
    },
    RadioBand {
        name: "NOAA Weather",
        preferred_demod: DemodMode::Nfm,
        start_hz: 162_400_000.0,
        end_hz: 162_550_000.0,
        color: 0x206060,
    },
    RadioBand {
        name: "1.22m Amateur",
        preferred_demod: DemodMode::Nfm,
        start_hz: 222_000_000.0,
        end_hz: 225_000_000.0,
        color: 0x204060,
    },
    RadioBand {
        name: "Military Air",
        preferred_demod: DemodMode::Nfm,
        start_hz: 225_000_000.0,
        end_hz: 400_000_000.0,
        color: 0x900000,
    },
    RadioBand {
        name: "70cm Amateur",
        preferred_demod: DemodMode::Nfm,
        start_hz: 420_000_000.0,
        end_hz: 450_000_000.0,
        color: 0x402060,
    },
];

/// A clipped version of a band that is currently visible on screen.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct VisibleRadioBand {
    pub name: &'static str,
    pub preferred_demod: DemodMode,
    pub start_hz: f32,
    pub end_hz: f32,
    pub color: u32,
}

/// Compute the subset of radio bands that intersect the current view.
///
/// Inputs:
/// - `left_hz`: left edge of visible spectrum
/// - `right_hz`: right edge of visible spectrum
///
/// Output:
/// - list of bands clipped to the visible range
pub fn visible_radio_bands(left_hz: f32, right_hz: f32) -> Vec<VisibleRadioBand> {
    if right_hz <= left_hz {
        return Vec::new();
    }

    RADIO_BANDS
        .iter()
        .filter_map(|band| {
            // Clamp band to visible region
            let start_hz = band.start_hz.max(left_hz);
            let end_hz = band.end_hz.min(right_hz);

            if end_hz <= start_hz {
                None
            } else {
                Some(VisibleRadioBand {
                    name: band.name,
                    preferred_demod: band.preferred_demod,
                    start_hz,
                    end_hz,
                    color: band.color,
                })
            }
        })
        .collect()
}
