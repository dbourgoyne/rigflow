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
        if snapshot.radio_acquired {
            egui::CollapsingHeader::new("Radio Control")
                .default_open(true)
                .show(ui, |ui| {
                    let mut save_demod_prefs = false;

                    if let Ok(mut state) = self.state.lock() {
                        // ---------------------------------------------------------
                        // Load active controls from per-demod preferences on mode switch
                        // ---------------------------------------------------------
                        let should_apply_controls = state.pending_apply_mode_controls
                            || state.last_demod_mode_for_controls != Some(snapshot.demod_mode);

                        if should_apply_controls {
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
                        }

                        // ---------------------------------------------------------
                        // Filter bandwidth
                        // ---------------------------------------------------------
                        let bw_limits = filter_bandwidth_limits(snapshot.demod_mode);

                        state.filter_bandwidth_hz =
                            clamp_filter_bandwidth(snapshot.demod_mode, state.filter_bandwidth_hz);

                        let at_default =
                            (state.filter_bandwidth_hz - bw_limits.default_hz).abs() < 1.0;

                        ui.horizontal(|ui| {
                            let slider_width = (ui.available_width() - 80.0).max(100.0);

                            let bw_response = ui.add_sized(
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
                                    .get_mut(snapshot.demod_mode)
                                    .filter_bandwidth_hz = default_hz;

                                state.filter_bw_debounce = DebounceState::new(default_hz);

                                self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                                    bandwidth_hz: default_hz,
                                });

                                save_demod_prefs = true;
                            }

                            // Keep per-demod preference in sync with active UI value
                            state
                                .demod_preferences
                                .get_mut(snapshot.demod_mode)
                                .filter_bandwidth_hz = state.filter_bandwidth_hz;

                            let now = Instant::now();

                            if bw_response.changed() {
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

                            if bw_response.drag_stopped() {
                                let final_hz = state
                                    .filter_bandwidth_hz
                                    .round()
                                    .clamp(bw_limits.min_hz, bw_limits.max_hz);

                                state.filter_bandwidth_hz = final_hz;
                                state
                                    .demod_preferences
                                    .get_mut(snapshot.demod_mode)
                                    .filter_bandwidth_hz = final_hz;

                                state.filter_bw_debounce.last_sent_value = final_hz;
                                state.filter_bw_debounce.last_send_time = now;

                                self.send_radio_msg(ClientRadioMessage::SetFilterBandwidth {
                                    bandwidth_hz: final_hz,
                                });

                                save_demod_prefs = true;
                            }
                        });

                        // ---------------------------------------------------------
                        // Pitch (only for modes where it applies)
                        // ---------------------------------------------------------
                        if let Some(limits) = pitch_limits(snapshot.demod_mode) {
                            state.pitch_hz = state.pitch_hz.clamp(limits.min_hz, limits.max_hz);

                            let at_default = (state.pitch_hz - limits.default_hz).abs() < 1.0;

                            ui.horizontal(|ui| {
                                let slider_width = (ui.available_width() - 80.0).max(100.0);

                                let pitch_response = ui.add_sized(
                                    [slider_width, 0.0],
                                    egui::Slider::new(
                                        &mut state.pitch_hz,
                                        limits.min_hz..=limits.max_hz,
                                    )
                                    .text(RichText::new(limits.label).size(11.0)),
                                );

                                if ui
                                    .add_enabled(
                                        !at_default,
                                        egui::Button::new(
                                            RichText::new("Restore Default").size(8.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    let default_hz = limits.default_hz;

                                    state.pitch_hz = default_hz;
                                    state
                                        .demod_preferences
                                        .get_mut(snapshot.demod_mode)
                                        .pitch_hz = default_hz;

                                    state.pitch_debounce = DebounceState::new(default_hz);

                                    self.send_radio_msg(ClientRadioMessage::SetPitch {
                                        pitch_hz: default_hz,
                                    });

                                    save_demod_prefs = true;
                                }

                                // Keep per-demod preference in sync with active UI value
                                state
                                    .demod_preferences
                                    .get_mut(snapshot.demod_mode)
                                    .pitch_hz = state.pitch_hz;

                                let now = Instant::now();

                                if pitch_response.changed() {
                                    if let Some(send_hz) = should_send_debounced(
                                        now,
                                        state.pitch_hz,
                                        &mut state.pitch_debounce,
                                        limits.debounce_delta_hz,
                                        Duration::from_millis(limits.debounce_interval_ms),
                                    ) {
                                        self.send_radio_msg(ClientRadioMessage::SetPitch {
                                            pitch_hz: send_hz,
                                        });
                                    }
                                }

                                if pitch_response.drag_stopped() {
                                    let final_hz =
                                        state.pitch_hz.round().clamp(limits.min_hz, limits.max_hz);

                                    state.pitch_hz = final_hz;
                                    state
                                        .demod_preferences
                                        .get_mut(snapshot.demod_mode)
                                        .pitch_hz = final_hz;

                                    state.pitch_debounce.last_sent_value = final_hz;
                                    state.pitch_debounce.last_send_time = now;

                                    self.send_radio_msg(ClientRadioMessage::SetPitch {
                                        pitch_hz: final_hz,
                                    });

                                    save_demod_prefs = true;
                                }
                            });
                        }

                        if default_deemphasis_mode(snapshot.demod_mode).is_some() {
                            let mut deemphasis_changed = false;
                            let default_mode =
                                default_deemphasis_mode(snapshot.demod_mode).unwrap();
                            let at_default = state.deemphasis_mode == default_mode;

                            ui.horizontal(|ui| {
                                ui.label("Deemphasis");

                                egui::ComboBox::from_id_salt("deemphasis_mode_combo")
                                    .selected_text(state.deemphasis_mode.label())
                                    .show_ui(ui, |ui| {
                                        deemphasis_changed |= ui
                                            .selectable_value(
                                                &mut state.deemphasis_mode,
                                                DeemphasisMode::Off,
                                                DeemphasisMode::Off.label(),
                                            )
                                            .changed();

                                        deemphasis_changed |= ui
                                            .selectable_value(
                                                &mut state.deemphasis_mode,
                                                DeemphasisMode::Tau50us,
                                                DeemphasisMode::Tau50us.label(),
                                            )
                                            .changed();

                                        deemphasis_changed |= ui
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
                                        egui::Button::new(
                                            RichText::new("Restore Default").size(8.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    state.deemphasis_mode = default_mode;
                                    state
                                        .demod_preferences
                                        .get_mut(snapshot.demod_mode)
                                        .deemphasis_mode = default_mode;

                                    self.send_radio_msg(ClientRadioMessage::SetDeemphasisMode {
                                        mode: state.deemphasis_mode,
                                    });

                                    save_demod_prefs = true;
                                }
                            });

                            if deemphasis_changed {
                                state
                                    .demod_preferences
                                    .get_mut(snapshot.demod_mode)
                                    .deemphasis_mode = state.deemphasis_mode;

                                self.send_radio_msg(ClientRadioMessage::SetDeemphasisMode {
                                    mode: state.deemphasis_mode,
                                });

                                save_demod_prefs = true;
                            }
                        }
                    }

                    ui.label("Demod");

                    let mut selected_demod = snapshot.demod_mode.clone();

                    ui.horizontal(|ui| {
                        ui.radio_value(&mut selected_demod, DemodMode::Wfm, "wfm");
                        ui.radio_value(&mut selected_demod, DemodMode::Nfm, "nfm");
                        ui.radio_value(&mut selected_demod, DemodMode::Am, "am");
                        ui.radio_value(&mut selected_demod, DemodMode::Cw, "cw");
                        ui.radio_value(&mut selected_demod, DemodMode::Lsb, "lsb");
                        ui.radio_value(&mut selected_demod, DemodMode::Usb, "usb");
                    });

                    if selected_demod != snapshot.demod_mode {
                        if let Ok(mut state) = self.state.lock() {
                            state.demod_mode = selected_demod.clone();
                            state.sideband = match selected_demod {
                                DemodMode::Lsb => Sideband::Lsb,
                                DemodMode::Usb => Sideband::Usb,
                                _ => state.sideband,
                            };
                        }

                        self.send_radio_msg(ClientRadioMessage::SetDemodMode {
                            mode: selected_demod,
                        });

                        if selected_demod == DemodMode::Lsb || selected_demod == DemodMode::Usb {
                            self.send_radio_msg(ClientRadioMessage::SetSideband {
                                sideband: match selected_demod {
                                    DemodMode::Lsb => Sideband::Lsb,
                                    DemodMode::Usb => Sideband::Usb,
                                    _ => unreachable!("sideband only sent for USB/LSB"),
                                },
                            });
                        }
                    }
                    if save_demod_prefs {
                        self.save_demod_preferences_to_current_operator();
                    }
                });
        }
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
