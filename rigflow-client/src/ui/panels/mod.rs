use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

/// Styling for a **top-level** left-panel section header (Radio Operator,
/// Radios, Radio Control, …).  Larger + the operator-name accent colour so the
/// main menus stand out from their sub-sections (which keep the default style).
pub(crate) fn panel_header(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(16.0)
        .strong()
        .color(egui::Color32::from_rgb(90, 200, 255))
}

/// Soft-amber, body-size styling for explanatory note / help captions — the short
/// sentences that explain a control or panel. Readable and visually distinct from
/// white control labels, replacing the old hard-to-read `.small().weak()` styling.
pub(crate) fn note_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(egui::Color32::from_rgb(230, 200, 120))
}

/// True once the pointer has *rested* on the widget `id` for the dwell period.
///
/// This distinguishes "parked on a control to wheel-adjust it" from "a control
/// scrolled under the pointer during a panel scroll": in the latter case each
/// control is under the cursor only briefly (the content is moving), so it never
/// dwells and the wheel falls through to the surrounding `ScrollArea`.  Shared by
/// [`slider_scroll`] and the Dual-VFO frequency-field wheel nudges, all of which
/// funnel the one hovered control through a single tracked (id, first-hover-time)
/// slot in egui memory.
pub(crate) fn wheel_dwell_ready(ui: &egui::Ui, id: egui::Id) -> bool {
    const DWELL_SECS: f64 = 0.2;
    // While the left panel is actively scrolling, NOTHING grabs the wheel — so a
    // multi-notch scroll can't be hijacked by a slider that happens to linger
    // under the pointer.  Cooldown outlives the gaps between notches.
    const PANEL_SCROLL_COOLDOWN_SECS: f64 = 0.35;
    let now = ui.input(|i| i.time);
    let scrolled_at = ui
        .ctx()
        .data(|d| d.get_temp::<f64>(egui::Id::new("panel_scroll_at")))
        .unwrap_or(f64::NEG_INFINITY);
    if now - scrolled_at < PANEL_SCROLL_COOLDOWN_SECS {
        return false;
    }
    let age = ui.ctx().data_mut(|d| {
        let e: &mut (egui::Id, f64) =
            d.get_temp_mut_or_insert_with(egui::Id::new("wheel_dwell"), || (id, now));
        if e.0 != id {
            *e = (id, now);
        }
        now - e.1
    });
    age >= DWELL_SECS
}

/// Record left-panel scroll activity: if the `ScrollArea` offset moved since the
/// last frame, stamp "now" so [`wheel_dwell_ready`] suppresses wheel-grab for a
/// short cooldown.  Call once per frame with the panel ScrollArea's offset.
pub(crate) fn note_panel_scroll(ui: &egui::Ui, offset: egui::Vec2) {
    let now = ui.input(|i| i.time);
    ui.ctx().data_mut(|d| {
        let prev = d
            .get_temp::<egui::Vec2>(egui::Id::new("panel_scroll_prev"))
            .unwrap_or(offset);
        if (prev - offset).length() > 0.5 {
            d.insert_temp(egui::Id::new("panel_scroll_at"), now);
        }
        d.insert_temp(egui::Id::new("panel_scroll_prev"), offset);
    });
}

/// Let the mouse wheel adjust a slider while the pointer is over it: scroll up
/// increases, scroll down decreases, by `step` (clamped to `min..=max`). Marks the
/// response changed so the caller's existing `.changed()` handling runs. Returns
/// `true` if the wheel changed the value (useful when the caller persists on
/// `drag_stopped()` rather than `changed()`). Call right after
/// `ui.add(Slider::new(&mut value, min..=max)…)`.
///
/// Wheel-adjust only engages once the pointer has *dwelled* on the slider (see
/// [`wheel_dwell_ready`]) so a slider that scrolls under the pointer mid-panel-
/// scroll isn't grabbed; until then the scroll passes through to the ScrollArea.
///
/// While the pointer is over the slider this **swallows all wheel scroll** so the
/// surrounding panel `ScrollArea` never scrolls and the slider stays under the
/// cursor. The step is taken from `raw_scroll_delta` (one notch per frame), but
/// both the raw and the multi-frame `smooth_scroll_delta` (which the `ScrollArea`
/// actually consumes) are zeroed every hovered frame — otherwise the smoothed tail
/// of a notch keeps scrolling the panel on the frames after the step.
pub(crate) fn slider_scroll<Num: egui::emath::Numeric>(
    ui: &egui::Ui,
    response: &mut egui::Response,
    value: &mut Num,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    if !response.hovered() {
        return false;
    }
    // Not armed until the pointer has rested here — let the wheel scroll the
    // panel (don't swallow) while a slider merely passes under the cursor.
    if !wheel_dwell_ready(ui, response.id) {
        return false;
    }

    // Step the value once per wheel notch (raw delta is per-frame).
    let raw_y = ui.input(|i| i.raw_scroll_delta.y);
    let mut changed = false;
    if raw_y != 0.0 {
        let (lo, hi) = (min.min(max), min.max(max));
        let cur = (*value).to_f64();
        let next = (cur + step * raw_y.signum() as f64).clamp(lo, hi);
        if next != cur {
            *value = Num::from_f64(next);
            response.mark_changed();
            changed = true;
        }
    }

    // Swallow wheel scroll for as long as the slider is hovered — including the
    // smoothed tail on frames where `raw_scroll_delta` is already zero — so the
    // panel ScrollArea (which reads `smooth_scroll_delta`) never moves.
    ui.ctx().input_mut(|i| {
        i.raw_scroll_delta = egui::Vec2::ZERO;
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });

    changed
}

/// A small padlock toggle for "accidental-change" locks.  Shows 🔒 when locked,
/// 🔓 when unlocked; clicking flips `*locked`.  Returns `true` only on the click
/// that *unlocked* it, so the caller can stamp an unlock time for auto-re-lock.
///
/// (Glyph note: the lock emoji comes from egui's bundled emoji font — if it ever
/// renders as tofu, swap for a drawn icon or `[L]`/`[U]` text.)
/// Padlock glyph sizes: small for the inline per-slider damage locks, larger for
/// the prominent global toggles (settings lock + dial lock).
pub(crate) const LOCK_SMALL: f32 = 12.0;
pub(crate) const LOCK_LARGE: f32 = 18.0;

pub(crate) fn lock_button(ui: &mut egui::Ui, locked: &mut bool, size: f32) -> bool {
    let (glyph, hover) = if *locked {
        ("🔒", "Locked — click to unlock")
    } else {
        ("🔓", "Unlocked — click to lock")
    };
    if ui
        .add(egui::Button::new(egui::RichText::new(glyph).size(size)))
        .on_hover_text(hover)
        .clicked()
    {
        *locked = !*locked;
        return !*locked;
    }
    false
}

mod bookmarks;
mod latency;
mod operator;
mod problems;
mod radio_control;
mod radios;
mod sections;

/// Shared S-meter label formatter (also used by the top status bar).
pub(crate) use radio_control::s_meter_label;
mod server;
mod source_control;
mod source_status;
mod tx_tune_test;
mod vfo;
mod waterfall_control;

impl RigflowApp {
    pub(crate) fn draw_left_panel(
        &mut self,
        ctx: &egui::Context,
        snapshot: &UiState,
        config_mode: bool,
    ) {
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                // Global settings lock — gates the set-once / wrong-frequency
                // config controls (they grey out while locked).  The damage
                // controls (TX Drive, Spot Level) have their own separate inline
                // locks and are NOT affected by this one.
                let mut config_locked = snapshot.config_locked;
                ui.horizontal(|ui| {
                    lock_button(ui, &mut config_locked, LOCK_LARGE);
                    ui.label(if config_locked {
                        "Settings locked"
                    } else {
                        "Settings unlocked"
                    });
                });
                if config_locked != snapshot.config_locked {
                    if let Ok(mut s) = self.state.lock() {
                        s.config_locked = config_locked;
                    }
                    self.save_config_lock_to_current_operator();
                }
                ui.separator();

                // Status console docked at the bottom of the side panel (fixed
                // height, internally scrollable).  Reserved here, before the
                // settings scroll area, so that area fills the space above it.
                let row_h = ui.text_style_height(&egui::TextStyle::Body);
                egui::TopBottomPanel::bottom("status_console")
                    .resizable(false)
                    .exact_height(row_h * 5.0 + 10.0)
                    .show_inside(ui, |ui| self.draw_status_console(ui, snapshot));

                let scroll_out = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // First-run / not-connected cue: point a new user at the
                        // two steps to get on the air (the panels below are open
                        // pre-connect).  Hidden once connected.
                        if !snapshot.server_connected {
                            let cue = if snapshot.known_operator_ids.is_empty() {
                                "Getting started: add an operator below, then enter your server IP and Connect."
                            } else {
                                "Not connected — enter your server IP and click Connect."
                            };
                            ui.colored_label(egui::Color32::from_rgb(255, 190, 70), cue);
                            ui.separator();
                        }

                        self.draw_operator_panel(ui, snapshot, config_mode);
                        ui.separator();
                        self.draw_server_panel(ui, snapshot, config_mode);
                        ui.separator();
                        self.draw_radios_panel(ui, snapshot);
                        self.draw_radio_control_panel(ui, snapshot);
                        ui.separator();
                        // "Dual VFO / Split" only applies to a rig with a second
                        // receiver (dual-watch) or transmit (split) — hidden for
                        // receive-only single-receiver sources (RTL-SDR, WAV, …).
                        if snapshot.source_capabilities.supports_dual_watch
                            || snapshot.source_capabilities.supports_transmit
                        {
                            self.draw_vfo_panel(ui, snapshot);
                            ui.separator();
                        }
                        self.draw_source_control_panel(ui, snapshot);
                        ui.separator();
                        self.draw_waterfall_control_panel(ui);
                        ui.separator();
                        self.draw_latency_panel(ui, snapshot);
                        ui.separator();
                        self.draw_bookmarks_panel(ui);
                        ui.separator();
                    });
                // Feed the scroll offset to the wheel-dwell logic so controls
                // don't grab the wheel while the panel is being scrolled.
                note_panel_scroll(ui, scroll_out.state.offset);
            });
    }
}
