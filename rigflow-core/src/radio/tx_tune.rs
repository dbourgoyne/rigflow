/// Client-local arming/availability state for the TX tune test feature.
///
/// This enum is a UI-only concept. It is never serialised into protocol
/// messages or persisted to disk. The server communicates TX support through
/// `SourceCapabilities::supports_tx_tune_test`; the client derives its local
/// state from that flag plus the user's arm checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxTuneState {
    /// Hardware does not advertise TX tune test support.
    Unavailable,

    /// Supported but not yet armed by the operator.
    Disarmed,

    /// Armed: operator has enabled the arm checkbox.
    Armed,
}

impl Default for TxTuneState {
    fn default() -> Self {
        Self::Disarmed
    }
}

/// Machine-readable outcome of a TX tune test.
///
/// Carried in `TxTuneResult::status`. Variants are ordered from "no test"
/// through "success" to "fault" so comparisons are meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxTuneStatus {
    /// No test has been run yet (initial state).
    NotRun,
    /// Test completed normally; check power readings for detail.
    Ok,
    /// Test completed but forward power was below the minimum threshold
    /// (no measurable RF, or calibration not available).
    NoForwardPower,
    /// Test completed but measured SWR exceeded the safe threshold.
    HighSwr,
    /// Rejected before TX: hardware reported TX inhibited.
    TxInhibited,
    /// Rejected before TX: frequency is outside the hardware's TX range.
    InvalidFrequency,
    /// TX loop exceeded the hard timeout.
    Timeout,
    /// HL2 TX IQ FIFO underflow detected during TX.
    Underflow,
    /// HL2 TX IQ FIFO overflow detected during TX.
    Overflow,
    /// Unexpected socket or hardware fault.
    Fault,
}

impl Default for TxTuneStatus {
    fn default() -> Self {
        Self::NotRun
    }
}

/// Result of a TX tune test measurement.
///
/// Serialisable so it can be carried in `RuntimeChanged` / `RuntimeSnapshot`.
/// New scalar fields carry `#[serde(default)]` for wire compatibility with
/// older builds that do not emit them.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TxTuneResult {
    /// Machine-readable test outcome.
    #[serde(default)]
    pub status: TxTuneStatus,

    /// Forward power measured during the pulse (Watts).
    /// `None` when calibration constants are not yet available.
    pub forward_power_w: Option<f32>,

    /// Reverse power measured during the pulse (Watts).
    /// `None` when calibration constants are not yet available.
    pub reverse_power_w: Option<f32>,

    /// Standing-wave ratio derived from forward/reverse power.
    /// `None` when power readings are not available or are invalid.
    pub swr: Option<f32>,

    /// Peak raw forward-power detector count captured during the pulse
    /// (uncalibrated ADC units). `None` if not measured.
    #[serde(default)]
    pub forward_raw: Option<u16>,

    /// Peak raw reverse-power detector count captured during the pulse
    /// (uncalibrated ADC units). `None` if not measured.
    #[serde(default)]
    pub reverse_raw: Option<u16>,

    /// Peak raw PA-current detector count captured during the pulse
    /// (uncalibrated ADC units). `None` if not measured.
    #[serde(default)]
    pub current_raw: Option<u16>,

    /// Frequency the test was run at (Hz). Zero if no test has been run.
    #[serde(default)]
    pub frequency_hz: u64,

    /// Clamped duration actually used (ms). Zero if no test has been run.
    #[serde(default)]
    pub duration_ms: u32,

    /// Amplitude used, normalised to full-scale. Zero if no test has been run.
    #[serde(default)]
    pub drive: f32,

    /// Optional human-readable detail string.
    pub message: Option<String>,
}

/// Compute SWR from forward and reverse power (both in watts).
///
/// Returns `None` when:
/// - either power value is `None`
/// - forward power is below `MIN_FORWARD_W` (not transmitting / noise floor)
/// - reverse power is negative or ≥ forward power (invalid reading)
/// - result is non-finite
///
/// Formula: SWR = (1 + √(Pr/Pf)) / (1 − √(Pr/Pf))
pub fn compute_swr(forward_w: Option<f32>, reverse_w: Option<f32>) -> Option<f32> {
    let fwd = forward_w?;
    let rev = reverse_w?;

    const MIN_FORWARD_W: f32 = 0.1;
    if fwd < MIN_FORWARD_W || rev < 0.0 || rev >= fwd {
        return None;
    }

    let gamma = (rev / fwd).sqrt();
    let denominator = 1.0 - gamma;
    if denominator.abs() < f32::EPSILON {
        return None;
    }

    let swr = (1.0 + gamma) / denominator;
    if !swr.is_finite() {
        return None;
    }

    Some(swr.clamp(1.0, 999.0))
}

/// Compute SWR from peak raw forward/reverse detector counts captured during
/// a TX tune pulse (uncalibrated ADC units — no watts conversion needed since
/// SWR depends only on the *ratio* of reflected to forward).
///
/// `gamma = √(rev / fwd)`, `SWR = (1 + gamma) / (1 − gamma)`.
///
/// Returns `None` (SWR unavailable) when:
/// - `max_fwd_raw == 0` (no forward reading)
/// - `max_rev_raw > max_fwd_raw` (invalid: more reflected than forward)
/// - `gamma >= 1.0` (open/short — SWR would be infinite/invalid)
/// - the result is non-finite
pub fn compute_swr_from_raw(max_fwd_raw: u16, max_rev_raw: u16) -> Option<f32> {
    if max_fwd_raw == 0 || max_rev_raw > max_fwd_raw {
        return None;
    }

    let gamma = (max_rev_raw as f32 / max_fwd_raw as f32).sqrt();
    if !(gamma < 1.0) {
        // catches gamma >= 1.0 and NaN
        return None;
    }

    let swr = (1.0 + gamma) / (1.0 - gamma);
    if !swr.is_finite() {
        return None;
    }

    Some(swr.clamp(1.0, 999.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_swr ────────────────────────────────────────────────────────

    #[test]
    fn swr_matched_load() {
        // Pr = 0 W (perfect match) → SWR = 1.0
        let swr = compute_swr(Some(1.0), Some(0.0)).expect("should compute");
        assert!((swr - 1.0).abs() < 0.01, "expected 1.0, got {swr}");
    }

    #[test]
    fn swr_quarter_reflected() {
        // Pr/Pf = 0.25 → gamma = 0.5 → SWR = 1.5/0.5 = 3.0
        let swr = compute_swr(Some(1.0), Some(0.25)).expect("should compute");
        assert!((swr - 3.0).abs() < 0.01, "expected 3.0, got {swr}");
    }

    #[test]
    fn swr_none_inputs() {
        assert_eq!(compute_swr(None, None), None);
        assert_eq!(compute_swr(Some(1.0), None), None);
        assert_eq!(compute_swr(None, Some(0.5)), None);
    }

    #[test]
    fn swr_below_threshold() {
        // Forward power too low to be meaningful
        assert_eq!(compute_swr(Some(0.05), Some(0.01)), None);
    }

    #[test]
    fn swr_invalid_reverse() {
        // Reverse ≥ forward
        assert_eq!(compute_swr(Some(1.0), Some(1.5)), None);
        assert_eq!(compute_swr(Some(1.0), Some(1.0)), None);
        // Negative reverse
        assert_eq!(compute_swr(Some(1.0), Some(-0.1)), None);
    }

    // ── compute_swr_from_raw ───────────────────────────────────────────────

    #[test]
    fn swr_from_raw_matches_field_example() {
        // fwd=55, rev=12 → gamma=√(0.218)=0.467 → SWR≈2.75 (TinyVNA 2.58–2.87)
        let swr = compute_swr_from_raw(55, 12).expect("should compute");
        assert!((swr - 2.75).abs() < 0.05, "expected ~2.75, got {swr}");
    }

    #[test]
    fn swr_from_raw_perfect_match() {
        // rev=0 → gamma=0 → SWR=1.0
        let swr = compute_swr_from_raw(100, 0).expect("should compute");
        assert!((swr - 1.0).abs() < 0.001, "expected 1.0, got {swr}");
    }

    #[test]
    fn swr_from_raw_unavailable_cases() {
        assert_eq!(compute_swr_from_raw(0, 0), None); // no forward
        assert_eq!(compute_swr_from_raw(0, 5), None); // no forward
        assert_eq!(compute_swr_from_raw(10, 20), None); // rev > fwd
        assert_eq!(compute_swr_from_raw(10, 10), None); // gamma == 1.0
    }

    #[test]
    fn swr_clamped_high() {
        // Very high SWR (open circuit): Pr approaches Pf
        // gamma → 1, SWR → ∞, clamped at 999
        let swr = compute_swr(Some(1.0), Some(0.999)).expect("should compute");
        assert!(swr <= 999.0, "should be clamped; got {swr}");
        assert!(swr > 100.0, "should be very high; got {swr}");
    }

    // ── TxTuneStatus ──────────────────────────────────────────────────────

    #[test]
    fn status_default_is_not_run() {
        assert_eq!(TxTuneStatus::default(), TxTuneStatus::NotRun);
    }

    #[test]
    fn status_serde_roundtrip() {
        for status in [
            TxTuneStatus::NotRun,
            TxTuneStatus::Ok,
            TxTuneStatus::NoForwardPower,
            TxTuneStatus::HighSwr,
            TxTuneStatus::TxInhibited,
            TxTuneStatus::InvalidFrequency,
            TxTuneStatus::Timeout,
            TxTuneStatus::Underflow,
            TxTuneStatus::Overflow,
            TxTuneStatus::Fault,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let decoded: TxTuneStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, decoded, "serde roundtrip failed for {status:?}");
        }
    }

    // ── TxTuneResult ─────────────────────────────────────────────────────

    #[test]
    fn result_default_state() {
        let r = TxTuneResult::default();
        assert_eq!(r.status, TxTuneStatus::NotRun);
        assert_eq!(r.forward_power_w, None);
        assert_eq!(r.reverse_power_w, None);
        assert_eq!(r.swr, None);
        assert_eq!(r.frequency_hz, 0);
        assert_eq!(r.duration_ms, 0);
        assert_eq!(r.drive, 0.0);
        assert_eq!(r.message, None);
    }

    #[test]
    fn result_serde_roundtrip() {
        let original = TxTuneResult {
            status: TxTuneStatus::Ok,
            forward_power_w: Some(1.5),
            reverse_power_w: Some(0.1),
            swr: Some(1.8),
            forward_raw: Some(55),
            reverse_raw: Some(12),
            current_raw: Some(309),
            frequency_hz: 14_200_000,
            duration_ms: 250,
            drive: 0.05,
            message: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: TxTuneResult = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn result_serde_missing_new_fields() {
        // Old JSON without status/frequency_hz/duration_ms/drive should
        // deserialise to defaults for those fields.
        let old_json =
            r#"{"forward_power_w":null,"reverse_power_w":null,"swr":null,"message":null}"#;
        let decoded: TxTuneResult = serde_json::from_str(old_json).unwrap();
        assert_eq!(decoded.status, TxTuneStatus::NotRun);
        assert_eq!(decoded.frequency_hz, 0);
        assert_eq!(decoded.duration_ms, 0);
        assert_eq!(decoded.drive, 0.0);
    }
}
