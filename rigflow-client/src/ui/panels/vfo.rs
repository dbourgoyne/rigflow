//! Dual-VFO / split / RIT-XIT control panel.
//!
//! VFO A is tuned by the main spectrum/LO controls; this panel adds the
//! independent VFO B (frequency + mode), the split / TX-VFO selection, the
//! RIT/XIT offsets, A↔B swap / A=B copy, and the dual-watch toggle (grayed out
//! when the source has no second hardware receiver).  Every control sends the
//! matching `ClientRadioMessage`; local `UiState` is nudged for snappy feedback
//! (the server echo via `RuntimeChanged` then confirms it).

use eframe::egui;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::vfo::VfoSelect;
use rigflow_protocol::radio_control::ClientRadioMessage;

use crate::UiState;
use crate::ui::app::RigflowApp;
use crate::ui::freq_limits::{active_freq_limits, clamp_center, clamp_target};
use crate::ui::tuning_steps::scaled_snap_step_hz;

/// Modes offered for VFO B (same set the demod selector uses).
const VFO_MODES: [DemodMode; 8] = [
    DemodMode::Usb,
    DemodMode::Lsb,
    DemodMode::Cwu,
    DemodMode::Cwl,
    DemodMode::Am,
    DemodMode::Nfm,
    DemodMode::Wfm,
    DemodMode::DgtU,
];

fn mode_label(m: DemodMode) -> &'static str {
    match m {
        DemodMode::Usb => "USB",
        DemodMode::Lsb => "LSB",
        DemodMode::Cwu => "CW-U",
        DemodMode::Cwl => "CW-L",
        DemodMode::Am => "AM",
        DemodMode::Nfm => "NFM",
        DemodMode::Wfm => "WFM",
        DemodMode::DgtU => "DATA-U",
    }
}

impl RigflowApp {
    pub(crate) fn draw_vfo_panel(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        ui.collapsing(super::panel_header("Dual VFO / Split"), |ui| {
            let acquired = snapshot.radio_acquired;
            // The A/B frequency fields are a tuning path → also gated by the dial
            // lock (mode combos etc. stay on `acquired` only).
            let freq_editable = acquired && !snapshot.dial_locked;
            let vfo_a_hz = snapshot.target_freq_hz.max(0.0) as u64;
            let vfo_b_hz = snapshot.vfo_b_target_freq_hz.max(0.0) as u64;

            // ── VFO A / B frequency + mode ────────────────────────────────
            egui::Grid::new("vfo_ab_grid")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    // VFO A: editable frequency like VFO B (type an exact MHz
                    // value, or roll the wheel over the field — mode-aware steps).
                    // Mode stays a read-only label: VFO A's mode is set by the
                    // prominent demod selector in Radio Control.  An edit QSYs the
                    // LO window onto the new frequency (target == center), same as
                    // VFO B; the LO / spectrum controls remain the live source.
                    let a_tx = snapshot.split_enabled && snapshot.tx_vfo == VfoSelect::A;
                    ui.label(if a_tx { "A ▶TX" } else { "A" });
                    let mut a_hz = vfo_a_hz as i64;
                    let mut a_resp = ui.add_enabled(
                        freq_editable,
                        egui::DragValue::new(&mut a_hz)
                            .speed(100.0)
                            .range(0..=470_000_000i64)
                            .custom_formatter(|n, _| format!("{:.6}", n / 1_000_000.0))
                            .custom_parser(|s| s.parse::<f64>().ok().map(|mhz| mhz * 1_000_000.0))
                            .update_while_editing(false)
                            .suffix(" MHz"),
                    );
                    if freq_editable && a_resp.hovered() {
                        let raw_y = ui.input(|i| i.raw_scroll_delta.y);
                        if raw_y != 0.0 {
                            let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
                            let snap = snapshot.tuning_step_preferences.get(snapshot.demod_mode);
                            let step = scaled_snap_step_hz(snap, shift, alt);
                            let next =
                                (a_hz as f32 + step * raw_y.signum()).clamp(0.0, 470_000_000.0);
                            if next as i64 != a_hz {
                                a_hz = next as i64;
                                a_resp.mark_changed();
                            }
                        }
                        ui.ctx().input_mut(|i| {
                            i.raw_scroll_delta = egui::Vec2::ZERO;
                            i.smooth_scroll_delta = egui::Vec2::ZERO;
                        });
                    }
                    if a_resp.changed() {
                        self.set_vfo_a_freq_centered(a_hz.max(0) as u64);
                    }
                    ui.label(mode_label(snapshot.demod_mode));
                    ui.end_row();

                    // VFO B (editable).  Click to type an exact value in MHz, or
                    // roll the mouse wheel over the field to nudge by the active
                    // Snap value (Shift ×10, Alt ×0.1, else ×1; same model as the
                    // spectrum wheel / arrow keys).  The custom_parser reads the
                    // typed text as MHz; without it egui would treat "14.055" as 14 Hz.
                    let b_tx = snapshot.split_enabled && snapshot.tx_vfo == VfoSelect::B;
                    ui.label(if b_tx { "B ▶TX" } else { "B" });
                    let mut b_hz = vfo_b_hz as i64;
                    let mut resp = ui.add_enabled(
                        freq_editable,
                        egui::DragValue::new(&mut b_hz)
                            .speed(100.0)
                            .range(0..=470_000_000i64)
                            .custom_formatter(|n, _| format!("{:.6}", n / 1_000_000.0))
                            .custom_parser(|s| s.parse::<f64>().ok().map(|mhz| mhz * 1_000_000.0))
                            // Commit a typed value only on Enter / focus-loss, not on
                            // every keystroke — otherwise each intermediate value
                            // (e.g. a half-typed "7") would retune VFO B and bounce
                            // the amp band as you type.  Dragging still updates live.
                            .update_while_editing(false)
                            .suffix(" MHz"),
                    );
                    // Mouse-wheel-over-field nudge (and swallow the scroll so the
                    // side-panel ScrollArea doesn't move while the pointer is here).
                    if freq_editable && resp.hovered() {
                        let raw_y = ui.input(|i| i.raw_scroll_delta.y);
                        if raw_y != 0.0 {
                            let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
                            let step =
                                scaled_snap_step_hz(snapshot.vfo_b_tuning_step_hz, shift, alt);
                            let next =
                                (b_hz as f32 + step * raw_y.signum()).clamp(0.0, 470_000_000.0);
                            if next as i64 != b_hz {
                                b_hz = next as i64;
                                resp.mark_changed();
                            }
                        }
                        ui.ctx().input_mut(|i| {
                            i.raw_scroll_delta = egui::Vec2::ZERO;
                            i.smooth_scroll_delta = egui::Vec2::ZERO;
                        });
                    }
                    if resp.changed() {
                        self.set_vfo_b_freq_centered(b_hz.max(0) as u64);
                    }
                    // VFO B mode.
                    let mut b_mode = snapshot.vfo_b_demod_mode;
                    egui::ComboBox::from_id_salt("vfo_b_mode")
                        .selected_text(mode_label(b_mode))
                        .show_ui(ui, |ui| {
                            for m in VFO_MODES {
                                if ui.selectable_value(&mut b_mode, m, mode_label(m)).clicked() {
                                    self.apply_vfo_b_mode(m);
                                }
                            }
                        });
                    ui.end_row();
                });

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(acquired, egui::Button::new("Copy A=B  (=)"))
                    .on_hover_text(
                        "Clone VFO A's entire state (freq, mode, filter, DSP) onto VFO B",
                    )
                    .clicked()
                {
                    self.copy_a_to_b(snapshot);
                }
                if ui
                    .add_enabled(acquired, egui::Button::new("Swap TX  (X)"))
                    .on_hover_text(
                        "Swap TX focus between VFO A and B — frequencies stay put; only the \
                         TX VFO (badge) changes",
                    )
                    .clicked()
                {
                    self.swap_tx_focus(snapshot);
                }
            });

            ui.separator();

            // ── Split + TX VFO ────────────────────────────────────────────
            // Split + TX-VFO are wrong-frequency controls → gated by the global
            // settings lock (as well as needing an acquired radio).
            let mut split = snapshot.split_enabled;
            if ui
                .add_enabled(
                    acquired && !snapshot.config_locked,
                    egui::Checkbox::new(&mut split, "Split (transmit on the TX VFO)"),
                )
                .changed()
            {
                self.set_local(|s| s.split_enabled = split);
                self.send_radio_msg(ClientRadioMessage::SetSplit { enabled: split });
            }
            if split {
                ui.add_enabled_ui(!snapshot.config_locked, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("TX on:");
                        for (label, v) in [("A", VfoSelect::A), ("B", VfoSelect::B)] {
                            if ui.selectable_label(snapshot.tx_vfo == v, label).clicked() {
                                self.set_local(|s| s.tx_vfo = v);
                                self.send_radio_msg(ClientRadioMessage::SetTxVfo { vfo: v });
                            }
                        }
                    });
                });
            }

            ui.separator();

            // ── RIT / XIT ─────────────────────────────────────────────────
            // RIT is per-VFO: VFO A always shown; VFO B's own RIT appears under
            // dual-watch (offsets only VFO B's receive).
            self.draw_offset_row(
                ui,
                acquired,
                if snapshot.dual_watch_enabled {
                    "RIT A"
                } else {
                    "RIT"
                },
                snapshot.rit_enabled,
                snapshot.rit_offset_hz,
                |enabled, offset_hz| ClientRadioMessage::SetRit { enabled, offset_hz },
                |s, en, off| {
                    s.rit_enabled = en;
                    s.rit_offset_hz = off;
                },
            );
            if snapshot.dual_watch_enabled {
                self.draw_offset_row(
                    ui,
                    acquired,
                    "RIT B",
                    snapshot.vfo_b_rit_enabled,
                    snapshot.vfo_b_rit_offset_hz,
                    |enabled, offset_hz| ClientRadioMessage::SetVfoBRit { enabled, offset_hz },
                    |s, en, off| {
                        s.vfo_b_rit_enabled = en;
                        s.vfo_b_rit_offset_hz = off;
                    },
                );
            }
            self.draw_offset_row(
                ui,
                acquired,
                "XIT",
                snapshot.xit_enabled,
                snapshot.xit_offset_hz,
                |enabled, offset_hz| ClientRadioMessage::SetXit { enabled, offset_hz },
                |s, en, off| {
                    s.xit_enabled = en;
                    s.xit_offset_hz = off;
                },
            );

            ui.separator();

            // ── Dual-watch (second receiver) ──────────────────────────────
            let dual_watch_supported = snapshot.source_capabilities.supports_dual_watch;
            let mut dw = snapshot.dual_watch_enabled;
            let resp = ui
                .add_enabled(
                    acquired && dual_watch_supported,
                    egui::Checkbox::new(&mut dw, "Dual-watch — hear + show VFO B"),
                )
                .on_disabled_hover_text("Requires a multi-receiver Hermes Lite 2");
            if resp.changed() {
                self.set_local(|s| s.dual_watch_enabled = dw);
                self.send_radio_msg(ClientRadioMessage::SetDualWatch { enabled: dw });
            }
            if !dual_watch_supported {
                ui.label(
                    egui::RichText::new("Dual-watch needs an HL2 (second receiver).")
                        .small()
                        .weak(),
                );
            }
        });
    }

    /// One RIT/XIT row: an enable checkbox + a ± offset (Hz) drag.
    #[allow(clippy::too_many_arguments)]
    fn draw_offset_row(
        &self,
        ui: &mut egui::Ui,
        acquired: bool,
        label: &str,
        enabled: bool,
        offset_hz: i32,
        msg: impl Fn(bool, i32) -> ClientRadioMessage,
        apply_local: impl Fn(&mut UiState, bool, i32),
    ) {
        ui.horizontal(|ui| {
            let mut en = enabled;
            if ui
                .add_enabled(acquired, egui::Checkbox::new(&mut en, label))
                .changed()
            {
                self.set_local(|s| apply_local(s, en, offset_hz));
                self.send_radio_msg(msg(en, offset_hz));
            }
            let mut off = offset_hz;
            if ui
                .add_enabled(
                    acquired && en,
                    egui::DragValue::new(&mut off)
                        .speed(1.0)
                        .range(-9999..=9999)
                        .suffix(" Hz"),
                )
                .changed()
            {
                self.set_local(|s| apply_local(s, en, off));
                self.send_radio_msg(msg(en, off));
            }
            if en && off != 0 && ui.small_button("0").clicked() {
                self.set_local(|s| apply_local(s, en, 0));
                self.send_radio_msg(msg(en, 0));
            }
        });
    }

    /// Apply a VFO B mode change (mode + matching sideband) locally and to the server.
    fn apply_vfo_b_mode(&self, m: DemodMode) {
        let sb = match m {
            DemodMode::Lsb | DemodMode::Cwl => Sideband::Lsb,
            _ => Sideband::Usb,
        };
        self.set_local(|s| {
            s.vfo_b_demod_mode = m;
            s.vfo_b_sideband = sb;
        });
        self.send_radio_msg(ClientRadioMessage::SetVfoBDemodMode { mode: m });
        self.send_radio_msg(ClientRadioMessage::SetVfoBSideband { sideband: sb });
    }

    /// Set VFO A's frequency and centre its LO window on it (target == center),
    /// the same "QSY here" behaviour as [`Self::set_vfo_b_freq_centered`].  Used by
    /// the editable VFO-A field in the Dual-VFO panel; the LO / spectrum controls
    /// remain the live tuning source and pan / offset within the window from here.
    fn set_vfo_a_freq_centered(&self, hz: u64) {
        // Route through the freq_limits clamp helpers (CLAUDE.md: every tuning
        // path must) so a typed value can't drive the LO out of the radio's RF
        // range.  QSY-here → centre == target == the clamped frequency.
        let Some((center, target)) = self.clamped_centered_freq(hz) else {
            return;
        };
        self.set_local(|s| {
            s.center_freq_hz = center;
            s.target_freq_hz = target;
        });
        self.send_radio_msg(ClientRadioMessage::SetCenterFrequency {
            center_freq_hz: center as u64,
        });
        self.send_radio_msg(ClientRadioMessage::SetTargetFrequency {
            target_freq_hz: target as u64,
        });
    }

    /// Set VFO B's frequency and centre its LO window on it (RX1 NCO = target),
    /// so the tuned freq is always inside the window — the "QSY here" behaviour.
    /// Clamped to the active RF range via the freq_limits helpers.
    fn set_vfo_b_freq_centered(&self, hz: u64) {
        let Some((center, target)) = self.clamped_centered_freq(hz) else {
            return;
        };
        self.set_local(|s| {
            s.vfo_b_center_freq_hz = center;
            s.vfo_b_target_freq_hz = target;
        });
        self.send_radio_msg(ClientRadioMessage::SetVfoBCenterFrequency {
            center_freq_hz: center as u64,
        });
        self.send_radio_msg(ClientRadioMessage::SetVfoBFrequency {
            target_freq_hz: target as u64,
        });
    }

    /// Clamp a "QSY here" frequency to the active RF range (and the visible band)
    /// using the shared `freq_limits` helpers.  Returns `(center, target)` with
    /// `center == target == clamped freq`, or `None` if state can't be locked.
    fn clamped_centered_freq(&self, hz: u64) -> Option<(f32, f32)> {
        let s = self.state.lock().ok()?;
        let limits = active_freq_limits(&s);
        let center = clamp_center(hz as f32, &limits);
        let target = clamp_target(center, center, s.input_sample_rate_hz, &limits);
        Some((center, target))
    }

    /// VFO Copy (A=B): clone VFO A's entire receiver state onto VFO B.  The server
    /// does the wholesale `VfoState` clone (`CopyVfoAToB`); we also mirror every
    /// VFO-B field locally for instant feedback (incl. the fire-and-forget nb/notch
    /// and the client-only volume, which the server echo does not carry).
    pub(crate) fn copy_a_to_b(&self, snapshot: &UiState) {
        self.set_local(|s| {
            s.vfo_b_target_freq_hz = s.target_freq_hz;
            s.vfo_b_center_freq_hz = s.center_freq_hz;
            s.vfo_b_demod_mode = s.demod_mode;
            s.vfo_b_sideband = s.sideband;
            s.vfo_b_filter_bandwidth_hz = s.filter_bandwidth_hz;
            // VFO A keeps a single `pitch_hz` for its current mode; route it into
            // VFO B's matching slot, mirroring the per-mode default into the other.
            let prefs = s.demod_preferences.get(s.demod_mode);
            match s.demod_mode {
                DemodMode::Cwu | DemodMode::Cwl => {
                    s.vfo_b_cw_pitch_hz = s.pitch_hz;
                    s.vfo_b_ssb_pitch_hz = prefs.pitch_hz;
                }
                _ => {
                    s.vfo_b_ssb_pitch_hz = s.pitch_hz;
                    s.vfo_b_cw_pitch_hz = prefs.pitch_hz;
                }
            }
            s.vfo_b_deemphasis_mode = s.deemphasis_mode;
            // Grid-snap step: copy VFO A's current per-mode step to VFO B's
            // independent (session) step.
            s.vfo_b_tuning_step_hz = s.tuning_step_preferences.get(s.demod_mode);
            s.vfo_b_squelch_enabled = s.squelch_enabled;
            s.vfo_b_squelch_threshold_db = s.squelch_threshold_db;
            s.vfo_b_nr2_enabled = s.nr2_enabled;
            s.vfo_b_nr2_strength = s.nr2_strength;
            s.vfo_b_nb_enabled = s.nb_enabled;
            s.vfo_b_nb_threshold = s.nb_threshold;
            s.vfo_b_notch_auto_enabled = s.notch_auto_enabled;
            s.vfo_b_agc_enabled = s.agc_enabled;
            s.vfo_b_agc_strength = s.agc_strength;
            s.vfo_b_rit_enabled = s.rit_enabled;
            s.vfo_b_rit_offset_hz = s.rit_offset_hz;
            s.volume_percent_b = s.volume_percent;
            // Waterfall display settings (client-side) — copy so B's panadapter
            // looks identical to A's; the adaptive estimates seed B before it
            // re-adapts to its own spectrum.
            s.vfo_b_display_zoom = s.display_zoom;
            s.vfo_b_adaptive_waterfall_normalization = s.adaptive_waterfall_normalization;
            s.vfo_b_manual_waterfall_top_db = s.manual_waterfall_top_db;
            s.vfo_b_manual_waterfall_range_db = s.manual_waterfall_range_db;
            s.vfo_b_adaptive_top_db_estimate = s.adaptive_top_db_estimate;
            s.vfo_b_adaptive_floor_db_estimate = s.adaptive_floor_db_estimate;
            s.vfo_b_adaptive_range_db_estimate = s.adaptive_range_db_estimate;
            s.vfo_b_waterfall_frame_rate_hz = s.waterfall_frame_rate_hz;
        });
        let rate_hz = snapshot.waterfall_frame_rate_hz;
        self.send_radio_msg(ClientRadioMessage::CopyVfoAToB);
        // Frame rate is server-side (paces the DSP-B waterfall thread); CopyVfoAToB
        // only clones the receiver VfoState, so sync B's rate explicitly.
        self.send_radio_msg(ClientRadioMessage::SetVfoBWaterfallFrameRate { rate_hz });
    }

    /// TX Focus Swap: flip which VFO transmits (and turn split on so the choice is
    /// honoured).  Deliberately lightweight — VFO frequencies and the waterfall
    /// layout stay anchored; only the TX VFO (and its badge) changes.
    pub(crate) fn swap_tx_focus(&self, snapshot: &UiState) {
        let new_tx = snapshot.tx_vfo.other();
        self.set_local(|s| {
            s.tx_vfo = new_tx;
            s.split_enabled = true;
        });
        self.send_radio_msg(ClientRadioMessage::SetTxVfo { vfo: new_tx });
        self.send_radio_msg(ClientRadioMessage::SetSplit { enabled: true });
    }

    /// Briefly lock `UiState` to apply a local edit for snappy feedback.
    fn set_local(&self, f: impl FnOnce(&mut UiState)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
    }
}
