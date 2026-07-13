//! Shared mouse interaction handler for the Spectrum and Waterfall views.
//!
//! Both views are synchronized windows onto the same radio data, so the mouse
//! behaviour must be identical.  Each view supplies its own pixel↔frequency
//! mapping (`freq_at_x`) and a click-sensing `Response`; this module turns the
//! raw input into a view-agnostic [`ViewMouseResult`] that the caller applies
//! through the normal tune/zoom paths.  Adding a future mouse gesture here makes
//! it work on both displays automatically.

use eframe::egui;

/// Minimum pointer speed at release (screen points/s) to start flick-momentum
/// panning.  Below this a release just stops the pan — no inertia.
const MIN_FLING_PX_PER_S: f32 = 150.0;

/// Outcome of one frame of mouse interaction over a Spectrum/Waterfall view.
/// All fields are inert (None/0/false) when there is nothing to do.
///
/// Wheel tune is reported as a *direction* + the *modifier* state rather than a
/// Hz delta: the step is the active Snap value scaled by the modifiers, and this
/// handler doesn't know the Snap value.  The caller resolves it via
/// [`crate::ui::tuning_steps::scaled_snap_step_hz`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewMouseResult {
    /// Single click: tune the target frequency to this absolute Hz.
    pub tune_to_hz: Option<f32>,
    /// `C` key pressed while the cursor is over the view: recenter the LO on the
    /// current target frequency and zero the LO offset.
    pub center_on_target: bool,
    /// Wheel tune direction: +1 = up, -1 = down, 0 = none/zoom.
    pub tune_dir: i32,
    /// Shift held during the wheel tune (×10 accelerate).
    pub tune_shift: bool,
    /// Alt held during the wheel tune (×0.1 decelerate).
    pub tune_alt: bool,
    /// Ctrl+wheel zoom: +1 = zoom in, -1 = zoom out, 0 = none.
    pub zoom_steps: i32,
    /// Horizontal click-drag this frame, converted to a target-frequency delta
    /// (Hz) via the view's local Hz-per-pixel.  Grab-and-slide: dragging right
    /// yields a negative delta (lower frequency).  0 when not dragging.
    pub drag_delta_hz: f32,
    /// Set on drag release to the fling velocity in Hz/s, to seed momentum
    /// panning.  `None` unless the release was a deliberate flick.
    pub fling_velocity_hz_per_s: Option<f32>,
}

/// Compute the shared mouse interaction for a view.
///
/// - `response` — the view's click-sensing response (gates on `hovered()` so an
///   unrelated scroll/keypress never tunes, and provides click + pointer).
/// - `rect` — the interactive area, for hit-testing clicks.
/// - `freq_at_x` — maps an absolute screen x within `rect` to a frequency.
///
/// Modifier priority for the wheel is **Ctrl → Shift → Alt → none** (Ctrl wins
/// even if other modifiers are also held).  Single click tunes; the `C` key
/// (while hovering) centers the LO on the current target.
pub fn handle_view_mouse(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    freq_at_x: impl Fn(f32) -> f32,
) -> ViewMouseResult {
    let mut result = ViewMouseResult::default();

    // --- Wheel: zoom (Ctrl) or modifier-scaled fine-tune -----------------
    if response.hovered() {
        // Read raw `MouseWheel` events rather than `raw_scroll_delta`: egui
        // rewrites Shift+wheel into *horizontal* scroll (delta.y → delta.x), so
        // `raw_scroll_delta.y` is 0 under Shift.  The event's `delta.y` is the
        // true vertical wheel motion, and its `modifiers` are accurate at
        // scroll time.
        let (wheel_y, mods) = ui.input(|i| {
            let mut y = 0.0;
            let mut mods = i.modifiers;
            for ev in &i.events {
                if let egui::Event::MouseWheel {
                    delta, modifiers, ..
                } = ev
                {
                    y += delta.y;
                    mods = *modifiers;
                }
            }
            (y, mods)
        });
        let dir = if wheel_y > 0.0 {
            1
        } else if wheel_y < 0.0 {
            -1
        } else {
            0
        };
        if dir != 0 {
            if mods.ctrl || mods.command {
                result.zoom_steps = dir;
            } else {
                result.tune_dir = dir;
                result.tune_shift = mods.shift;
                result.tune_alt = mods.alt;
            }
        }
    }

    // --- Single click → tune target to clicked frequency -----------------
    if response.clicked() {
        result.tune_to_hz = response
            .interact_pointer_pos()
            .filter(|p| rect.contains(*p))
            .map(|p| freq_at_x(p.x));
    }

    // --- Click-drag → pan the spectrum (grab-and-slide) ------------------
    // Convert horizontal drag to a frequency delta using the view's local
    // Hz-per-pixel (the `freq_at_x` map is linear across `rect`, so a 1 px
    // sample gives the slope).  Dragging right pulls the spectrum right,
    // revealing lower frequencies, so the delta is negated.  A drag is never a
    // click, so any click-tune the response also reported is suppressed.
    let hz_per_px = freq_at_x(rect.left() + 1.0) - freq_at_x(rect.left());
    if response.dragged() {
        result.drag_delta_hz = -response.drag_delta().x * hz_per_px;
        result.tune_to_hz = None;
    }
    if response.drag_stopped() {
        let vx = ui.input(|i| i.pointer.velocity().x);
        if vx.abs() > MIN_FLING_PX_PER_S {
            result.fling_velocity_hz_per_s = Some(-vx * hz_per_px);
        }
        result.tune_to_hz = None;
    }

    // --- `C` key (cursor over the view) → center LO on the target --------
    if response.hovered()
        && !ui.ctx().wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::C))
    {
        result.center_on_target = true;
    }

    result
}
