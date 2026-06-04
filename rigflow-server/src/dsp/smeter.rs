//! S-meter: convert pre-demod channel power to dBm and S-units.
//!
//! Source- and mode-independent. The power is measured on the channelized
//! (post-channel-filter, pre-demod) IQ in [`crate::dsp::pipeline`], so it does
//! not depend on demod mode, AGC, NR2, squelch, or audio level.

/// Reference: S9 = -73 dBm on HF (IARU Region 1 convention).
pub const S9_DBM: f32 = -73.0;
/// One S-unit = 6 dB.
pub const DB_PER_S_UNIT: f32 = 6.0;

/// Maps full-scale channel power (|z|^2 == 1.0, i.e. 0 dBFS) to this dBm value.
///
/// This is an UNCALIBRATED placeholder: the float IQ is normalized to
/// full-scale ±1.0, so `power` is in dBFS-relative units, not absolute power.
/// Absolute dBm depends on source/front-end gain (RTL gain table, HL2 LNA),
/// which we do not yet have a calibration table for.  `dbm = dBFS + OFFSET`
/// gives a stable RELATIVE reading; replace this offset (ideally per-source,
/// per-gain) once a real calibration exists.
pub const FULL_SCALE_DBM: f32 = -30.0;

/// Lower display clamp (no negative S-units / silly values for the noise floor).
pub const MIN_DBM: f32 = -140.0;

/// Convert mean channel power (normalized full-scale units) to (relative) dBm.
pub fn channel_power_to_dbm(power: f32) -> f32 {
    // 10*log10(power) is dBFS (power == 1.0 → 0 dBFS); add the full-scale dBm
    // offset.  Guard log10(0).
    let dbfs = 10.0 * (power.max(1e-20)).log10();
    (dbfs + FULL_SCALE_DBM).max(MIN_DBM)
}

/// Convert dBm to an integer S-unit in 0..=9 (clamped; values above S9 still
/// report 9 — the caller can show "S9+N dB" from the dBm value).
pub fn dbm_to_s_units(dbm: f32) -> i32 {
    let s = 9.0 + (dbm - S9_DBM) / DB_PER_S_UNIT;
    (s.round() as i32).clamp(0, 9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s_unit_reference_points() {
        // From the standard table: S9=-73, S8=-79, S7=-85, S6=-91.
        assert_eq!(dbm_to_s_units(-73.0), 9);
        assert_eq!(dbm_to_s_units(-79.0), 8);
        assert_eq!(dbm_to_s_units(-85.0), 7);
        assert_eq!(dbm_to_s_units(-91.0), 6);
    }

    #[test]
    fn s_unit_clamps() {
        assert_eq!(dbm_to_s_units(-50.0), 9); // above S9 clamps to 9
        assert_eq!(dbm_to_s_units(-200.0), 0); // far below S1 → S0
    }

    #[test]
    fn dbm_is_finite_and_monotonic() {
        let weak = channel_power_to_dbm(1e-9);
        let strong = channel_power_to_dbm(0.5);
        assert!(weak.is_finite() && strong.is_finite());
        assert!(strong > weak);
        // Zero power is guarded (no -inf), clamped at MIN_DBM.
        assert_eq!(channel_power_to_dbm(0.0), MIN_DBM);
    }

    #[test]
    fn full_scale_maps_to_offset() {
        assert!((channel_power_to_dbm(1.0) - FULL_SCALE_DBM).abs() < 1e-3);
    }
}
