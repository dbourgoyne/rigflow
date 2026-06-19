/// Static description of a radio band, used for rendering the band overlay on
/// the spectrum.
///
/// The list below is transcribed 1:1 from `reference/Radio_Bands.csv` (the single
/// source of truth for band extents, names, and colors — amateur and non-amateur
/// alike). Update the CSV and re-transcribe when changing bands.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct RadioBand {
    /// Display name (e.g. "20m", "FM Broadcast").
    pub name: &'static str,

    /// Start frequency (Hz).
    pub start_hz: f32,

    /// End frequency (Hz).
    pub end_hz: f32,

    /// RGB color (0xRRGGBB).
    pub color: u32,
}

/// Global list of known radio bands — see `reference/Radio_Bands.csv`.
///
/// NOTE: frequencies are approximate, for visualization only.
#[rustfmt::skip]
pub const RADIO_BANDS: &[RadioBand] = &[
    RadioBand { name: "2200m",        start_hz: 135_700.0,       end_hz: 137_800.0,       color: 0x6557AD },
    RadioBand { name: "630m",         start_hz: 472_000.0,       end_hz: 479_000.0,       color: 0x6557AD },
    RadioBand { name: "AM Broadcast", start_hz: 530_000.0,       end_hz: 1_700_000.0,     color: 0x804000 },
    RadioBand { name: "160m",         start_hz: 1_800_000.0,     end_hz: 2_000_000.0,     color: 0x6557AD },
    RadioBand { name: "80m",          start_hz: 3_500_000.0,     end_hz: 4_000_000.0,     color: 0x6557AD },
    RadioBand { name: "60m",          start_hz: 5_330_500.0,     end_hz: 5_405_000.0,     color: 0x6557AD },
    RadioBand { name: "40m",          start_hz: 7_000_000.0,     end_hz: 7_300_000.0,     color: 0x6557AD },
    RadioBand { name: "30m",          start_hz: 10_100_000.0,    end_hz: 10_150_000.0,    color: 0x6557AD },
    RadioBand { name: "20m",          start_hz: 14_000_000.0,    end_hz: 14_350_000.0,    color: 0x6557AD },
    RadioBand { name: "17m",          start_hz: 18_068_000.0,    end_hz: 18_168_000.0,    color: 0x6557AD },
    RadioBand { name: "15m",          start_hz: 21_000_000.0,    end_hz: 21_450_000.0,    color: 0x6557AD },
    RadioBand { name: "12m",          start_hz: 24_890_000.0,    end_hz: 24_990_000.0,    color: 0x6557AD },
    RadioBand { name: "10m",          start_hz: 28_000_000.0,    end_hz: 29_700_000.0,    color: 0x6557AD },
    RadioBand { name: "6m",           start_hz: 50_000_000.0,    end_hz: 54_000_000.0,    color: 0x6557AD },
    RadioBand { name: "FM Broadcast", start_hz: 88_000_000.0,    end_hz: 108_000_000.0,   color: 0x205020 },
    RadioBand { name: "Air Band",     start_hz: 118_000_000.0,   end_hz: 137_000_000.0,   color: 0x505020 },
    RadioBand { name: "2m",           start_hz: 144_000_000.0,   end_hz: 148_000_000.0,   color: 0x6557AD },
    RadioBand { name: "NOAA Weather", start_hz: 162_400_000.0,   end_hz: 162_550_000.0,   color: 0x206060 },
    RadioBand { name: "1.25m",        start_hz: 219_000_000.0,   end_hz: 225_000_000.0,   color: 0x6557AD },
    RadioBand { name: "Military Air", start_hz: 225_000_000.0,   end_hz: 400_000_000.0,   color: 0x900000 },
    RadioBand { name: "70cm",         start_hz: 420_000_000.0,   end_hz: 450_000_000.0,   color: 0x6557AD },
    RadioBand { name: "33cm",         start_hz: 902_000_000.0,   end_hz: 928_000_000.0,   color: 0x6557AD },
    RadioBand { name: "23cm",         start_hz: 1_240_000_000.0, end_hz: 1_300_000_000.0, color: 0x6557AD },
];

/// A clipped version of a band that is currently visible on screen.
/// `start_hz`/`end_hz` are clipped to the view (for drawing); `band_start_hz`/
/// `band_end_hz` are the full allocation edges (for the hover tooltip).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct VisibleRadioBand {
    pub name: &'static str,
    pub start_hz: f32,
    pub end_hz: f32,
    pub band_start_hz: f32,
    pub band_end_hz: f32,
    pub color: u32,
}

/// Compute the subset of radio bands that intersect the current view, clipped to
/// the visible range.
pub fn visible_radio_bands(left_hz: f32, right_hz: f32) -> Vec<VisibleRadioBand> {
    if right_hz <= left_hz {
        return Vec::new();
    }

    RADIO_BANDS
        .iter()
        .filter_map(|band| {
            // Clamp band to visible region.
            let start_hz = band.start_hz.max(left_hz);
            let end_hz = band.end_hz.min(right_hz);

            if end_hz <= start_hz {
                None
            } else {
                Some(VisibleRadioBand {
                    name: band.name,
                    start_hz,
                    end_hz,
                    band_start_hz: band.start_hz,
                    band_end_hz: band.end_hz,
                    color: band.color,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_at(freq_hz: f32) -> Vec<&'static str> {
        visible_radio_bands(freq_hz - 100.0, freq_hz + 100.0)
            .into_iter()
            .map(|b| b.name)
            .collect()
    }

    #[test]
    fn full_band_count() {
        assert_eq!(RADIO_BANDS.len(), 23);
    }

    #[test]
    fn previously_missing_bands_now_present() {
        assert_eq!(names_at(136_000.0), vec!["2200m"]);
        assert_eq!(names_at(475_000.0), vec!["630m"]);
        assert_eq!(names_at(5_360_000.0), vec!["60m"]);
        assert_eq!(names_at(915_000_000.0), vec!["33cm"]);
        assert_eq!(names_at(1_270_000_000.0), vec!["23cm"]);
    }

    #[test]
    fn non_amateur_bands_retained() {
        assert_eq!(names_at(1_000_000.0), vec!["AM Broadcast"]);
        assert_eq!(names_at(98_000_000.0), vec!["FM Broadcast"]);
        assert_eq!(names_at(125_000_000.0), vec!["Air Band"]);
        assert_eq!(names_at(162_500_000.0), vec!["NOAA Weather"]);
        assert_eq!(names_at(300_000_000.0), vec!["Military Air"]);
    }

    #[test]
    fn clipping_carries_full_band_edges() {
        // A view window inside 20m returns the band clipped, but keeps the true
        // allocation edges for the tooltip.
        let v = visible_radio_bands(14_100_000.0, 14_200_000.0);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "20m");
        assert_eq!(v[0].start_hz, 14_100_000.0); // clipped to view
        assert_eq!(v[0].end_hz, 14_200_000.0);
        assert_eq!(v[0].band_start_hz, 14_000_000.0); // full edges
        assert_eq!(v[0].band_end_hz, 14_350_000.0);
    }
}
