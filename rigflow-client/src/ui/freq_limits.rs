use rigflow_core::radio::source_control::DirectSamplingMode;

use crate::ui::state::UiState;

const DS_FREQ_HZ_MAX_FALLBACK: f32 = 30_000_000.0;

pub struct FreqLimits {
    pub center_min: f32,
    pub center_max: f32,
}

/// Compute the valid center-frequency range given the current source mode.
///
/// Returns `center_max == 0.0` when capabilities have not been received yet;
/// callers treat this as "no constraint".
pub fn active_freq_limits(state: &UiState) -> FreqLimits {
    let ds_active = state.source_control.direct_sampling != DirectSamplingMode::Off;

    if ds_active {
        let max = if state.source_capabilities.direct_sampling_freq_hz_max > 0 {
            state.source_capabilities.direct_sampling_freq_hz_max as f32
        } else {
            DS_FREQ_HZ_MAX_FALLBACK
        };
        FreqLimits {
            center_min: 0.0,
            center_max: max,
        }
    } else {
        FreqLimits {
            center_min: state.radio_capabilities.min_freq_hz as f32,
            center_max: state.radio_capabilities.max_freq_hz as f32,
        }
    }
}

/// Clamp a candidate center frequency to the active limits.
///
/// When `limits.center_max == 0.0` the capabilities are not yet known and
/// only a floor of 0 Hz is enforced.
pub fn clamp_center(hz: f32, limits: &FreqLimits) -> f32 {
    if limits.center_max == 0.0 {
        return hz.max(0.0);
    }
    hz.clamp(limits.center_min, limits.center_max)
}

/// Clamp a candidate target frequency.
///
/// Enforces two constraints in order:
/// 1. Target must stay within `center ± sample_rate / 2` (visible band).
/// 2. Target must fall within the active RF range.
pub fn clamp_target(
    target_hz: f32,
    center_hz: f32,
    sample_rate_hz: f32,
    limits: &FreqLimits,
) -> f32 {
    let half_bw = (sample_rate_hz / 2.0).max(0.0);
    let band_lo = (center_hz - half_bw).max(0.0);
    let band_hi = center_hz + half_bw;

    let clamped = target_hz.clamp(band_lo, band_hi);

    if limits.center_max == 0.0 {
        return clamped.max(0.0);
    }
    clamped.clamp(limits.center_min, limits.center_max)
}

/// Return `true` if `freq_hz` is within the active RF range.
///
/// Always returns `true` when capabilities have not yet been received
/// (`max_freq_hz == 0`), so the UI stays fully usable before radio acquire.
pub fn is_freq_valid(freq_hz: f32, state: &UiState) -> bool {
    let limits = active_freq_limits(state);
    if limits.center_max == 0.0 {
        return true;
    }
    freq_hz >= limits.center_min && freq_hz <= limits.center_max
}

/// Return a human-readable rejection message if `freq_hz` is outside the
/// active RF range, or `None` if the frequency is valid.
pub fn bookmark_rejection_message(freq_hz: f32, state: &UiState) -> Option<&'static str> {
    if is_freq_valid(freq_hz, state) {
        return None;
    }
    if state.source_control.direct_sampling != DirectSamplingMode::Off {
        Some("Bookmark is not valid in direct sampling mode")
    } else {
        Some("Bookmark is not valid in tuner mode")
    }
}
