use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_core::radio::source_control::{DirectSamplingMode, GainMode};
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    pub(crate) fn draw_source_control_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if snapshot.radio_acquired {
            egui::CollapsingHeader::new("Source Control")
                .default_open(true)
                .show(ui, |ui| {
                    if let Ok(mut state) = self.state.lock() {
                        // -----------------------------
                        // Sample rate
                        // -----------------------------
                        if state.source_capabilities.supports_sample_rate {
                            let sample_rates = state.source_capabilities.sample_rates_hz.clone();

                            if !sample_rates.is_empty() {
                                let mut selected_sample_rate = state.source_control.sample_rate_hz;

                                egui::ComboBox::from_id_salt("source_sample_rate_combo")
                                    .selected_text(format_sample_rate(selected_sample_rate))
                                    .show_ui(ui, |ui| {
                                        for sample_rate_hz in sample_rates {
                                            ui.selectable_value(
                                                &mut selected_sample_rate,
                                                sample_rate_hz,
                                                format_sample_rate(sample_rate_hz),
                                            );
                                        }
                                    });

                                if selected_sample_rate != state.source_control.sample_rate_hz {
                                    state.source_control.sample_rate_hz = selected_sample_rate;

                                    self.send_radio_msg(ClientRadioMessage::SetSourceSampleRate {
                                        sample_rate_hz: selected_sample_rate,
                                    });
                                }
                            } else {
                                ui.label("Sample rates unavailable");
                            }
                        }

                        // -----------------------------
                        // Gain mode: Auto / Manual
                        // -----------------------------
                        let ds_active =
                            state.source_control.direct_sampling != DirectSamplingMode::Off;

                        if state.source_capabilities.supports_gain_mode {
                            ui.add_enabled_ui(!ds_active, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Gain Mode");

                                    let mut gain_mode = state.source_control.gain_mode;

                                    let auto_changed = ui
                                        .radio_value(&mut gain_mode, GainMode::Auto, "Auto")
                                        .changed();

                                    let manual_changed = ui
                                        .radio_value(&mut gain_mode, GainMode::Manual, "Manual")
                                        .changed();

                                    if auto_changed || manual_changed {
                                        state.source_control.gain_mode = gain_mode;

                                        self.send_radio_msg(
                                            ClientRadioMessage::SetSourceGainMode {
                                                mode: gain_mode,
                                            },
                                        );
                                    }
                                });
                            });
                        }

                        // -----------------------------
                        // Gain value
                        // -----------------------------
                        if state.source_capabilities.supports_gain {
                            let manual_gain =
                                !ds_active && state.source_control.gain_mode == GainMode::Manual;

                            ui.add_enabled_ui(manual_gain, |ui| {
                                let gains = &state.source_capabilities.gain_values_db;

                                if !gains.is_empty() {
                                    let min_gain = gains.first().copied().unwrap_or(0.0);
                                    let max_gain = gains.last().copied().unwrap_or(50.0);

                                    let mut gain_db = state.source_control.gain_db;

                                    let response = ui.add(
                                        egui::Slider::new(&mut gain_db, min_gain..=max_gain).text(
                                            format!(
                                                "Gain ({:.1} dB)",
                                                state.source_control.gain_db
                                            ),
                                        ),
                                    );
                                    if response.changed() {
                                        let snapped_gain = gains
                                            .iter()
                                            .copied()
                                            .min_by(|a, b| {
                                                (gain_db - *a)
                                                    .abs()
                                                    .partial_cmp(&(gain_db - *b).abs())
                                                    .unwrap_or(std::cmp::Ordering::Equal)
                                            })
                                            .unwrap_or(gain_db);

                                        if (snapped_gain - state.source_control.gain_db).abs()
                                            > f32::EPSILON
                                        {
                                            state.source_control.gain_db = snapped_gain;

                                            self.send_radio_msg(
                                                ClientRadioMessage::SetSourceGain {
                                                    gain_db: snapped_gain,
                                                },
                                            );
                                        }
                                    }
                                } else {
                                    ui.label("Gain values unavailable");
                                }
                            });
                        }

                        if ds_active
                            && (state.source_capabilities.supports_gain_mode
                                || state.source_capabilities.supports_gain)
                        {
                            ui.label("Gain is not applicable in direct sampling mode.");
                        }

                        // -----------------------------
                        // PPM correction
                        // -----------------------------
                        if state.source_capabilities.supports_ppm_correction {
                            let ppm_min = state.source_capabilities.ppm_min;
                            let ppm_max = state.source_capabilities.ppm_max;
                            let mut ppm = state.source_control.ppm_correction;

                            ui.label("PPM Correction");
                            ui.horizontal(|ui| {
                                let slider = ui.add(
                                    egui::Slider::new(&mut ppm, ppm_min..=ppm_max)
                                        .integer()
                                        .show_value(false),
                                );

                                let sign = if ppm > 0 { "+" } else { "" };
                                ui.label(format!("{sign}{ppm} ppm"));

                                let reset = ui
                                    .add_enabled(ppm != 0, egui::Button::new("Reset"))
                                    .clicked();

                                if slider.changed() || reset {
                                    if reset {
                                        ppm = 0;
                                    }
                                    state.source_control.ppm_correction = ppm;
                                    self.send_radio_msg(
                                        ClientRadioMessage::SetSourcePpmCorrection { ppm },
                                    );
                                }
                            });
                        }

                        // -----------------------------
                        // Direct sampling mode
                        // -----------------------------
                        if state.source_capabilities.supports_direct_sampling {
                            let modes = state.source_capabilities.direct_sampling_modes.clone();

                            if !modes.is_empty() {
                                let mut selected = state.source_control.direct_sampling;

                                ui.horizontal(|ui| {
                                    ui.label("Direct Sampling");

                                    egui::ComboBox::from_id_salt("source_direct_sampling_combo")
                                        .selected_text(format_direct_sampling_mode(selected))
                                        .show_ui(ui, |ui| {
                                            for mode in modes {
                                                ui.selectable_value(
                                                    &mut selected,
                                                    mode,
                                                    format_direct_sampling_mode(mode),
                                                );
                                            }
                                        });
                                });

                                if selected != state.source_control.direct_sampling {
                                    state.source_control.direct_sampling = selected;

                                    self.send_radio_msg(
                                        ClientRadioMessage::SetSourceDirectSampling {
                                            mode: selected,
                                        },
                                    );
                                }
                            }
                        }
                    }
                });
        }
    }
}

fn format_direct_sampling_mode(mode: DirectSamplingMode) -> &'static str {
    match mode {
        DirectSamplingMode::Off => "Off",
        DirectSamplingMode::I => "I channel",
        DirectSamplingMode::Q => "Q channel",
    }
}

fn format_sample_rate(sample_rate_hz: u32) -> String {
    if sample_rate_hz >= 1_000_000 {
        format!("{:.3} MSPS", sample_rate_hz as f32 / 1_000_000.0)
    } else if sample_rate_hz >= 1_000 {
        format!("{:.1} kSPS", sample_rate_hz as f32 / 1_000.0)
    } else {
        format!("{sample_rate_hz} SPS")
    }
}
