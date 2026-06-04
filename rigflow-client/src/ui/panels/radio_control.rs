use std::time::{Duration, Instant};

use crate::ui::app::RigflowApp;
use crate::ui::state::DebounceState;
use crate::ui::utils::should_send_debounced;
use crate::UiState;
use eframe::egui;
use egui::RichText;
use rigflow_core::dsp::modes::{
    clamp_filter_bandwidth, default_deemphasis_mode, filter_bandwidth_limits, pitch_limits,
    DeemphasisMode, DemodMode, Sideband,
};
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    pub(crate) fn draw_radio_control_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if !snapshot.radio_acquired {
            return;
        }

        // Read-only status (S-meter, extensible for future fields), shown above
        // the controls.
        self.draw_radio_status_section(ui, snapshot);

        egui::CollapsingHeader::new("Radio Control")
            .default_open(true)
            .show(ui, |ui| {
                let mut save_demod_prefs = false;
                let mut save_volume = false;

                if let Ok(mut state) = self.state.lock() {
                    // Apply persisted per-demod controls when the mode changes.
                    let should_apply = state.pending_apply_mode_controls
                        || state.last_demod_mode_for_controls != Some(snapshot.demod_mode);

                    if should_apply {
                        state.pending_apply_mode_controls = false;
                        apply_mode_preferences(&mut state, snapshot.demod_mode);

                        self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                            bandwidth_hz: state.filter_bandwidth_hz,
                        });
                        if pitch_limits(snapshot.demod_mode).is_some() {
                            self.send_radio_msg(ClientRadioMessage::SetPitch {
                                pitch_hz: state.pitch_hz,
                            });
                        }
                        if default_deemphasis_mode(snapshot.demod_mode).is_some() {
                            self.send_radio_msg(ClientRadioMessage::SetDeemphasisMode {
                                mode: state.deemphasis_mode,
                            });
                        }
                        // Push the persisted receive volume to the server (the
                        // snapshot's server default is intentionally ignored).
                        self.send_radio_msg(ClientRadioMessage::SetVolume {
                            volume_percent: state.volume_percent,
                        });
                    }

                    save_demod_prefs |=
                        self.draw_filter_bandwidth_row(ui, &mut state, snapshot.demod_mode);
                    save_demod_prefs |= self.draw_pitch_row(ui, &mut state, snapshot.demod_mode);
                    save_demod_prefs |=
                        self.draw_deemphasis_row(ui, &mut state, snapshot.demod_mode);

                    self.draw_squelch_row(ui, &mut state);
                    self.draw_nr2_row(ui, &mut state);
                    self.draw_agc_row(ui, &mut state);
                    save_volume = self.draw_volume_row(ui, &mut state);
                    self.draw_cw_sidetone_row(ui, &mut state, snapshot.demod_mode);
                }

                save_demod_prefs |= self.draw_demod_selector(ui, snapshot);

                if save_demod_prefs {
                    self.save_demod_preferences_to_current_operator();
                }
                if save_volume {
                    self.save_volume_to_current_operator();
                }
            });
    }

    /// Receive squelch: enable checkbox, threshold slider, and a live gate
    /// indicator.  These are radio (DSP) controls sent to the server; they are
    /// not persisted as demod preferences.
    fn draw_squelch_row(&self, ui: &mut egui::Ui, state: &mut UiState) {
        ui.separator();

        ui.horizontal(|ui| {
            let mut enabled = state.squelch_enabled;
            if ui.checkbox(&mut enabled, "Squelch").changed() {
                state.squelch_enabled = enabled;
                self.send_radio_msg(ClientRadioMessage::SetSquelchEnabled { enabled });
            }

            // Live gate indicator from the server-reported open state.
            let (text, color) = if !state.squelch_enabled {
                ("—", egui::Color32::GRAY)
            } else if state.squelch_open {
                ("● open", egui::Color32::from_rgb(100, 220, 100))
            } else {
                ("muted", egui::Color32::from_rgb(210, 130, 130))
            };
            ui.label(RichText::new(text).color(color).small());
        });

        let enabled = state.squelch_enabled;
        ui.add_enabled_ui(enabled, |ui| {
            let mut threshold_db = state.squelch_threshold_db;
            let response = ui.add(
                egui::Slider::new(&mut threshold_db, -120.0..=0.0)
                    .step_by(1.0)
                    .fixed_decimals(0)
                    .suffix(" dBFS")
                    .text("Threshold"),
            );
            if response.changed() {
                state.squelch_threshold_db = threshold_db.clamp(-120.0, 0.0);
                self.send_radio_msg(ClientRadioMessage::SetSquelchThreshold {
                    threshold_db: state.squelch_threshold_db,
                });
            }
        });
    }

    /// NR2 spectral noise reduction enable.  A radio (DSP) control sent to the
    /// server; applied to demodulated receive audio.  Not persisted.
    fn draw_nr2_row(&self, ui: &mut egui::Ui, state: &mut UiState) {
        let mut enabled = state.nr2_enabled;
        if ui.checkbox(&mut enabled, "NR2 noise reduction").changed() {
            state.nr2_enabled = enabled;
            self.send_radio_msg(ClientRadioMessage::SetNr2Enabled { enabled });
        }

        ui.add_enabled_ui(state.nr2_enabled, |ui| {
            let mut strength = state.nr2_strength;
            let response = ui.add(
                egui::Slider::new(&mut strength, 0.0..=1.0)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .text("NR2 Strength"),
            );
            if response.changed() {
                state.nr2_strength = strength.clamp(0.0, 1.0);
                self.send_radio_msg(ClientRadioMessage::SetNr2Strength {
                    strength: state.nr2_strength,
                });
            }
        });
    }

    /// Receive-audio volume slider (0–100%).  Sends `SetVolume` on change and
    /// returns `true` when the value changed (so the caller persists it).
    fn draw_volume_row(&self, ui: &mut egui::Ui, state: &mut UiState) -> bool {
        ui.separator();
        let mut volume = state.volume_percent as i32;
        let response = ui.add(
            egui::Slider::new(&mut volume, 0..=100)
                .integer()
                .suffix("%")
                .text("Volume"),
        );
        if response.changed() {
            let v = volume.clamp(0, 100) as u8;
            if v != state.volume_percent {
                state.volume_percent = v;
                self.send_radio_msg(ClientRadioMessage::SetVolume { volume_percent: v });
                return true;
            }
        }
        false
    }

    /// CW Sidetone Volume slider (0–100%), shown only in CWU/CWL.  This is a
    /// client-local control (independent of RX Volume) that drives the locally
    /// generated sidetone; it is never sent to the server.  The audio callback
    /// reads `state.sidetone`, which the Space-bar handler keeps in sync.
    fn draw_cw_sidetone_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Cwu | DemodMode::Cwl) {
            return;
        }
        ui.separator();
        let mut vol = state.cw_sidetone_volume as i32;
        let response = ui.add(
            egui::Slider::new(&mut vol, 0..=100)
                .integer()
                .suffix("%")
                .text("CW Sidetone"),
        );
        if response.changed() {
            let v = vol.clamp(0, 100) as u8;
            state.cw_sidetone_volume = v;
            // Reflect immediately into the lock-free audio control.
            state.sidetone.set_volume(v as f32 / 100.0);
        }
    }

    /// Read-only "Radio Status" section (S-meter for now; extensible).
    fn draw_radio_status_section(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        egui::CollapsingHeader::new("Radio Status")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("radio_status_grid")
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("Signal");
                        ui.label(format!(
                            "{} ({:.0} dBm)",
                            s_meter_label(snapshot.signal_dbm),
                            snapshot.signal_dbm
                        ));
                        ui.end_row();
                    });
            });
    }

    /// AGC enable + strength.  A radio (DSP) control sent to the server;
    /// applied to demodulated receive audio (before NR2/squelch). Not persisted.
    fn draw_agc_row(&self, ui: &mut egui::Ui, state: &mut UiState) {
        ui.separator();

        let mut enabled = state.agc_enabled;
        if ui.checkbox(&mut enabled, "AGC").changed() {
            state.agc_enabled = enabled;
            self.send_radio_msg(ClientRadioMessage::SetAgcEnabled { enabled });
        }

        ui.add_enabled_ui(state.agc_enabled, |ui| {
            let mut strength = state.agc_strength;
            let response = ui.add(
                egui::Slider::new(&mut strength, 0.0..=1.0)
                    .step_by(0.01)
                    .fixed_decimals(2)
                    .text("AGC Strength"),
            );
            if response.changed() {
                state.agc_strength = strength.clamp(0.0, 1.0);
                self.send_radio_msg(ClientRadioMessage::SetAgcStrength {
                    strength: state.agc_strength,
                });
            }
        });
    }

    fn draw_filter_bandwidth_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
    ) -> bool {
        let mut save = false;
        let bw_limits = filter_bandwidth_limits(demod_mode);

        state.filter_bandwidth_hz = clamp_filter_bandwidth(demod_mode, state.filter_bandwidth_hz);

        let at_default = (state.filter_bandwidth_hz - bw_limits.default_hz).abs() < 1.0;

        ui.horizontal(|ui| {
            let slider_width = (ui.available_width() - 80.0).max(100.0);

            let response = ui.add_sized(
                [slider_width, 0.0],
                egui::Slider::new(
                    &mut state.filter_bandwidth_hz,
                    bw_limits.min_hz..=bw_limits.max_hz,
                )
                .text(RichText::new("Filter BW (Hz)").size(11.0)),
            );

            if ui
                .add_enabled(
                    !at_default,
                    egui::Button::new(RichText::new("Restore Default").size(8.0)),
                )
                .clicked()
            {
                let default_hz = bw_limits.default_hz;
                state.filter_bandwidth_hz = default_hz;
                state
                    .demod_preferences
                    .get_mut(demod_mode)
                    .filter_bandwidth_hz = default_hz;
                state.filter_bw_debounce = DebounceState::new(default_hz);
                self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                    bandwidth_hz: default_hz,
                });
                save = true;
            }

            state
                .demod_preferences
                .get_mut(demod_mode)
                .filter_bandwidth_hz = state.filter_bandwidth_hz;

            let now = Instant::now();

            if response.changed() {
                if let Some(send_hz) = should_send_debounced(
                    now,
                    state.filter_bandwidth_hz,
                    &mut state.filter_bw_debounce,
                    10.0,
                    Duration::from_millis(75),
                ) {
                    self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                        bandwidth_hz: send_hz,
                    });
                }
            }

            if response.drag_stopped() {
                let final_hz = state
                    .filter_bandwidth_hz
                    .round()
                    .clamp(bw_limits.min_hz, bw_limits.max_hz);

                state.filter_bandwidth_hz = final_hz;
                state
                    .demod_preferences
                    .get_mut(demod_mode)
                    .filter_bandwidth_hz = final_hz;
                state.filter_bw_debounce.last_sent_value = final_hz;
                state.filter_bw_debounce.last_send_time = now;
                self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                    bandwidth_hz: final_hz,
                });
                save = true;
            }
        });

        save
    }

    fn draw_pitch_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
    ) -> bool {
        let Some(limits) = pitch_limits(demod_mode) else {
            return false;
        };

        let mut save = false;

        state.pitch_hz = state.pitch_hz.clamp(limits.min_hz, limits.max_hz);
        let at_default = (state.pitch_hz - limits.default_hz).abs() < 1.0;

        ui.horizontal(|ui| {
            let slider_width = (ui.available_width() - 80.0).max(100.0);

            let response = ui.add_sized(
                [slider_width, 0.0],
                egui::Slider::new(&mut state.pitch_hz, limits.min_hz..=limits.max_hz)
                    .text(RichText::new(limits.label).size(11.0)),
            );

            if ui
                .add_enabled(
                    !at_default,
                    egui::Button::new(RichText::new("Restore Default").size(8.0)),
                )
                .clicked()
            {
                let default_hz = limits.default_hz;
                state.pitch_hz = default_hz;
                state.demod_preferences.get_mut(demod_mode).pitch_hz = default_hz;
                state.pitch_debounce = DebounceState::new(default_hz);
                self.send_radio_msg(ClientRadioMessage::SetPitch {
                    pitch_hz: default_hz,
                });
                save = true;
            }

            state.demod_preferences.get_mut(demod_mode).pitch_hz = state.pitch_hz;

            let now = Instant::now();

            if response.changed() {
                if let Some(send_hz) = should_send_debounced(
                    now,
                    state.pitch_hz,
                    &mut state.pitch_debounce,
                    limits.debounce_delta_hz,
                    Duration::from_millis(limits.debounce_interval_ms),
                ) {
                    self.send_radio_msg(ClientRadioMessage::SetPitch { pitch_hz: send_hz });
                }
            }

            if response.drag_stopped() {
                let final_hz = state.pitch_hz.round().clamp(limits.min_hz, limits.max_hz);
                state.pitch_hz = final_hz;
                state.demod_preferences.get_mut(demod_mode).pitch_hz = final_hz;
                state.pitch_debounce.last_sent_value = final_hz;
                state.pitch_debounce.last_send_time = now;
                self.send_radio_msg(ClientRadioMessage::SetPitch { pitch_hz: final_hz });
                save = true;
            }
        });

        save
    }

    fn draw_deemphasis_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
    ) -> bool {
        if default_deemphasis_mode(demod_mode).is_none() {
            return false;
        }

        let mut save = false;
        let mut changed = false;
        let default_mode = default_deemphasis_mode(demod_mode).unwrap();
        let at_default = state.deemphasis_mode == default_mode;

        ui.horizontal(|ui| {
            ui.label("Deemphasis");

            egui::ComboBox::from_id_salt("deemphasis_mode_combo")
                .selected_text(state.deemphasis_mode.label())
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut state.deemphasis_mode,
                            DeemphasisMode::Off,
                            DeemphasisMode::Off.label(),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut state.deemphasis_mode,
                            DeemphasisMode::Tau50us,
                            DeemphasisMode::Tau50us.label(),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut state.deemphasis_mode,
                            DeemphasisMode::Tau75us,
                            DeemphasisMode::Tau75us.label(),
                        )
                        .changed();
                });

            if ui
                .add_enabled(
                    !at_default,
                    egui::Button::new(RichText::new("Restore Default").size(8.0)),
                )
                .clicked()
            {
                state.deemphasis_mode = default_mode;
                state.demod_preferences.get_mut(demod_mode).deemphasis_mode = default_mode;
                self.send_radio_msg(ClientRadioMessage::SetDeemphasisMode {
                    mode: state.deemphasis_mode,
                });
                save = true;
            }
        });

        if changed {
            state.demod_preferences.get_mut(demod_mode).deemphasis_mode = state.deemphasis_mode;
            self.send_radio_msg(ClientRadioMessage::SetDeemphasisMode {
                mode: state.deemphasis_mode,
            });
            save = true;
        }

        save
    }

    fn draw_demod_selector(&self, ui: &mut egui::Ui, snapshot: &UiState) -> bool {
        ui.label("Demod");

        let mut selected = snapshot.demod_mode.clone();

        ui.horizontal(|ui| {
            ui.radio_value(&mut selected, DemodMode::Wfm, "wfm");
            ui.radio_value(&mut selected, DemodMode::Nfm, "nfm");
            ui.radio_value(&mut selected, DemodMode::Am, "am");
            ui.radio_value(&mut selected, DemodMode::Lsb, "lsb");
            ui.radio_value(&mut selected, DemodMode::Usb, "usb");
            ui.radio_value(&mut selected, DemodMode::Cwu, "cwu");
            ui.radio_value(&mut selected, DemodMode::Cwl, "cwl");
        });

        if selected == snapshot.demod_mode {
            return false;
        }

        if let Ok(mut state) = self.state.lock() {
            state.demod_mode = selected.clone();
            state.sideband = match selected {
                DemodMode::Lsb => Sideband::Lsb,
                DemodMode::Usb => Sideband::Usb,
                _ => state.sideband,
            };
        }

        self.send_radio_msg(ClientRadioMessage::SetDemodMode { mode: selected });

        if selected == DemodMode::Lsb || selected == DemodMode::Usb {
            self.send_radio_msg(ClientRadioMessage::SetSideband {
                sideband: match selected {
                    DemodMode::Lsb => Sideband::Lsb,
                    DemodMode::Usb => Sideband::Usb,
                    _ => unreachable!("sideband only sent for USB/LSB"),
                },
            });
        }

        false // demod change does not trigger persisting per-demod prefs
    }
}

fn apply_mode_preferences(state: &mut UiState, mode: DemodMode) {
    let prefs = state.demod_preferences.get(mode);

    state.filter_bandwidth_hz = prefs.filter_bandwidth_hz;
    state.pitch_hz = prefs.pitch_hz;
    state.deemphasis_mode = prefs.deemphasis_mode;

    state.filter_bw_debounce = DebounceState::new(state.filter_bandwidth_hz);
    state.pitch_debounce = DebounceState::new(state.pitch_hz);

    state.last_demod_mode_for_controls = Some(mode);
}

/// Format a signal level (dBm) as an S-meter label.
///
/// HF convention: S9 = -73 dBm, 6 dB per S-unit.  Below S1 clamps to "S0";
/// above S9 shows "S9+N dB" (N rounded to the nearest 10 dB, as is customary).
fn s_meter_label(dbm: f32) -> String {
    const S9_DBM: f32 = -73.0;
    const DB_PER_S_UNIT: f32 = 6.0;

    if dbm > S9_DBM {
        let over = dbm - S9_DBM;
        let rounded = ((over / 10.0).round() * 10.0) as i32;
        if rounded <= 0 {
            "S9".to_string()
        } else {
            format!("S9+{rounded} dB")
        }
    } else {
        let s = (9.0 + (dbm - S9_DBM) / DB_PER_S_UNIT)
            .round()
            .clamp(0.0, 9.0) as i32;
        format!("S{s}")
    }
}
