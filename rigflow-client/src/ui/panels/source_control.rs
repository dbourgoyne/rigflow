use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::ham_band::{
    band_from_frequency, default_frequency_for_band, default_mode_for_band, HamBand,
};
use rigflow_core::radio::source_control::{DirectSamplingMode, GainMode};
use rigflow_core::radio::swr_sweep::{validate_sweep_range, SWR_SWEEP_POINTS};
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    pub(crate) fn draw_source_control_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if snapshot.radio_acquired {
            egui::CollapsingHeader::new("Source Control")
                .default_open(true)
                .show(ui, |ui| {
                    let mut save_source_control = false;

                    if let Ok(mut state) = self.state.lock() {
                        // Apply saved source-control preferences to hardware after
                        // a radio acquire if persisted settings were found.
                        if state.pending_apply_source_control {
                            state.pending_apply_source_control = false;
                            if state.source_capabilities.supports_sample_rate {
                                self.send_radio_msg(ClientRadioMessage::SetSourceSampleRate {
                                    sample_rate_hz: state.source_control.sample_rate_hz,
                                });
                            }
                            if state.source_capabilities.supports_gain_mode {
                                self.send_radio_msg(ClientRadioMessage::SetSourceGainMode {
                                    mode: state.source_control.gain_mode,
                                });
                            }
                            if state.source_capabilities.supports_gain {
                                self.send_radio_msg(ClientRadioMessage::SetSourceGain {
                                    gain_db: state.source_control.gain_db,
                                });
                            }
                            if state.source_capabilities.supports_ppm_correction {
                                self.send_radio_msg(ClientRadioMessage::SetSourcePpmCorrection {
                                    ppm: state.source_control.ppm_correction,
                                });
                            }
                            if state.source_capabilities.supports_direct_sampling {
                                self.send_radio_msg(ClientRadioMessage::SetSourceDirectSampling {
                                    mode: state.source_control.direct_sampling,
                                });
                            }
                            if state.source_capabilities.supports_tx_tune_test {
                                self.send_radio_msg(ClientRadioMessage::SetSourceTxDrive {
                                    tx_drive_percent: state.source_control.tx_drive_percent,
                                });
                                self.send_radio_msg(ClientRadioMessage::SetSourceSpotLevel {
                                    spot_level_percent: state.source_control.spot_level_percent,
                                });
                            }
                            if state.source_capabilities.supports_band_control {
                                self.send_radio_msg(ClientRadioMessage::SetSourceN2adrEnabled {
                                    enabled: state.source_control.n2adr_enabled,
                                });
                            }
                            if state.source_capabilities.supports_fdx {
                                self.send_radio_msg(ClientRadioMessage::SetSourceFdxEnabled {
                                    enabled: state.source_control.fdx_enabled,
                                });
                            }
                        }

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
                                    save_source_control = true;
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
                                        save_source_control = true;
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
                                            save_source_control = true;
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
                                    save_source_control = true;
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
                                    save_source_control = true;
                                }
                            }
                        }

                        // -----------------------------
                        // TX Drive (%) — operator transmit power.  Part of
                        // source control: applies to all transmit operations
                        // (Spot now; CW/SSB/digital/sweep later).  Gated on TX
                        // support.  Flows through the source-control plane like
                        // gain (SetSourceTxDrive); the server uses it when a
                        // Spot/SWR measurement runs.
                        // -----------------------------
                        if state.source_capabilities.supports_tx_tune_test {
                            let mut tx_drive = state.source_control.tx_drive_percent;
                            let resp = ui.add(
                                egui::Slider::new(&mut tx_drive, 0.0..=100.0)
                                    .step_by(1.0)
                                    .fixed_decimals(0)
                                    .suffix("%")
                                    .text("TX Drive"),
                            );
                            if resp.changed() {
                                let snapped = tx_drive.clamp(0.0, 100.0).round();
                                if (snapped - state.source_control.tx_drive_percent).abs()
                                    > f32::EPSILON
                                {
                                    state.source_control.tx_drive_percent = snapped;
                                    self.send_radio_msg(ClientRadioMessage::SetSourceTxDrive {
                                        tx_drive_percent: snapped,
                                    });
                                    save_source_control = true;
                                }
                            }
                        }

                        // -----------------------------
                        // Band Control + N2ADR (HL2).
                        // -----------------------------
                        if state.source_capabilities.supports_band_control {
                            save_source_control |= self.draw_band_control(ui, &mut state);
                        }

                        // -----------------------------
                        // SWR Sweep (HL2 TX).
                        // -----------------------------
                        if state.source_capabilities.supports_tx_tune_test {
                            self.draw_swr_sweep_section(ui, &mut state);
                        }

                        // -----------------------------
                        // FDX / TX Monitor Spectrum (HL2).
                        // -----------------------------
                        if state.source_capabilities.supports_fdx {
                            save_source_control |= self.draw_fdx_control(ui, &mut state);
                        }

                        // -----------------------------
                        // TX Test Tone (HL2, FDX Phase 2).
                        // -----------------------------
                        if state.source_capabilities.supports_tx_tune_test {
                            self.draw_tx_test_tone_section(ui, &mut state);
                        }
                    }

                    if save_source_control {
                        self.save_source_control_prefs_to_current_operator();
                    }
                });
        }
    }

    /// HL2 Band Control: band radio buttons (tune to default freq + mode via the
    /// existing control paths) and the N2ADR filter-board enable.  Returns
    /// `true` when the N2ADR enable changed (so the caller persists it).
    ///
    /// The selected band is *derived* from the current target frequency, so it
    /// always reflects actual tuning (band detection), no matter how the
    /// frequency was changed.  Band buttons only set frequency + mode; AGC, NR2,
    /// Volume, Squelch, filter bandwidth and pitch are untouched here.
    fn draw_band_control(&self, ui: &mut egui::Ui, state: &mut UiState) -> bool {
        let mut save = false;
        ui.separator();
        ui.label("Band");

        let current_band = band_from_frequency(state.target_freq_hz.max(0.0) as u64);
        let mut selected = current_band;

        ui.horizontal_wrapped(|ui| {
            for band in HamBand::ALL {
                ui.radio_value(&mut selected, Some(band), band.label());
            }
        });

        if let Some(band) = selected {
            if Some(band) != current_band {
                // Tune to the band default through the existing tuning path
                // (clamped, server-validated).  Move both the LO (center) and
                // the target so the band is actually received.
                let freq = default_frequency_for_band(band) as f32;
                let mode = default_mode_for_band(band);

                let limits = crate::ui::freq_limits::active_freq_limits(state);
                let new_center = crate::ui::freq_limits::clamp_center(freq, &limits);
                let new_target = crate::ui::freq_limits::clamp_target(
                    freq,
                    new_center,
                    state.input_sample_rate_hz,
                    &limits,
                );

                state.center_freq_hz = new_center;
                state.target_freq_hz = new_target;
                state.demod_mode = mode;

                // Auto-populate the SWR-sweep range to this band's edges.
                let (lo, hi) = band.range_hz();
                state.swr_sweep_start_mhz = lo as f64 / 1_000_000.0;
                state.swr_sweep_stop_mhz = hi as f64 / 1_000_000.0;
                state.sideband = match mode {
                    DemodMode::Usb => Sideband::Usb,
                    DemodMode::Lsb => Sideband::Lsb,
                    _ => state.sideband,
                };

                self.send_radio_msg(ClientRadioMessage::SetCenterFrequency {
                    center_freq_hz: new_center as u64,
                });
                self.send_radio_msg(ClientRadioMessage::SetTargetFrequency {
                    target_freq_hz: new_target as u64,
                });
                self.send_radio_msg(ClientRadioMessage::SetDemodMode { mode });
                if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
                    self.send_radio_msg(ClientRadioMessage::SetSideband {
                        sideband: state.sideband,
                    });
                }
            }
        }

        // N2ADR filter board enable (persisted via source-control prefs; the
        // server programs the band filter from the tuned frequency).
        let mut n2adr = state.source_control.n2adr_enabled;
        if ui.checkbox(&mut n2adr, "N2ADR Filter Board").changed() {
            state.source_control.n2adr_enabled = n2adr;
            self.send_radio_msg(ClientRadioMessage::SetSourceN2adrEnabled { enabled: n2adr });
            save = true;
        }

        save
    }

    /// FDX / TX Monitor Spectrum: a single enable checkbox.  When enabled the
    /// server keeps the RX spectrum and waterfall live during Spot/SWR (the
    /// transmit carrier becomes visible) instead of freezing.  Visual-only — it
    /// does not change audio.  Persisted via the source-control prefs.  Returns
    /// `true` when the enable changed (so the caller persists it).
    fn draw_fdx_control(&self, ui: &mut egui::Ui, state: &mut UiState) -> bool {
        let mut save = false;
        ui.separator();
        ui.label("FDX");

        let mut fdx = state.source_control.fdx_enabled;
        if ui.checkbox(&mut fdx, "TX Monitor Spectrum").changed() {
            state.source_control.fdx_enabled = fdx;
            self.send_radio_msg(ClientRadioMessage::SetSourceFdxEnabled { enabled: fdx });
            save = true;
        }
        ui.label(
            egui::RichText::new("Keep RX spectrum/waterfall live during Spot/SWR (visual only).")
                .small()
                .weak(),
        );

        save
    }

    /// TX Test Tone (FDX Phase 2): transmit a pure SSB tone to visually verify
    /// USB/LSB placement, carrier suppression and bandwidth.  Amplitude is the
    /// Spot Level; drive is the TX Drive.  Diagnostic only — no audio is played.
    /// Tone settings are client-local (not persisted).
    fn draw_tx_test_tone_section(&self, ui: &mut egui::Ui, state: &mut UiState) {
        ui.separator();

        ui.checkbox(&mut state.tx_tone_enabled, "TX Test Tone");
        if !state.tx_tone_enabled {
            // Hiding the section while a tone runs must also stop it.
            if state.tx_tone_running {
                self.send_radio_msg(ClientRadioMessage::StopTxTestTone);
                state.tx_tone_running = false;
            }
            return;
        }

        // Mode: USB / LSB.
        ui.horizontal(|ui| {
            ui.label("Mode");
            ui.radio_value(&mut state.tx_tone_usb, true, "USB");
            ui.radio_value(&mut state.tx_tone_usb, false, "LSB");
        });

        // Tone frequency (Hz).
        ui.horizontal(|ui| {
            ui.label("Tone");
            ui.add(
                egui::DragValue::new(&mut state.tx_tone_freq_hz)
                    .speed(10.0)
                    .range(100.0..=12_000.0)
                    .fixed_decimals(0)
                    .suffix(" Hz"),
            );
        });

        // Visibility hint: the spectrum spans ±sample_rate/2, so a low tone at a
        // high sample rate sits right on the carrier centre-spike and is hard to
        // see (it is still transmitted correctly — this is purely visual).  Warn
        // adaptively when the tone is too close to centre.
        let sr_hz = state.source_control.sample_rate_hz as f32;
        if sr_hz > 0.0 {
            let off_center_pct = state.tx_tone_freq_hz / (sr_hz / 2.0) * 100.0;
            if off_center_pct < 3.0 {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ At {:.0} kHz sample rate a {:.0} Hz tone is only {:.2}% off centre — \
                         hard to see. Use a ~10 kHz tone, drop to 48 kHz, or zoom the spectrum.",
                        sr_hz / 1000.0,
                        state.tx_tone_freq_hz,
                        off_center_pct,
                    ))
                    .small()
                    .color(egui::Color32::from_rgb(255, 200, 50)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!(
                        "Tone is {:.1}% off centre at {:.0} kHz — visible on the spectrum.",
                        off_center_pct,
                        sr_hz / 1000.0,
                    ))
                    .small()
                    .weak(),
                );
            }
        }

        // Start / Stop.
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!state.tx_tone_running, egui::Button::new("Start Tone"))
                .clicked()
            {
                self.send_radio_msg(ClientRadioMessage::StartTxTestTone {
                    tone_hz: state.tx_tone_freq_hz,
                    usb: state.tx_tone_usb,
                });
                state.tx_tone_running = true;
            }
            if ui
                .add_enabled(state.tx_tone_running, egui::Button::new("Stop Tone"))
                .clicked()
            {
                self.send_radio_msg(ClientRadioMessage::StopTxTestTone);
                state.tx_tone_running = false;
            }
        });

        if state.tx_tone_running {
            ui.label(
                egui::RichText::new("● Transmitting tone…")
                    .small()
                    .color(egui::Color32::from_rgb(100, 220, 100)),
            );
        }
        if !state.source_control.fdx_enabled {
            ui.label(
                egui::RichText::new("Enable FDX to see the tone on the spectrum/waterfall.")
                    .small()
                    .weak(),
            );
        }
    }

    /// SWR Sweep section: editable Start/Stop (MHz), a Run/Cancel button, and a
    /// live progress line.  Reuses the existing Spot/SWR path on the server at a
    /// fixed [`SWR_SWEEP_POINTS`] points; uses the current TX Drive unchanged.
    /// Start/Stop are *not* persisted.  Results open in a separate popup window.
    fn draw_swr_sweep_section(&self, ui: &mut egui::Ui, state: &mut UiState) {
        ui.separator();
        ui.label("SWR Sweep");

        let running = state.swr_sweep_progress.map(|p| p.running).unwrap_or(false);

        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal(|ui| {
                ui.label("Start");
                ui.add(
                    egui::DragValue::new(&mut state.swr_sweep_start_mhz)
                        .speed(0.001)
                        .range(0.0..=60.0)
                        .fixed_decimals(6)
                        .suffix(" MHz"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Stop ");
                ui.add(
                    egui::DragValue::new(&mut state.swr_sweep_stop_mhz)
                        .speed(0.001)
                        .range(0.0..=60.0)
                        .fixed_decimals(6)
                        .suffix(" MHz"),
                );
            });
        });

        if running {
            let (done, total) = state
                .swr_sweep_progress
                .map(|p| (p.done, p.total))
                .unwrap_or((0, SWR_SWEEP_POINTS));
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(format!("Sweeping… {done}/{total}"));
                if ui.button("Cancel").clicked() {
                    self.send_radio_msg(ClientRadioMessage::CancelSwrSweep);
                }
            });
        } else if ui.button("Run Sweep").clicked() {
            let start_hz = (state.swr_sweep_start_mhz * 1_000_000.0).round() as u64;
            let stop_hz = (state.swr_sweep_stop_mhz * 1_000_000.0).round() as u64;
            match validate_sweep_range(start_hz, stop_hz) {
                Ok(()) => {
                    state.swr_sweep_error = None;
                    self.send_radio_msg(ClientRadioMessage::RequestSwrSweep { start_hz, stop_hz });
                }
                Err(msg) => {
                    state.swr_sweep_error = Some(msg);
                }
            }
        }

        if let Some(err) = &state.swr_sweep_error {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
        }

        if state.swr_sweep_result.is_some() && ui.button("Show Last Results").clicked() {
            state.show_swr_sweep_window = true;
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
