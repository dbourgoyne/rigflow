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
use crate::ui::tuning_steps::{TuneTier, target_step_hz};

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

fn fmt_mhz(hz: u64) -> String {
    format!("{:.6} MHz", hz as f64 / 1_000_000.0)
}

impl RigflowApp {
    pub(crate) fn draw_vfo_panel(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        ui.collapsing(super::panel_header("Dual VFO / Split"), |ui| {
            let acquired = snapshot.radio_acquired;
            let vfo_a_hz = snapshot.target_freq_hz.max(0.0) as u64;
            let vfo_b_hz = snapshot.vfo_b_target_freq_hz.max(0.0) as u64;

            // ── VFO A / B frequency + mode ────────────────────────────────
            egui::Grid::new("vfo_ab_grid")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    // VFO A (read-only here; tuned by the spectrum/LO).
                    let a_tx = snapshot.split_enabled && snapshot.tx_vfo == VfoSelect::A;
                    ui.label(if a_tx { "A ▶TX" } else { "A" });
                    ui.label(egui::RichText::new(fmt_mhz(vfo_a_hz)).strong());
                    ui.label(mode_label(snapshot.demod_mode));
                    ui.end_row();

                    // VFO B (editable).  Click to type an exact value in MHz, or
                    // roll the mouse wheel over the field to nudge — mode-aware
                    // steps, same table as VFO A tuning (wheel = fine, Shift =
                    // medium, Alt = coarse).  The custom_parser reads the typed
                    // text as MHz; without it egui would treat "14.055" as 14 Hz.
                    let b_tx = snapshot.split_enabled && snapshot.tx_vfo == VfoSelect::B;
                    ui.label(if b_tx { "B ▶TX" } else { "B" });
                    let mut b_hz = vfo_b_hz as i64;
                    let mut resp = ui.add_enabled(
                        acquired,
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
                    if acquired && resp.hovered() {
                        let raw_y = ui.input(|i| i.raw_scroll_delta.y);
                        if raw_y != 0.0 {
                            let tier = ui.input(|i| {
                                if i.modifiers.alt {
                                    TuneTier::Coarse
                                } else if i.modifiers.shift {
                                    TuneTier::Medium
                                } else {
                                    TuneTier::Fine
                                }
                            });
                            let step = target_step_hz(snapshot.vfo_b_demod_mode, tier);
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
                    .add_enabled(acquired, egui::Button::new("Copy A -> B"))
                    .on_hover_text("Copy VFO A's frequency + mode to VFO B")
                    .clicked()
                {
                    self.copy_a_to_b(snapshot);
                }
                if ui
                    .add_enabled(acquired, egui::Button::new("Swap A <-> B"))
                    .on_hover_text("Swap VFO A and VFO B (frequency + mode)")
                    .clicked()
                {
                    self.swap_ab(snapshot);
                }
            });

            ui.separator();

            // ── Split + TX VFO ────────────────────────────────────────────
            let mut split = snapshot.split_enabled;
            if ui
                .add_enabled(
                    acquired,
                    egui::Checkbox::new(&mut split, "Split (transmit on the TX VFO)"),
                )
                .changed()
            {
                self.set_local(|s| s.split_enabled = split);
                self.send_radio_msg(ClientRadioMessage::SetSplit { enabled: split });
            }
            if split {
                ui.horizontal(|ui| {
                    ui.label("TX on:");
                    for (label, v) in [("A", VfoSelect::A), ("B", VfoSelect::B)] {
                        if ui.selectable_label(snapshot.tx_vfo == v, label).clicked() {
                            self.set_local(|s| s.tx_vfo = v);
                            self.send_radio_msg(ClientRadioMessage::SetTxVfo { vfo: v });
                        }
                    }
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
            let mut dw = snapshot.dual_watch_enabled;
            let resp = ui
                .add_enabled(
                    acquired && snapshot.dual_watch_supported,
                    egui::Checkbox::new(&mut dw, "Dual-watch — hear + show VFO B"),
                )
                .on_disabled_hover_text("Requires a multi-receiver Hermes Lite 2");
            if resp.changed() {
                self.set_local(|s| s.dual_watch_enabled = dw);
                self.send_radio_msg(ClientRadioMessage::SetDualWatch { enabled: dw });
            }
            if !snapshot.dual_watch_supported {
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

    /// Set VFO B's frequency and centre its LO window on it (RX1 NCO = target),
    /// so the tuned freq is always inside the window — the "QSY here" behaviour.
    /// Spectrum / spinner tuning then pans / offsets within the window from there.
    fn set_vfo_b_freq_centered(&self, hz: u64) {
        self.set_local(|s| {
            s.vfo_b_target_freq_hz = hz as f32;
            s.vfo_b_center_freq_hz = hz as f32;
        });
        self.send_radio_msg(ClientRadioMessage::SetVfoBFrequency { target_freq_hz: hz });
        self.send_radio_msg(ClientRadioMessage::SetVfoBCenterFrequency { center_freq_hz: hz });
    }

    fn copy_a_to_b(&self, snapshot: &UiState) {
        let hz = snapshot.target_freq_hz.max(0.0) as u64;
        self.set_vfo_b_freq_centered(hz);
        self.apply_vfo_b_mode(snapshot.demod_mode);
    }

    fn swap_ab(&self, snapshot: &UiState) {
        let a_hz = snapshot.target_freq_hz.max(0.0) as u64;
        let b_hz = snapshot.vfo_b_target_freq_hz.max(0.0) as u64;
        let a_mode = snapshot.demod_mode;
        let a_sb = snapshot.sideband;
        let b_mode = snapshot.vfo_b_demod_mode;
        let b_sb = snapshot.vfo_b_sideband;
        // VFO A ⇄ VFO B: swap frequency + mode/sideband, re-centring each VFO's
        // LO window on its new frequency.
        self.set_local(|s| {
            s.target_freq_hz = b_hz as f32;
            s.center_freq_hz = b_hz as f32;
            s.demod_mode = b_mode;
            s.sideband = b_sb;
        });
        self.send_radio_msg(ClientRadioMessage::SetTargetFrequency {
            target_freq_hz: b_hz,
        });
        self.send_radio_msg(ClientRadioMessage::SetCenterFrequency {
            center_freq_hz: b_hz,
        });
        self.send_radio_msg(ClientRadioMessage::SetDemodMode { mode: b_mode });
        self.send_radio_msg(ClientRadioMessage::SetSideband { sideband: b_sb });
        // VFO B ← A's freq + mode (centred).
        self.set_vfo_b_freq_centered(a_hz);
        self.send_radio_msg(ClientRadioMessage::SetVfoBDemodMode { mode: a_mode });
        self.send_radio_msg(ClientRadioMessage::SetVfoBSideband { sideband: a_sb });
    }

    /// Briefly lock `UiState` to apply a local edit for snappy feedback.
    fn set_local(&self, f: impl FnOnce(&mut UiState)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
    }
}
