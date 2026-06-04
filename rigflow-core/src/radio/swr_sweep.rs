//! SWR sweep: types, point/frequency math, and range validation.
//!
//! A sweep runs the existing Spot/SWR measurement at a fixed number of
//! frequencies across a single amateur band and collects SWR (and raw
//! forward/reverse detector readings) per point.  V1 is fixed at
//! [`SWR_SWEEP_POINTS`] points; no user-configurable point count or step.

use serde::{Deserialize, Serialize};

use crate::radio::ham_band::band_from_frequency;

/// Number of sweep points (inclusive of both endpoints).
pub const SWR_SWEEP_POINTS: u32 = 25;

/// One measured point of an SWR sweep.
///
/// `swr` is the ratio-derived SWR (uncalibrated forward/reverse).  Watts are
/// not yet calibrated, so `forward_raw`/`reverse_raw` carry the raw detector
/// counts (the available power proxy).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SwrSweepPoint {
    pub frequency_hz: u64,
    pub swr: Option<f32>,
    pub forward_raw: Option<u16>,
    pub reverse_raw: Option<u16>,
}

/// A completed (or cancelled) SWR sweep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwrSweepResult {
    pub start_hz: u64,
    pub stop_hz: u64,
    pub points: Vec<SwrSweepPoint>,
}

impl SwrSweepResult {
    /// The point with the lowest valid SWR, if any.
    pub fn min_swr_point(&self) -> Option<&SwrSweepPoint> {
        self.points
            .iter()
            .filter(|p| p.swr.is_some())
            .min_by(|a, b| {
                a.swr
                    .unwrap()
                    .partial_cmp(&b.swr.unwrap())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// Live progress of an in-flight sweep (published as runtime status).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SwrSweepProgress {
    pub running: bool,
    pub done: u32,
    pub total: u32,
}

/// Frequency (Hz) of sweep point `i` in `0..points`, with inclusive endpoints:
/// point 0 == `start_hz`, point `points-1` == `stop_hz`.
pub fn sweep_frequency_hz(start_hz: u64, stop_hz: u64, i: u32, points: u32) -> u64 {
    if points <= 1 || i == 0 {
        return start_hz;
    }
    if i >= points - 1 {
        return stop_hz;
    }
    let span = stop_hz as f64 - start_hz as f64;
    let step = span / (points - 1) as f64;
    (start_hz as f64 + step * i as f64).round() as u64
}

/// Validate a sweep range.  Returns a user-friendly error string on failure.
///
/// Rules: stop > start, and both endpoints map to the *same* supported band.
/// No license-class logic.
pub fn validate_sweep_range(start_hz: u64, stop_hz: u64) -> Result<(), String> {
    if stop_hz <= start_hz {
        return Err("Stop frequency must be greater than Start frequency.".to_string());
    }
    match (band_from_frequency(start_hz), band_from_frequency(stop_hz)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(a), Some(b)) => Err(format!(
            "Start is in {} but Stop is in {} — a sweep must stay within one band.",
            a.label(),
            b.label()
        )),
        _ => Err("Start and Stop must both be inside a supported amateur band.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radio::ham_band::HamBand;

    #[test]
    fn sweep_endpoints_and_spacing() {
        let (start, stop) = (14_000_000, 14_350_000);
        assert_eq!(sweep_frequency_hz(start, stop, 0, 25), start);
        assert_eq!(sweep_frequency_hz(start, stop, 24, 25), stop);
        // Step ≈ (stop-start)/24 ≈ 14583 Hz; midpoint ≈ start + 12*step.
        let mid = sweep_frequency_hz(start, stop, 12, 25);
        assert!((mid as i64 - 14_175_000).abs() < 1_000, "mid={mid}");
        // Monotonic non-decreasing.
        for i in 1..25 {
            assert!(sweep_frequency_hz(start, stop, i, 25) >= sweep_frequency_hz(start, stop, i - 1, 25));
        }
    }

    #[test]
    fn validate_ok_within_band() {
        let (lo, hi) = HamBand::B20.range_hz();
        assert!(validate_sweep_range(lo, hi).is_ok());
        assert!(validate_sweep_range(14_100_000, 14_300_000).is_ok());
    }

    #[test]
    fn validate_rejects_bad_ranges() {
        assert!(validate_sweep_range(14_200_000, 14_200_000).is_err()); // stop == start
        assert!(validate_sweep_range(14_300_000, 14_100_000).is_err()); // stop < start
        assert!(validate_sweep_range(14_300_000, 21_100_000).is_err()); // different bands
        assert!(validate_sweep_range(5_000_000, 5_100_000).is_err()); // outside any band
    }

    #[test]
    fn min_swr_point_picks_lowest() {
        let r = SwrSweepResult {
            start_hz: 14_000_000,
            stop_hz: 14_350_000,
            points: vec![
                SwrSweepPoint { frequency_hz: 14_000_000, swr: Some(3.1), forward_raw: None, reverse_raw: None },
                SwrSweepPoint { frequency_hz: 14_200_000, swr: Some(1.4), forward_raw: None, reverse_raw: None },
                SwrSweepPoint { frequency_hz: 14_300_000, swr: None, forward_raw: None, reverse_raw: None },
            ],
        };
        let p = r.min_swr_point().unwrap();
        assert_eq!(p.frequency_hz, 14_200_000);
        assert_eq!(p.swr, Some(1.4));
    }
}
