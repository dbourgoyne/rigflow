//! Tuning-step model for the target dial and the LO.
//!
//! **Target tuning** (mouse wheel, ←/→, and the Dual-VFO frequency fields) moves
//! by a *relative* multiple of the currently selected UI "Snap" value (the
//! LO-strip dropdown), scaled by the modifier keys with a factor-of-10 model:
//!
//! - **no modifier** → ×1 the active Snap value,
//! - **Shift** → ×10 (accelerate),
//! - **Alt** → ×0.1 (decelerate),
//!
//! clamped to a 1 Hz floor — see [`scaled_snap_step_hz`].
//!
//! **LO / centre tuning** (↑/↓) is deliberately coarser and mode-appropriate so
//! the whole display window can be swept across a band quickly — see
//! [`center_step_hz`] (Shift selects the coarse column).

use rigflow_core::dsp::modes::DemodMode;

/// The fixed set of grid-snap / tuning-step sizes (Hz) offered in the LO-strip
/// "Snap" dropdown.  This is the ×1 base for every tuning action (wheel,
/// spectrum/waterfall click, ←/→, ↑/↓) — purely client-side; the server still
/// receives the resulting Hz integer.
pub const TUNING_STEP_OPTIONS_HZ: [f32; 8] =
    [1.0, 10.0, 50.0, 100.0, 500.0, 1_000.0, 5_000.0, 10_000.0];

// Sensible per-mode defaults (SSB 1 kHz, CW 50 Hz, AM/NFM 5 kHz, Digital 1 Hz,
// WFM 10 kHz) live in `persistence::models::TuningStepPreferencesFile::default`
// (persistence can't depend on this UI module).

/// Snap a frequency (Hz) to the nearest multiple of `step_hz` (the grid).  A
/// non-positive step disables snapping (returns the input unchanged).  Used to
/// grid-align a *click*-to-tune; incremental tuning is relative (see
/// [`scaled_snap_step_hz`]).
pub fn snap_to_step_hz(hz: f32, step_hz: f32) -> f32 {
    if step_hz <= 0.0 {
        hz
    } else {
        (hz / step_hz).round() * step_hz
    }
}

/// Human label for a step size, e.g. `1 Hz`, `500 Hz`, `1 kHz`, `10 kHz`.
pub fn tuning_step_label(step_hz: f32) -> String {
    let hz = step_hz.round() as i64;
    if hz >= 1_000 && hz % 1_000 == 0 {
        format!("{} kHz", hz / 1_000)
    } else {
        format!("{hz} Hz")
    }
}

/// Relative modifier multiplier shared by ALL tuning inputs (mouse wheel, ←/→,
/// ↑/↓): **Shift = ×10** (accelerate), **Alt = ×0.1** (decelerate), neither =
/// ×1.  Shift wins if both are held.
pub fn snap_multiplier(shift: bool, alt: bool) -> f32 {
    if shift {
        10.0
    } else if alt {
        0.1
    } else {
        1.0
    }
}

/// The tuning step (Hz) for one input event: the active Snap value scaled by the
/// modifier multiplier ([`snap_multiplier`]), clamped to a **1 Hz floor** so
/// Alt-decelerated tuning never attempts a fractional-Hz step.
pub fn scaled_snap_step_hz(snap_hz: f32, shift: bool, alt: bool) -> f32 {
    (snap_hz * snap_multiplier(shift, alt)).max(1.0)
}

// --- Centre / LO step table (↑/↓) --------------------------------------------
// The LO step is coarse + mode-appropriate (not the Snap value) so the whole
// window can be swept across a band quickly.  Columns: none / Shift.

const CENTER_SSB_FINE: f32 = 1_000.0;
const CENTER_SSB_SHIFT: f32 = 25_000.0;

const CENTER_AM_FINE: f32 = 10_000.0;
const CENTER_AM_SHIFT: f32 = 100_000.0;

const CENTER_NFM_FINE: f32 = 25_000.0;
const CENTER_NFM_SHIFT: f32 = 250_000.0;

const CENTER_WFM_FINE: f32 = 200_000.0; // US FM broadcast channel spacing
const CENTER_WFM_SHIFT: f32 = 2_000_000.0;

/// Group a mode into one of the four LO-step families.  CWU/CWL, USB/LSB and
/// DgtU all behave identically here.
enum Family {
    Ssb,
    Am,
    Nfm,
    Wfm,
}

fn family(mode: DemodMode) -> Family {
    match mode {
        DemodMode::Cwu | DemodMode::Cwl | DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU => {
            Family::Ssb
        }
        DemodMode::Am => Family::Am,
        DemodMode::Nfm => Family::Nfm,
        DemodMode::Wfm => Family::Wfm,
    }
}

/// Centre/LO step (Hz) for one ↑/↓ press, by mode.  `shift` selects the coarse
/// column.  Independent of the Snap value (which governs target tuning only).
pub fn center_step_hz(mode: DemodMode, shift: bool) -> f32 {
    match (family(mode), shift) {
        (Family::Ssb, false) => CENTER_SSB_FINE,
        (Family::Ssb, true) => CENTER_SSB_SHIFT,
        (Family::Am, false) => CENTER_AM_FINE,
        (Family::Am, true) => CENTER_AM_SHIFT,
        (Family::Nfm, false) => CENTER_NFM_FINE,
        (Family::Nfm, true) => CENTER_NFM_SHIFT,
        (Family::Wfm, false) => CENTER_WFM_FINE,
        (Family::Wfm, true) => CENTER_WFM_SHIFT,
    }
}
