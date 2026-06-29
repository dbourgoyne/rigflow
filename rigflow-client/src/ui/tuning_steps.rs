//! Mode-aware tuning step sizes, shared by the mouse wheel and the arrow keys.
//!
//! Both the wheel applier (`app_center::apply_view_interaction`) and the keyboard
//! handler (`app::handle_keyboard_shortcuts`) resolve their step from here, so the
//! two input paths can never drift apart again.  Steps scale with the current
//! [`DemodMode`], which 1:1 covers the bands we care about: `Wfm` = FM broadcast,
//! `Am` = AM broadcast / airband, SSB/CW = HF.  (FM broadcast is only reachable on
//! RTL-SDR + direct sampling — the HL2 caps at 30 MHz — so the large WFM steps are
//! safe everywhere they apply.)
//!
//! All values are named consts grouped by tier so they are trivial to tweak.

use rigflow_core::dsp::modes::DemodMode;

/// The fixed set of grid-snap / tuning-step sizes (Hz) offered in the LO-strip
/// "Snap" dropdown.  These govern every target tuning action (wheel, spectrum/
/// waterfall click, ←/→ arrows) — purely client-side; the server still receives
/// the resulting Hz integer.
pub const TUNING_STEP_OPTIONS_HZ: [f32; 8] =
    [1.0, 10.0, 50.0, 100.0, 500.0, 1_000.0, 5_000.0, 10_000.0];

// Sensible per-mode defaults (SSB 1 kHz, CW 50 Hz, AM/NFM 5 kHz, Digital 1 Hz,
// WFM 10 kHz) live in `persistence::models::TuningStepPreferencesFile::default`
// (persistence can't depend on this UI module).

/// Snap a frequency (Hz) to the nearest multiple of `step_hz` (the grid).  A
/// non-positive step disables snapping (returns the input unchanged).
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

/// Which step tier the modifier keys selected.
///
/// `Fine` = no modifier, `Medium` = Shift, `Coarse` = Alt.  Applies to *target*
/// tuning (wheel + ←/→).  Center tuning (↑/↓) only distinguishes Shift, so it
/// takes a plain `bool` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuneTier {
    #[default]
    Fine,
    Medium,
    Coarse,
}

// --- Target step table (wheel + ←/→) ------------------------------------------
// Rows: CW/SSB/Data, AM, NFM, WFM.  Columns: Fine / Medium / Coarse.

const TARGET_SSB_FINE: f32 = 10.0;
const TARGET_SSB_MEDIUM: f32 = 100.0;
const TARGET_SSB_COARSE: f32 = 1_000.0;

const TARGET_AM_FINE: f32 = 100.0;
const TARGET_AM_MEDIUM: f32 = 1_000.0;
const TARGET_AM_COARSE: f32 = 10_000.0; // US AM broadcast channel spacing

const TARGET_NFM_FINE: f32 = 1_000.0;
const TARGET_NFM_MEDIUM: f32 = 5_000.0;
const TARGET_NFM_COARSE: f32 = 25_000.0; // land-mobile channel spacing

const TARGET_WFM_FINE: f32 = 10_000.0;
const TARGET_WFM_MEDIUM: f32 = 100_000.0; // EU FM broadcast channel spacing
const TARGET_WFM_COARSE: f32 = 1_000_000.0;

// --- Center / LO step table (↑/↓) ---------------------------------------------
// Columns: none / Shift.

const CENTER_SSB_FINE: f32 = 1_000.0;
const CENTER_SSB_SHIFT: f32 = 25_000.0;

const CENTER_AM_FINE: f32 = 10_000.0;
const CENTER_AM_SHIFT: f32 = 100_000.0;

const CENTER_NFM_FINE: f32 = 25_000.0;
const CENTER_NFM_SHIFT: f32 = 250_000.0;

const CENTER_WFM_FINE: f32 = 200_000.0; // US FM broadcast channel spacing
const CENTER_WFM_SHIFT: f32 = 2_000_000.0;

/// Group a mode into one of the four step families.  CWU/CWL, USB/LSB and DgtU
/// all behave identically here.
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

/// Target-frequency step (Hz) for one wheel notch / arrow press, by mode + tier.
pub fn target_step_hz(mode: DemodMode, tier: TuneTier) -> f32 {
    match (family(mode), tier) {
        (Family::Ssb, TuneTier::Fine) => TARGET_SSB_FINE,
        (Family::Ssb, TuneTier::Medium) => TARGET_SSB_MEDIUM,
        (Family::Ssb, TuneTier::Coarse) => TARGET_SSB_COARSE,
        (Family::Am, TuneTier::Fine) => TARGET_AM_FINE,
        (Family::Am, TuneTier::Medium) => TARGET_AM_MEDIUM,
        (Family::Am, TuneTier::Coarse) => TARGET_AM_COARSE,
        (Family::Nfm, TuneTier::Fine) => TARGET_NFM_FINE,
        (Family::Nfm, TuneTier::Medium) => TARGET_NFM_MEDIUM,
        (Family::Nfm, TuneTier::Coarse) => TARGET_NFM_COARSE,
        (Family::Wfm, TuneTier::Fine) => TARGET_WFM_FINE,
        (Family::Wfm, TuneTier::Medium) => TARGET_WFM_MEDIUM,
        (Family::Wfm, TuneTier::Coarse) => TARGET_WFM_COARSE,
    }
}

/// Center/LO step (Hz) for one ↑/↓ press, by mode.  `shift` selects the coarse
/// column.
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
