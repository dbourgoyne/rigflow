use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::UiState;
use crate::ui::app::RigflowApp;
use crate::ui::panels::sections::{Panel, Section};
use crate::ui::state::DebounceState;
use crate::ui::utils::should_send_debounced;
use eframe::egui;
use egui::RichText;
use rigflow_core::dsp::modes::{
    DeemphasisMode, DemodMode, Sideband, clamp_filter_bandwidth, default_deemphasis_mode,
    filter_bandwidth_limits, pitch_limits,
};
use rigflow_core::radio::vfo::VfoSelect;
use rigflow_protocol::radio_control::ClientRadioMessage;

/// Routes the Receive-section controls to VFO A or VFO B under dual-watch.
///
/// Each accessor returns a `&mut` to the active VFO's `UiState` field, and each
/// `*_msg` builds the matching A or B `ClientRadioMessage`.  The draw helpers
/// keep their "read field → on change, write field + send message" shape; only
/// the field they touch and the message they send vary by VFO.  When dual-watch
/// is off the panel always builds `RxTargets { vfo: A }`, so a stale
/// `active_control_vfo == B` can never misroute an edit.
#[derive(Clone, Copy)]
struct RxTargets {
    vfo: VfoSelect,
}

macro_rules! rx_field {
    ($name:ident, $ty:ty, $a:ident, $b:ident) => {
        fn $name<'a>(self, s: &'a mut UiState) -> &'a mut $ty {
            match self.vfo {
                VfoSelect::A => &mut s.$a,
                VfoSelect::B => &mut s.$b,
            }
        }
    };
}

impl RxTargets {
    fn is_b(self) -> bool {
        matches!(self.vfo, VfoSelect::B)
    }

    rx_field!(
        deemphasis_mode,
        DeemphasisMode,
        deemphasis_mode,
        vfo_b_deemphasis_mode
    );
    rx_field!(
        squelch_enabled,
        bool,
        squelch_enabled,
        vfo_b_squelch_enabled
    );
    rx_field!(
        squelch_threshold_db,
        f32,
        squelch_threshold_db,
        vfo_b_squelch_threshold_db
    );
    rx_field!(nr2_enabled, bool, nr2_enabled, vfo_b_nr2_enabled);
    rx_field!(nr2_strength, f32, nr2_strength, vfo_b_nr2_strength);
    rx_field!(nb_enabled, bool, nb_enabled, vfo_b_nb_enabled);
    rx_field!(nb_threshold, f32, nb_threshold, vfo_b_nb_threshold);
    rx_field!(
        notch_auto_enabled,
        bool,
        notch_auto_enabled,
        vfo_b_notch_auto_enabled
    );
    rx_field!(agc_enabled, bool, agc_enabled, vfo_b_agc_enabled);
    rx_field!(agc_strength, f32, agc_strength, vfo_b_agc_strength);
    rx_field!(
        filter_bandwidth,
        f32,
        filter_bandwidth_hz,
        vfo_b_filter_bandwidth_hz
    );

    /// Pitch field for the active VFO.  VFO A uses one `pitch_hz` (the server
    /// routes it by mode); VFO B keeps SSB and CW pitch separate, so pick by the
    /// active demod mode.
    fn pitch<'a>(self, s: &'a mut UiState, mode: DemodMode) -> &'a mut f32 {
        match self.vfo {
            VfoSelect::A => &mut s.pitch_hz,
            VfoSelect::B => match mode {
                DemodMode::Cwu | DemodMode::Cwl => &mut s.vfo_b_cw_pitch_hz,
                _ => &mut s.vfo_b_ssb_pitch_hz,
            },
        }
    }

    fn squelch_enabled_msg(self, enabled: bool) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetSquelchEnabled { enabled },
            VfoSelect::B => ClientRadioMessage::SetVfoBSquelchEnabled { enabled },
        }
    }
    fn squelch_threshold_msg(self, threshold_db: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetSquelchThreshold { threshold_db },
            VfoSelect::B => ClientRadioMessage::SetVfoBSquelchThreshold { threshold_db },
        }
    }
    fn nr2_enabled_msg(self, enabled: bool) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetNr2Enabled { enabled },
            VfoSelect::B => ClientRadioMessage::SetVfoBNr2Enabled { enabled },
        }
    }
    fn nr2_strength_msg(self, strength: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetNr2Strength { strength },
            VfoSelect::B => ClientRadioMessage::SetVfoBNr2Strength { strength },
        }
    }
    fn nb_enabled_msg(self, enabled: bool) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetNoiseBlankerEnabled { enabled },
            VfoSelect::B => ClientRadioMessage::SetVfoBNoiseBlankerEnabled { enabled },
        }
    }
    fn nb_threshold_msg(self, threshold: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetNoiseBlankerThreshold { threshold },
            VfoSelect::B => ClientRadioMessage::SetVfoBNoiseBlankerThreshold { threshold },
        }
    }
    fn notch_auto_msg(self, enabled: bool) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetNotchAutoEnabled { enabled },
            VfoSelect::B => ClientRadioMessage::SetVfoBNotchAutoEnabled { enabled },
        }
    }
    fn agc_enabled_msg(self, enabled: bool) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetAgcEnabled { enabled },
            VfoSelect::B => ClientRadioMessage::SetVfoBAgcEnabled { enabled },
        }
    }
    fn agc_strength_msg(self, strength: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetAgcStrength { strength },
            VfoSelect::B => ClientRadioMessage::SetVfoBAgcStrength { strength },
        }
    }
    fn deemphasis_msg(self, mode: DeemphasisMode) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetDeemphasisMode { mode },
            VfoSelect::B => ClientRadioMessage::SetVfoBDeemphasisMode { mode },
        }
    }
    fn filter_bandwidth_msg(self, bandwidth_hz: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetFilterBandwidth { bandwidth_hz },
            VfoSelect::B => ClientRadioMessage::SetVfoBFilterBandwidth { bandwidth_hz },
        }
    }
    fn pitch_msg(self, pitch_hz: f32) -> ClientRadioMessage {
        match self.vfo {
            VfoSelect::A => ClientRadioMessage::SetPitch { pitch_hz },
            VfoSelect::B => ClientRadioMessage::SetVfoBPitch { pitch_hz },
        }
    }
}

impl RigflowApp {
    pub(crate) fn draw_radio_control_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if !snapshot.radio_acquired {
            return;
        }

        // Live telemetry (S-meter, etc.) now lives in the top status bar; the
        // left panel holds configuration and controls only.

        egui::CollapsingHeader::new(super::panel_header("Radio Control"))
            .default_open(true)
            .show(ui, |ui| {
                // Preamble (runs every frame, independent of section state):
                // apply persisted per-demod controls when the mode changes.
                if let Ok(mut state) = self.state.lock() {
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

                    // On radio acquire, also replay the restored mode / sideband /
                    // squelch / NR2 / AGC to the server's DSP (these are not part
                    // of the per-demod prefs that `apply_mode_preferences` resends).
                    if state.pending_apply_radio_settings {
                        state.pending_apply_radio_settings = false;
                        self.send_radio_msg(ClientRadioMessage::SetCenterFrequency {
                            center_freq_hz: state.center_freq_hz as u64,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetTargetFrequency {
                            target_freq_hz: state.target_freq_hz as u64,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetDemodMode {
                            mode: snapshot.demod_mode,
                        });
                        if matches!(
                            snapshot.demod_mode,
                            DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU
                        ) {
                            self.send_radio_msg(ClientRadioMessage::SetSideband {
                                sideband: state.sideband,
                            });
                        }
                        self.send_radio_msg(ClientRadioMessage::SetSquelchEnabled {
                            enabled: state.squelch_enabled,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetSquelchThreshold {
                            threshold_db: state.squelch_threshold_db,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetNr2Enabled {
                            enabled: state.nr2_enabled,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetNr2Strength {
                            strength: state.nr2_strength,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetNoiseBlankerEnabled {
                            enabled: state.nb_enabled,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetNoiseBlankerThreshold {
                            threshold: state.nb_threshold,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetNotchAutoEnabled {
                            enabled: state.notch_auto_enabled,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetAgcEnabled {
                            enabled: state.agc_enabled,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetAgcStrength {
                            strength: state.agc_strength,
                        });
                        // TX processing is server-side — replay it too (CW decode
                        // is client-side, already restored on acquire).
                        self.send_radio_msg(ClientRadioMessage::SetTxLimiter {
                            enabled: state.tx_limiter_enabled,
                            threshold_percent: state.tx_limiter_threshold_percent as f32,
                        });
                        self.send_radio_msg(ClientRadioMessage::SetCompression {
                            enabled: state.compressor_enabled,
                            level: state.compressor_level,
                        });
                    }
                }

                // Sub-sections are drawn in weight order by the shared composer:
                // Audio · Receive · Transmit · [Advanced · Diagnostics] · checkbox.
                self.render_panel_sections(
                    ui,
                    snapshot,
                    Panel::RadioControl,
                    snapshot.show_advanced,
                );
            });
    }

    /// Draw one Radio Control sub-section body (dispatched by the weighted
    /// composer in [`super::sections`]).  Each arm does its own per-operator save.
    pub(crate) fn draw_radio_section_body(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &UiState,
        section: Section,
    ) {
        match section {
            // Audio (volume) — used constantly.
            Section::Audio => {
                let mut save_volume = false;
                if let Ok(mut state) = self.state.lock() {
                    save_volume = self.draw_volume_row(ui, &mut state);
                }
                if save_volume {
                    self.save_volume_to_current_operator();
                }
            }

            // Receive: frequently-used RX controls.  Under dual-watch an
            // "Active VFO: A | B" selector chooses which receiver every control
            // below edits; otherwise the controls always edit VFO A.
            Section::Receive => {
                let active_vfo = if snapshot.dual_watch_enabled {
                    self.draw_active_vfo_selector(ui, snapshot)
                } else {
                    VfoSelect::A
                };
                let t = RxTargets { vfo: active_vfo };
                // The mode/filter/pitch/deemphasis controls follow the active
                // VFO's own demod mode.
                let eff_mode = match active_vfo {
                    VfoSelect::A => snapshot.demod_mode,
                    VfoSelect::B => snapshot.vfo_b_demod_mode,
                };
                // Demod mode buttons first (locks state internally → must be
                // outside the lock below to avoid a deadlock).  Gated by the
                // global settings lock (demod mode affects TX sideband/mode).
                let mut save_demod_prefs = ui
                    .add_enabled_ui(!snapshot.config_locked, |ui| {
                        self.draw_demod_selector(ui, snapshot, t)
                    })
                    .inner;
                if let Ok(mut state) = self.state.lock() {
                    save_demod_prefs |= self.draw_filter_bandwidth_row(ui, &mut state, eff_mode, t);
                    save_demod_prefs |= self.draw_pitch_row(ui, &mut state, eff_mode, t);
                    save_demod_prefs |= self.draw_deemphasis_row(ui, &mut state, eff_mode, t);
                    self.draw_squelch_row(ui, &mut state, t);
                    self.draw_nr2_row(ui, &mut state, t);
                    self.draw_agc_row(ui, &mut state, t);
                    self.draw_noise_blanker_row(ui, &mut state, t);
                    self.draw_notch_row(ui, &mut state, t);
                    self.draw_cw_decode_row(ui, &mut state, eff_mode);
                }
                if save_demod_prefs {
                    self.save_demod_preferences_to_current_operator();
                }
            }

            // Transmit: how you transmit — mic (SSB) + CW message/macros/sidetone.
            Section::Transmit => {
                let mut save_mic = false;
                let mut save_cw = false;
                if let Ok(mut state) = self.state.lock() {
                    save_mic = self.draw_microphone_row(ui, &mut state);
                    self.draw_voice_keyer_row(ui, &mut state, snapshot.demod_mode);
                    save_cw |= self.draw_cw_message_row(ui, &mut state, snapshot.demod_mode);
                    save_cw |= self.draw_cw_macros_row(ui, &mut state, snapshot.demod_mode);
                    self.draw_cw_sidetone_row(ui, &mut state, snapshot.demod_mode);
                }
                if save_cw {
                    self.save_cw_message_to_current_operator();
                }
                if save_mic {
                    self.save_mic_settings_to_current_operator();
                }
            }

            // Advanced: normal-operation controls rarely changed.  TX Processing
            // (limiter/compressor) + the one-time WSJT-X / FT8 setup helper.
            Section::Advanced => {
                // The whole Advanced section is TX-gated by the composer
                // (`present_sections`): both TX Processing and the WSJT-X/FT8
                // setup helper are part of the transmit workflow.
                if let Ok(mut state) = self.state.lock() {
                    self.draw_tx_processing_row(ui, &mut state, snapshot.demod_mode);
                }
                ui.separator();
                if ui.button("WSJT-X / FT8 Setup…").clicked() {
                    if let Ok(mut state) = self.state.lock() {
                        state.show_wsjtx_setup_window = true;
                    }
                }
            }

            // Diagnostics: system-testing tools + their read-only meters.
            Section::Diagnostics => {
                if let Ok(mut state) = self.state.lock() {
                    self.draw_two_tone_test_row(ui, &mut state, snapshot.demod_mode);
                    self.draw_tx_audio_diag_row(ui, &mut state, snapshot.demod_mode);
                }
            }

            // No other section is listed for Radio Control.
            _ => {}
        }
    }

    /// "Active VFO: A | B" selector shown at the top of the Receive section
    /// under dual-watch.  Returns the selected VFO; updates only the client-local
    /// `active_control_vfo` (no message is sent).
    fn draw_active_vfo_selector(&self, ui: &mut egui::Ui, snapshot: &UiState) -> VfoSelect {
        let mut sel = snapshot.active_control_vfo;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Active VFO").size(11.0).strong());
            ui.selectable_value(&mut sel, VfoSelect::A, "A");
            ui.selectable_value(&mut sel, VfoSelect::B, "B");
            ui.label(RichText::new("(controls below)").size(11.0));
        });
        if sel != snapshot.active_control_vfo {
            if let Ok(mut state) = self.state.lock() {
                state.active_control_vfo = sel;
            }
        }
        ui.separator();
        sel
    }

    /// Receive squelch: enable checkbox, threshold slider, and a live gate
    /// indicator.  These are radio (DSP) controls sent to the server; they are
    /// not persisted as demod preferences.
    fn draw_squelch_row(&self, ui: &mut egui::Ui, state: &mut UiState, t: RxTargets) {
        ui.separator();

        ui.horizontal(|ui| {
            let mut enabled = *t.squelch_enabled(state);
            if ui.checkbox(&mut enabled, "Squelch").changed() {
                *t.squelch_enabled(state) = enabled;
                self.send_radio_msg(t.squelch_enabled_msg(enabled));
            }

            // Live gate indicator from the server-reported open state (per VFO).
            let gate_open = if t.is_b() {
                state.vfo_b_squelch_open
            } else {
                state.squelch_open
            };
            let (text, color) = if !*t.squelch_enabled(state) {
                ("—", egui::Color32::GRAY)
            } else if gate_open {
                ("● open", egui::Color32::from_rgb(100, 220, 100))
            } else {
                ("muted", egui::Color32::from_rgb(210, 130, 130))
            };
            ui.label(RichText::new(text).color(color).small());
        });

        let enabled = *t.squelch_enabled(state);
        ui.add_enabled_ui(enabled, |ui| {
            let mut threshold_db = *t.squelch_threshold_db(state);
            let mut response = ui.add(
                egui::Slider::new(&mut threshold_db, -120.0..=0.0)
                    .step_by(1.0)
                    .fixed_decimals(0)
                    .suffix(" dBFS")
                    .text("Threshold"),
            );
            super::slider_scroll(ui, &mut response, &mut threshold_db, -120.0, 0.0, 1.0);
            if response.changed() {
                let v = threshold_db.clamp(-120.0, 0.0);
                *t.squelch_threshold_db(state) = v;
                self.send_radio_msg(t.squelch_threshold_msg(v));
            }
        });
    }

    /// NR2 spectral noise reduction enable.  A radio (DSP) control sent to the
    /// server; applied to demodulated receive audio.  Not persisted.
    fn draw_nr2_row(&self, ui: &mut egui::Ui, state: &mut UiState, t: RxTargets) {
        let mut enabled = *t.nr2_enabled(state);
        if ui.checkbox(&mut enabled, "NR2 noise reduction").changed() {
            *t.nr2_enabled(state) = enabled;
            self.send_radio_msg(t.nr2_enabled_msg(enabled));
        }

        ui.add_enabled_ui(*t.nr2_enabled(state), |ui| {
            let mut strength = *t.nr2_strength(state);
            let mut response = ui.add(
                egui::Slider::new(&mut strength, 0.0..=1.0)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .text("NR2 Strength"),
            );
            super::slider_scroll(ui, &mut response, &mut strength, 0.0, 1.0, 0.05);
            if response.changed() {
                let v = strength.clamp(0.0, 1.0);
                *t.nr2_strength(state) = v;
                self.send_radio_msg(t.nr2_strength_msg(v));
            }
        });
    }

    /// Impulse noise blanker enable + level.  Radio (DSP) control sent to the
    /// server; persisted per radio via the background autosave.
    fn draw_noise_blanker_row(&self, ui: &mut egui::Ui, state: &mut UiState, t: RxTargets) {
        ui.separator();
        let mut enabled = *t.nb_enabled(state);
        if ui.checkbox(&mut enabled, "Noise blanker").changed() {
            *t.nb_enabled(state) = enabled;
            self.send_radio_msg(t.nb_enabled_msg(enabled));
        }

        ui.add_enabled_ui(*t.nb_enabled(state), |ui| {
            let mut threshold = *t.nb_threshold(state);
            let mut response = ui.add(
                egui::Slider::new(&mut threshold, 0.0..=1.0)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .text("NB Level"),
            );
            super::slider_scroll(ui, &mut response, &mut threshold, 0.0, 1.0, 0.05);
            if response.changed() {
                let v = threshold.clamp(0.0, 1.0);
                *t.nb_threshold(state) = v;
                self.send_radio_msg(t.nb_threshold_msg(v));
            }
        });
    }

    /// Adaptive auto-notch enable (nulls steady carriers) plus a "Restore Default"
    /// for the noise-blanker + notch group.  Radio (DSP) controls; persisted per radio.
    fn draw_notch_row(&self, ui: &mut egui::Ui, state: &mut UiState, t: RxTargets) {
        let mut enabled = *t.notch_auto_enabled(state);
        if ui.checkbox(&mut enabled, "Auto notch").changed() {
            *t.notch_auto_enabled(state) = enabled;
            self.send_radio_msg(t.notch_auto_msg(enabled));
        }

        // Defaults must match `UiState::default` / the persistence serde defaults.
        const DEF_NB_ENABLED: bool = false;
        const DEF_NB_THRESHOLD: f32 = 0.5;
        const DEF_NOTCH_AUTO_ENABLED: bool = false;
        let at_default = *t.nb_enabled(state) == DEF_NB_ENABLED
            && (*t.nb_threshold(state) - DEF_NB_THRESHOLD).abs() < f32::EPSILON
            && *t.notch_auto_enabled(state) == DEF_NOTCH_AUTO_ENABLED;
        if ui
            .add_enabled(
                !at_default,
                egui::Button::new(RichText::new("Restore Default").size(8.0)),
            )
            .clicked()
        {
            *t.nb_enabled(state) = DEF_NB_ENABLED;
            *t.nb_threshold(state) = DEF_NB_THRESHOLD;
            *t.notch_auto_enabled(state) = DEF_NOTCH_AUTO_ENABLED;
            self.send_radio_msg(t.nb_enabled_msg(DEF_NB_ENABLED));
            self.send_radio_msg(t.nb_threshold_msg(DEF_NB_THRESHOLD));
            self.send_radio_msg(t.notch_auto_msg(DEF_NOTCH_AUTO_ENABLED));
        }
    }

    /// One labelled receive-volume slider (0–100%).  Returns the new value when
    /// the user moved it, else `None`.
    /// One volume control: `[slider %]  [mute button]  [label]`.  The slider shows
    /// 0 while muted (the real level stays in `current`).  Returns the new volume
    /// if the slider moved, and whether the mute button was clicked this frame.
    /// (Glyph note: 🔊/🔇 come from the emoji font — swap for text if they tofu.)
    fn volume_control(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        current: u8,
        muted: bool,
    ) -> (Option<u8>, bool) {
        let mut new_vol = None;
        let mut toggled = false;
        ui.horizontal(|ui| {
            let mut volume = if muted { 0 } else { current as i32 };
            let mut response = ui.add(
                egui::Slider::new(&mut volume, 0..=100)
                    .integer()
                    .suffix("%"),
            );
            super::slider_scroll(ui, &mut response, &mut volume, 0.0, 100.0, 1.0);
            if response.changed() {
                new_vol = Some(volume.clamp(0, 100) as u8);
            }
            let (icon, hover) = if muted {
                ("🔇", "Muted — click to unmute")
            } else {
                ("🔊", "Click to mute")
            };
            if ui
                .add(egui::Button::new(icon).small())
                .on_hover_text(hover)
                .clicked()
            {
                toggled = true;
            }
            ui.label(label);
        });
        (new_vol, toggled)
    }

    /// Receive-audio volume.  Off dual-watch this is the single "Volume" control
    /// (sends `SetVolume`).  Under dual-watch it expands to always-visible
    /// "VFO A" + "VFO B" controls so both levels are mixed live; VFO B's volume is
    /// applied client-side only (no server message).  Each has its own mute button
    /// (client-side, remembers the level).  Returns `true` when a *volume* changed
    /// (mute toggles are session-only and don't persist).
    fn draw_volume_row(&self, ui: &mut egui::Ui, state: &mut UiState) -> bool {
        ui.separator();

        if !state.dual_watch_enabled {
            let (nv, tog) =
                self.volume_control(ui, "Volume", state.volume_percent, state.volume_muted);
            if tog {
                state.volume_muted = !state.volume_muted;
            }
            if let Some(v) = nv {
                state.volume_percent = v;
                state.volume_muted = false; // dragging the slider unmutes
                self.send_radio_msg(ClientRadioMessage::SetVolume { volume_percent: v });
                return true;
            }
            return false;
        }

        let mut changed = false;
        let (nv, tog) = self.volume_control(ui, "VFO A", state.volume_percent, state.volume_muted);
        if tog {
            state.volume_muted = !state.volume_muted;
        }
        if let Some(v) = nv {
            state.volume_percent = v;
            state.volume_muted = false;
            self.send_radio_msg(ClientRadioMessage::SetVolume { volume_percent: v });
            changed = true;
        }
        let (nvb, togb) =
            self.volume_control(ui, "VFO B", state.volume_percent_b, state.volume_b_muted);
        if togb {
            state.volume_b_muted = !state.volume_b_muted;
        }
        if let Some(v) = nvb {
            // Client-side only — applied to the right channel in the audio
            // callback; the server streams full-level audio.
            state.volume_percent_b = v;
            state.volume_b_muted = false;
            changed = true;
        }
        changed
    }

    /// CW controls shown only in CWU/CWL: Sidetone Volume (client-local, drives
    /// the locally generated sidetone; never sent to the server) and Hang Time
    /// (semi break-in — sent to the server, controls how long PTT persists after
    /// the last element).  CW Pitch is the existing pitch row above.
    /// SSB voice keyer: record a clip from the mic (local, no transmit), preview
    /// it through the speakers, delete it, and transmit the selected clip on the
    /// current frequency.  Every action just sets a request flag processed in
    /// `update()`; the safety-critical keying lives in `voice_keyer.rs`.
    fn draw_voice_keyer_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        ui.separator();
        ui.label("Voice Keyer");

        // Precompute everything that reads `state`, so the closures below only
        // write request flags / the name field (avoids overlapping borrows).
        let ssb = matches!(mode, DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU);
        let playing = state.voice_keyer.is_playing();
        let progress = state.voice_keyer.progress();
        let previewing = state.clip_preview.is_active();
        let recording = state.clip_recording;
        let clip_elapsed = state.clip_rec_status.elapsed_secs;
        let clips = state.voice_keyer_clips.clone();
        let selected = state.voice_keyer_clip.clone();
        let have_clip = !selected.is_empty();
        let tx_ready =
            ssb && state.radio_acquired && state.source_capabilities.supports_tx_tune_test;
        let busy = playing || previewing;

        // --- Record a new clip from the mic (local; never transmits) --------
        ui.horizontal(|ui| {
            ui.label("New clip:");
            ui.add_enabled(
                !recording,
                egui::TextEdit::singleline(&mut state.clip_name_input)
                    .hint_text("name")
                    .desired_width(120.0),
            );
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!recording && !busy, egui::Button::new("Record"))
                .clicked()
            {
                state.clip_rec_request = Some(true);
            }
            if ui
                .add_enabled(recording, egui::Button::new("Stop"))
                .clicked()
            {
                state.clip_rec_request = Some(false);
            }
            if recording {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 90, 90),
                    format!("● Recording {clip_elapsed}s"),
                );
            }
        });

        // --- Select / preview / delete -------------------------------------
        let mut new_selected: Option<String> = None;
        egui::ComboBox::from_label("Clip")
            .selected_text(if selected.is_empty() {
                "— none —".to_string()
            } else {
                selected.clone()
            })
            .show_ui(ui, |ui| {
                for c in &clips {
                    if ui.selectable_label(&selected == c, c).clicked() {
                        new_selected = Some(c.clone());
                    }
                }
            });
        if let Some(sel) = new_selected {
            state.voice_keyer_clip = sel;
            state.voice_keyer_clip_dirty = true;
        }

        ui.horizontal(|ui| {
            if previewing {
                if ui.button("Stop preview").clicked() {
                    state.clip_preview_request = Some(false);
                }
            } else if ui
                .add_enabled(have_clip && !recording, egui::Button::new("Preview"))
                .clicked()
            {
                state.clip_preview_request = Some(true);
            }
            if ui
                .add_enabled(
                    have_clip && !recording && !busy,
                    egui::Button::new("Delete"),
                )
                .clicked()
            {
                state.clip_delete_request = true;
            }
        });

        // --- Transmit / abort ----------------------------------------------
        ui.horizontal(|ui| {
            if playing {
                if ui.button("Abort").clicked() {
                    state.voice_keyer_abort_request = true;
                }
                ui.add(egui::ProgressBar::new(progress).desired_width(120.0));
            } else if ui
                .add_enabled(
                    have_clip && tx_ready && !recording && !previewing,
                    egui::Button::new("Transmit"),
                )
                .clicked()
            {
                state.voice_keyer_play_request = true;
            }
        });

        if playing {
            ui.colored_label(
                egui::Color32::from_rgb(100, 220, 100),
                "● Transmitting clip…",
            );
        } else if !ssb {
            ui.label(
                RichText::new("Switch to USB / LSB / Data to transmit a clip.")
                    .small()
                    .weak(),
            );
        }
        if let Some(err) = &state.voice_keyer_error {
            ui.colored_label(egui::Color32::from_rgb(230, 140, 60), err.clone());
        }
    }

    fn draw_cw_sidetone_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Cwu | DemodMode::Cwl) {
            return;
        }
        ui.separator();

        let mut vol = state.cw_sidetone_volume as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut vol, 0..=100)
                .integer()
                .suffix("%")
                .text("CW Sidetone"),
        );
        super::slider_scroll(ui, &mut response, &mut vol, 0.0, 100.0, 1.0);
        if response.changed() {
            let v = vol.clamp(0, 100) as u8;
            state.cw_sidetone_volume = v; // Reflect immediately into the lock-free audio control.
            state.sidetone.set_volume(v as f32 / 100.0);
        }

        // Semi break-in hang time: 0–2000 ms, step 50.
        let mut hang = state.cw_hang_ms as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut hang, 0..=2000)
                .step_by(50.0)
                .integer()
                .suffix(" ms")
                .text("Hang Time"),
        );
        super::slider_scroll(ui, &mut response, &mut hang, 0.0, 2000.0, 50.0);
        if response.changed() {
            let h = hang.clamp(0, 2000) as u32;
            if h != state.cw_hang_ms {
                state.cw_hang_ms = h;
                self.send_radio_msg(ClientRadioMessage::SetCwHangTime { hang_ms: h });
            }
        }
    }

    /// Text-to-CW: message text, speed, and Send/Stop, shown only in CWU/CWL.
    /// All Morse encoding/timing happens client-side (see `crate::cw_text`); the
    /// server only receives the same StartCwKey/StopCwKey events as Space-bar
    /// keying.  Returns `true` when the message/speed should be persisted.
    fn draw_cw_message_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) -> bool {
        if !matches!(mode, DemodMode::Cwu | DemodMode::Cwl) {
            return false;
        }
        ui.separator();
        ui.label("CW Message");

        let mut save = false;
        let sending = self.cw_text_sending.load(Ordering::Relaxed);

        // Message text (locked out while a message is playing).
        ui.add_enabled_ui(!sending, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.cw_message)
                    .hint_text("text to send")
                    .desired_width(220.0),
            );
        });

        // Sending speed: 5–50 WPM, step 1.
        let mut wpm = state.cw_speed_wpm as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut wpm, 5..=50)
                .integer()
                .suffix(" WPM")
                .text("CW Speed"),
        );
        super::slider_scroll(ui, &mut response, &mut wpm, 5.0, 50.0, 1.0);
        if response.changed() {
            let w = wpm.clamp(5, 50) as u32;
            if w != state.cw_speed_wpm {
                state.cw_speed_wpm = w;
                save = true;
            }
        }

        ui.horizontal(|ui| {
            let can_send = !sending && !state.cw_message.trim().is_empty();
            if ui
                .add_enabled(can_send, egui::Button::new("Send"))
                .clicked()
            {
                // Spawn the client-side Morse sender; it sends the same CW key
                // events as the Space bar and drives the local sidetone.
                crate::cw_text::spawn_send(
                    state.cw_message.clone(),
                    state.cw_speed_wpm,
                    self.ws_cmd_tx.clone(),
                    Arc::clone(&state.sidetone),
                    Arc::clone(&self.cw_text_abort),
                    Arc::clone(&self.cw_text_sending),
                );
                save = true; // persist the just-sent message + speed
            }
            if ui.add_enabled(sending, egui::Button::new("Stop")).clicked() {
                // Abort promptly; the server's semi break-in releases PTT.
                self.cw_text_abort.store(true, Ordering::Relaxed);
            }
            if sending {
                ui.label(RichText::new("● sending…").small());
            }
        });

        save
    }

    /// CW memory macros (F1–F4), shown only in CWU/CWL: 4 buttons that load and
    /// send their text via the existing Text-to-CW path, plus editable
    /// label/text fields.  Empty macros are disabled.  Returns `true` when a
    /// label/text edit should be persisted.
    fn draw_cw_macros_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) -> bool {
        if !matches!(mode, DemodMode::Cwu | DemodMode::Cwl) {
            return false;
        }
        ui.separator();
        ui.label("CW Macros");

        let mut save = false;
        let sending = self.cw_text_sending.load(Ordering::Relaxed);

        // Macro buttons (F1–F4).  Clicking loads the text into the message field
        // and starts sending it.
        ui.horizontal_wrapped(|ui| {
            for i in 0..4 {
                // Build the button label and enabled flag without holding a
                // borrow of `state` across the click handler (which mutates it).
                let label = {
                    let l = state.cw_macros[i].label.trim();
                    if l.is_empty() {
                        format!("F{}", i + 1)
                    } else {
                        format!("F{} {}", i + 1, l)
                    }
                };
                let enabled = !sending && !state.cw_macros[i].text.trim().is_empty();
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    let text = state.cw_macros[i].text.clone();
                    let wpm = state.cw_speed_wpm;
                    let sidetone = Arc::clone(&state.sidetone);
                    state.cw_message = text.clone();
                    self.trigger_cw_text(text, wpm, sidetone);
                    save = true;
                }
            }
        });

        // Editable label + text for each slot.
        for i in 0..4 {
            ui.horizontal(|ui| {
                ui.label(format!("M{}", i + 1));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut state.cw_macros[i].label)
                            .hint_text("label")
                            .desired_width(60.0),
                    )
                    .changed()
                {
                    save = true;
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut state.cw_macros[i].text)
                            .hint_text("macro text")
                            .desired_width(240.0),
                    )
                    .changed()
                {
                    save = true;
                }
            });
        }

        save
    }

    /// CW decode (assistive CW-to-text), shown only in CWU/CWL.  Pushes the
    /// current CW Pitch + speed to the decoder's shared control, exposes the
    /// Enable toggle, the auto-WPM estimate, the scrolling decoded text, and a
    /// Clear button.  The decoder itself runs in the media thread on received
    /// audio; nothing here alters the receive path or transmits.
    fn draw_cw_decode_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Cwu | DemodMode::Cwl) {
            return;
        }
        ui.separator();
        ui.label("CW Decode");

        // Keep the decoder's target tone (CW Pitch) and WPM seed current.
        state.cw_decode.set_pitch_hz(state.pitch_hz);
        state.cw_decode.set_wpm(state.cw_speed_wpm);

        ui.horizontal(|ui| {
            let mut enabled = state.cw_decode.enabled();
            if ui.checkbox(&mut enabled, "Enable Decode").changed() {
                state.cw_decode.set_enabled(enabled);
            }
            ui.label(
                RichText::new(format!("Auto WPM (~{})", state.cw_decode.est_wpm()))
                    .small()
                    .weak(),
            );
        });

        // Scrolling, read-only decoded text (auto-scrolls to the newest).
        let mut text = state.cw_decode.text();
        egui::ScrollArea::vertical()
            .id_salt("cw_decode_text")
            .max_height(100.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });

        if ui.button("Clear").clicked() {
            state.cw_decode.clear();
        }
    }

    /// Microphone section: input device selection, mic gain (0–200%), a live
    /// peak level meter, and a clip indicator (held ~500 ms).  Capture is
    /// client-only and never touches RX/TX/PTT/network (Phase 1 — no RF).
    /// Returns `true` when the device/gain should be persisted.
    fn draw_microphone_row(&self, ui: &mut egui::Ui, state: &mut UiState) -> bool {
        ui.separator();
        ui.label("Microphone");

        let mut save = false;

        // Input device dropdown ("" = system default).
        let devices = state.mic_devices.clone();
        ui.horizontal(|ui| {
            ui.label("Input");
            let mut selected = state.mic_device.clone();
            let current = if selected.is_empty() {
                "System default".to_string()
            } else {
                selected.clone()
            };
            egui::ComboBox::from_id_salt("mic_input_device")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, String::new(), "System default");
                    for name in &devices {
                        ui.selectable_value(&mut selected, name.clone(), name);
                    }
                });
            if selected != state.mic_device {
                state.mic_device = selected;
                save = true; // ensure_mic() restarts capture next frame
            }
        });

        // Mic gain (measurement only this phase).
        let mut gain = state.mic_gain_percent as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut gain, 0..=200)
                .integer()
                .suffix("%")
                .text("Mic Gain"),
        );
        super::slider_scroll(ui, &mut response, &mut gain, 0.0, 200.0, 1.0);
        if response.changed() {
            state.mic_gain_percent = gain.clamp(0, 200) as u16;
            state
                .mic_shared
                .set_gain(state.mic_gain_percent as f32 / 100.0);
            save = true;
        }

        // Level meter (decaying peak) + clip indicator.
        let peak = state.mic_shared.take_peak();
        state.mic_meter = (state.mic_meter * 0.85).max(peak);
        if state.mic_shared.take_clipped() {
            state.mic_clip_until = Some(Instant::now() + Duration::from_millis(500));
        }
        let clipping = state
            .mic_clip_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false);

        ui.horizontal(|ui| {
            ui.add(
                egui::ProgressBar::new(state.mic_meter.min(1.0))
                    .desired_width(180.0)
                    .text(format!("{:.0}%", (state.mic_meter * 100.0).min(999.0))),
            );
            if clipping {
                ui.colored_label(egui::Color32::from_rgb(230, 60, 60), "● CLIP");
            } else {
                ui.colored_label(egui::Color32::DARK_GRAY, "○ clip");
            }
        });

        if !state.mic_status.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 50),
                RichText::new(&state.mic_status).small(),
            );
        }

        save
    }

    /// SSB Two-Tone Test generator (USB/LSB only).  Enable bypasses the mic and
    /// makes the server generate `Tone A + Tone B` through the normal mic-TX
    /// path; transmit with the usual Space-bar PTT.  A standard tool for SSB
    /// quality / IMD / clipping checks via FDX.  Sends `SetTwoToneTest` on any
    /// change.  Tone/level generation and clip behaviour live server-side.
    fn draw_two_tone_test_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
            return;
        }

        ui.separator();
        ui.label("Two-Tone Test");

        let mut changed = false;

        let mut enabled = state.two_tone_enabled;
        if ui.checkbox(&mut enabled, "Enable Two-Tone Test").changed() {
            state.two_tone_enabled = enabled;
            changed = true;
        }

        ui.horizontal(|ui| {
            ui.label("Tone A");
            let mut a = state.two_tone_a_hz;
            if ui
                .add(
                    egui::DragValue::new(&mut a)
                        .range(100.0..=4000.0)
                        .speed(10.0)
                        .suffix(" Hz"),
                )
                .changed()
            {
                state.two_tone_a_hz = a;
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Tone B");
            let mut b = state.two_tone_b_hz;
            if ui
                .add(
                    egui::DragValue::new(&mut b)
                        .range(100.0..=4000.0)
                        .speed(10.0)
                        .suffix(" Hz"),
                )
                .changed()
            {
                state.two_tone_b_hz = b;
                changed = true;
            }
        });

        let mut level = state.two_tone_level_percent as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut level, 0..=100)
                .integer()
                .suffix("%")
                .text("Level"),
        );
        super::slider_scroll(ui, &mut response, &mut level, 0.0, 100.0, 1.0);
        if response.changed() {
            state.two_tone_level_percent = level.clamp(0, 100) as u16;
            changed = true;
        }

        if changed {
            self.send_radio_msg(ClientRadioMessage::SetTwoToneTest {
                enabled: state.two_tone_enabled,
                tone_a_hz: state.two_tone_a_hz,
                tone_b_hz: state.two_tone_b_hz,
                level_percent: state.two_tone_level_percent as f32,
            });
        }
    }

    /// TX Processing (USB/LSB only): the soft peak limiter (ALC Phase 1).
    /// Enable + threshold are operator controls (sent via `SetTxLimiter`); the
    /// gain-reduction meter is read from server telemetry (`tx_audio_diag`).
    fn draw_tx_processing_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
            return;
        }

        ui.separator();
        ui.label("TX Processing");

        let mut changed = false;

        let mut enabled = state.tx_limiter_enabled;
        if ui.checkbox(&mut enabled, "Enable Limiter").changed() {
            state.tx_limiter_enabled = enabled;
            changed = true;
        }

        let mut threshold = state.tx_limiter_threshold_percent as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut threshold, 50..=99)
                .integer()
                .suffix("%")
                .text("Limiter Threshold"),
        );
        super::slider_scroll(ui, &mut response, &mut threshold, 50.0, 99.0, 1.0);
        if response.changed() {
            state.tx_limiter_threshold_percent = threshold.clamp(50, 99) as u16;
            changed = true;
        }

        // Limiter gain-reduction meter (from server telemetry).  Shown as -N dB;
        // the bar fills toward a nominal 20 dB of reduction.
        let gr = state.tx_audio_diag.gain_reduction_db.max(0.0);
        ui.horizontal(|ui| {
            ui.label("Limiter GR");
            ui.add(
                egui::ProgressBar::new((gr / 20.0).min(1.0))
                    .desired_width(160.0)
                    .text(format!("-{gr:.1} dB")),
            );
        });

        if changed {
            self.send_radio_msg(ClientRadioMessage::SetTxLimiter {
                enabled: state.tx_limiter_enabled,
                threshold_percent: state.tx_limiter_threshold_percent as f32,
            });
        }

        // --- Speech compression (before the limiter) ---------------------
        let mut comp_changed = false;

        let mut comp_enabled = state.compressor_enabled;
        if ui
            .checkbox(&mut comp_enabled, "Enable Compression")
            .changed()
        {
            state.compressor_enabled = comp_enabled;
            comp_changed = true;
        }

        let mut level = state.compressor_level as i32;
        let mut response = ui.add(
            egui::Slider::new(&mut level, 0..=10)
                .integer()
                .text("Compression Level"),
        );
        super::slider_scroll(ui, &mut response, &mut level, 0.0, 10.0, 1.0);
        if response.changed() {
            state.compressor_level = level.clamp(0, 10) as u8;
            comp_changed = true;
        }

        // Compressor gain-reduction meter (from server telemetry).
        let cgr = state.tx_audio_diag.compressor_reduction_db.max(0.0);
        ui.horizontal(|ui| {
            ui.label("Compression GR");
            ui.add(
                egui::ProgressBar::new((cgr / 20.0).min(1.0))
                    .desired_width(160.0)
                    .text(format!("-{cgr:.1} dB")),
            );
        });

        if comp_changed {
            self.send_radio_msg(ClientRadioMessage::SetCompression {
                enabled: state.compressor_enabled,
                level: state.compressor_level,
            });
        }

        // Restore the TX-processing controls to their defaults (mirrors the
        // Filter BW "Restore Default" button); greyed out when already at them.
        // Must match `UiState::default` / the persistence defaults.
        const DEF_LIMITER_ENABLED: bool = true;
        const DEF_LIMITER_THRESHOLD_PCT: u16 = 90;
        const DEF_COMPRESSOR_ENABLED: bool = false;
        const DEF_COMPRESSOR_LEVEL: u8 = 3;
        let at_default = state.tx_limiter_enabled == DEF_LIMITER_ENABLED
            && state.tx_limiter_threshold_percent == DEF_LIMITER_THRESHOLD_PCT
            && state.compressor_enabled == DEF_COMPRESSOR_ENABLED
            && state.compressor_level == DEF_COMPRESSOR_LEVEL;
        if ui
            .add_enabled(
                !at_default,
                egui::Button::new(RichText::new("Restore Default").size(8.0)),
            )
            .clicked()
        {
            state.tx_limiter_enabled = DEF_LIMITER_ENABLED;
            state.tx_limiter_threshold_percent = DEF_LIMITER_THRESHOLD_PCT;
            state.compressor_enabled = DEF_COMPRESSOR_ENABLED;
            state.compressor_level = DEF_COMPRESSOR_LEVEL;
            self.send_radio_msg(ClientRadioMessage::SetTxLimiter {
                enabled: DEF_LIMITER_ENABLED,
                threshold_percent: DEF_LIMITER_THRESHOLD_PCT as f32,
            });
            self.send_radio_msg(ClientRadioMessage::SetCompression {
                enabled: DEF_COMPRESSOR_ENABLED,
                level: DEF_COMPRESSOR_LEVEL,
            });
        }
    }

    /// TX Audio Diagnostics (USB/LSB only).  Shows the server-measured audio
    /// feeding the SSB modulator: live RMS level, held peak, a clip indicator,
    /// and underrun/overrun transport counters with a reset button.
    /// Diagnostics only — nothing here changes transmitted audio.
    fn draw_tx_audio_diag_row(&self, ui: &mut egui::Ui, state: &mut UiState, mode: DemodMode) {
        if !matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
            return;
        }

        ui.separator();
        ui.label("TX Audio Diagnostics");

        let diag = state.tx_audio_diag;

        // TX RMS level meter.
        ui.horizontal(|ui| {
            ui.label("Level");
            ui.add(
                egui::ProgressBar::new(diag.rms.min(1.0))
                    .desired_width(160.0)
                    .text(format!("{:.0}%", (diag.rms * 100.0).min(999.0))),
            );
        });

        // TX peak meter + clip indicator.
        ui.horizontal(|ui| {
            ui.label("Peak");
            ui.add(
                egui::ProgressBar::new(diag.peak.min(1.0))
                    .desired_width(160.0)
                    .text(format!("{:.0}%", (diag.peak * 100.0).min(999.0))),
            );
            if diag.clipping {
                ui.colored_label(egui::Color32::from_rgb(230, 60, 60), "● CLIP");
            } else {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "○ ok");
            }
        });

        // Transport-health counters + reset (server-side counters).
        ui.horizontal(|ui| {
            ui.label(format!("Underruns: {}", diag.underruns));
            // `overruns` counts dropped samples (clock-drift surplus), not events.
            ui.label(format!("Overrun drops: {} smp", diag.overruns));
            if ui.button("Reset Counters").clicked() {
                self.send_radio_msg(ClientRadioMessage::ResetTxAudioDiag);
            }
        });
    }

    /// AGC enable + strength.  A radio (DSP) control sent to the server;
    /// applied to demodulated receive audio (before NR2/squelch). Not persisted.
    fn draw_agc_row(&self, ui: &mut egui::Ui, state: &mut UiState, t: RxTargets) {
        ui.separator();

        let mut enabled = *t.agc_enabled(state);
        if ui.checkbox(&mut enabled, "AGC").changed() {
            *t.agc_enabled(state) = enabled;
            self.send_radio_msg(t.agc_enabled_msg(enabled));
        }

        ui.add_enabled_ui(*t.agc_enabled(state), |ui| {
            let mut strength = *t.agc_strength(state);
            let mut response = ui.add(
                egui::Slider::new(&mut strength, 0.0..=1.0)
                    .step_by(0.01)
                    .fixed_decimals(2)
                    .text("AGC Strength"),
            );
            super::slider_scroll(ui, &mut response, &mut strength, 0.0, 1.0, 0.01);
            if response.changed() {
                let v = strength.clamp(0.0, 1.0);
                *t.agc_strength(state) = v;
                self.send_radio_msg(t.agc_strength_msg(v));
            }
        });
    }

    fn draw_filter_bandwidth_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
        t: RxTargets,
    ) -> bool {
        // VFO B's filter is session state — not written to the shared per-demod
        // preferences (which stay VFO-A defaults), so it never sets `save`.
        let persist = !t.is_b();
        let mut save = false;
        let bw_limits = filter_bandwidth_limits(demod_mode);

        *t.filter_bandwidth(state) = clamp_filter_bandwidth(demod_mode, *t.filter_bandwidth(state));
        let mut bw = *t.filter_bandwidth(state);
        let at_default = (bw - bw_limits.default_hz).abs() < 1.0;

        ui.horizontal(|ui| {
            let slider_width = (ui.available_width() - 80.0).max(100.0);

            let mut response = ui.add_sized(
                [slider_width, 0.0],
                egui::Slider::new(&mut bw, bw_limits.min_hz..=bw_limits.max_hz)
                    .text(RichText::new("Filter BW (Hz)").size(11.0)),
            );
            let bw_scrolled = super::slider_scroll(
                ui,
                &mut response,
                &mut bw,
                bw_limits.min_hz as f64,
                bw_limits.max_hz as f64,
                50.0,
            );

            if ui
                .add_enabled(
                    !at_default,
                    egui::Button::new(RichText::new("Restore Default").size(8.0)),
                )
                .clicked()
            {
                let default_hz = bw_limits.default_hz;
                bw = default_hz;
                *t.filter_bandwidth(state) = default_hz;
                if persist {
                    state
                        .demod_preferences
                        .get_mut(demod_mode)
                        .filter_bandwidth_hz = default_hz;
                    save = true;
                }
                state.filter_bw_debounce = DebounceState::new(default_hz);
                self.send_radio_msg(t.filter_bandwidth_msg(default_hz));
            }

            *t.filter_bandwidth(state) = bw;
            if persist {
                state
                    .demod_preferences
                    .get_mut(demod_mode)
                    .filter_bandwidth_hz = bw;
            }

            let now = Instant::now();

            if response.changed() {
                if let Some(send_hz) = should_send_debounced(
                    now,
                    bw,
                    &mut state.filter_bw_debounce,
                    10.0,
                    Duration::from_millis(75),
                ) {
                    self.send_radio_msg(t.filter_bandwidth_msg(send_hz));
                }
            }

            if response.drag_stopped() || bw_scrolled {
                let final_hz = bw.round().clamp(bw_limits.min_hz, bw_limits.max_hz);

                *t.filter_bandwidth(state) = final_hz;
                if persist {
                    state
                        .demod_preferences
                        .get_mut(demod_mode)
                        .filter_bandwidth_hz = final_hz;
                    save = true;
                }
                state.filter_bw_debounce.last_sent_value = final_hz;
                state.filter_bw_debounce.last_send_time = now;
                self.send_radio_msg(t.filter_bandwidth_msg(final_hz));
            }
        });

        save
    }

    fn draw_pitch_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
        t: RxTargets,
    ) -> bool {
        let Some(limits) = pitch_limits(demod_mode) else {
            return false;
        };

        // VFO B's pitch is session state — not persisted to the shared per-demod
        // preferences (see the filter row).
        let persist = !t.is_b();
        let mut save = false;

        {
            let p = t.pitch(state, demod_mode);
            *p = p.clamp(limits.min_hz, limits.max_hz);
        }
        let mut pitch = *t.pitch(state, demod_mode);
        let at_default = (pitch - limits.default_hz).abs() < 1.0;

        ui.horizontal(|ui| {
            let slider_width = (ui.available_width() - 80.0).max(100.0);

            let mut response = ui.add_sized(
                [slider_width, 0.0],
                egui::Slider::new(&mut pitch, limits.min_hz..=limits.max_hz)
                    .text(RichText::new(limits.label).size(11.0)),
            );
            let pitch_scrolled = super::slider_scroll(
                ui,
                &mut response,
                &mut pitch,
                limits.min_hz as f64,
                limits.max_hz as f64,
                10.0,
            );

            if ui
                .add_enabled(
                    !at_default,
                    egui::Button::new(RichText::new("Restore Default").size(8.0)),
                )
                .clicked()
            {
                let default_hz = limits.default_hz;
                pitch = default_hz;
                *t.pitch(state, demod_mode) = default_hz;
                if persist {
                    state.demod_preferences.get_mut(demod_mode).pitch_hz = default_hz;
                    save = true;
                }
                state.pitch_debounce = DebounceState::new(default_hz);
                self.send_radio_msg(t.pitch_msg(default_hz));
            }

            *t.pitch(state, demod_mode) = pitch;
            if persist {
                state.demod_preferences.get_mut(demod_mode).pitch_hz = pitch;
            }

            let now = Instant::now();

            if response.changed() {
                if let Some(send_hz) = should_send_debounced(
                    now,
                    pitch,
                    &mut state.pitch_debounce,
                    limits.debounce_delta_hz,
                    Duration::from_millis(limits.debounce_interval_ms),
                ) {
                    self.send_radio_msg(t.pitch_msg(send_hz));
                }
            }

            if response.drag_stopped() || pitch_scrolled {
                let final_hz = pitch.round().clamp(limits.min_hz, limits.max_hz);
                *t.pitch(state, demod_mode) = final_hz;
                if persist {
                    state.demod_preferences.get_mut(demod_mode).pitch_hz = final_hz;
                    save = true;
                }
                state.pitch_debounce.last_sent_value = final_hz;
                state.pitch_debounce.last_send_time = now;
                self.send_radio_msg(t.pitch_msg(final_hz));
            }
        });

        save
    }

    fn draw_deemphasis_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut UiState,
        demod_mode: DemodMode,
        t: RxTargets,
    ) -> bool {
        if default_deemphasis_mode(demod_mode).is_none() {
            return false;
        }

        // VFO B's deemphasis is session state — not persisted (see the filter row).
        let persist = !t.is_b();
        let mut save = false;
        let mut changed = false;
        let default_mode = default_deemphasis_mode(demod_mode).unwrap();
        let mut mode = *t.deemphasis_mode(state);
        let at_default = mode == default_mode;

        ui.horizontal(|ui| {
            ui.label("Deemphasis");

            egui::ComboBox::from_id_salt("deemphasis_mode_combo")
                .selected_text(mode.label())
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut mode,
                            DeemphasisMode::Off,
                            DeemphasisMode::Off.label(),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut mode,
                            DeemphasisMode::Tau50us,
                            DeemphasisMode::Tau50us.label(),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut mode,
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
                mode = default_mode;
                *t.deemphasis_mode(state) = default_mode;
                if persist {
                    state.demod_preferences.get_mut(demod_mode).deemphasis_mode = default_mode;
                    save = true;
                }
                self.send_radio_msg(t.deemphasis_msg(default_mode));
            }
        });

        if changed {
            *t.deemphasis_mode(state) = mode;
            if persist {
                state.demod_preferences.get_mut(demod_mode).deemphasis_mode = mode;
                save = true;
            }
            self.send_radio_msg(t.deemphasis_msg(mode));
        }

        save
    }

    fn draw_demod_selector(&self, ui: &mut egui::Ui, snapshot: &UiState, t: RxTargets) -> bool {
        let label = if t.is_b() { "Demod (B)" } else { "Demod" };
        ui.label(RichText::new(label).size(15.0).strong());

        let current = if t.is_b() {
            snapshot.vfo_b_demod_mode
        } else {
            snapshot.demod_mode
        };
        let mut selected = current;

        ui.horizontal(|ui| {
            ui.radio_value(&mut selected, DemodMode::Wfm, "WFM");
            ui.radio_value(&mut selected, DemodMode::Nfm, "NFM");
            ui.radio_value(&mut selected, DemodMode::Am, "AM");
            ui.radio_value(&mut selected, DemodMode::Lsb, "LSB");
            ui.radio_value(&mut selected, DemodMode::Usb, "USB");
            ui.radio_value(&mut selected, DemodMode::DgtU, "DATA");
            ui.radio_value(&mut selected, DemodMode::Cwu, "CWU");
            ui.radio_value(&mut selected, DemodMode::Cwl, "CWL");
        });

        if selected == current {
            return false;
        }

        let sideband = match selected {
            DemodMode::Lsb => Sideband::Lsb,
            DemodMode::Usb | DemodMode::DgtU => Sideband::Usb,
            _ => {
                if t.is_b() {
                    snapshot.vfo_b_sideband
                } else {
                    snapshot.sideband
                }
            }
        };
        let is_ssb = matches!(selected, DemodMode::Lsb | DemodMode::Usb | DemodMode::DgtU);

        if t.is_b() {
            // VFO B carries its own mode; on a mode switch load the shared
            // per-demod defaults into VFO B's (session-only) filter/pitch/deemph
            // so the new mode gets a sensible passband.
            let (bw, pitch, deemph) = if let Ok(mut state) = self.state.lock() {
                state.vfo_b_demod_mode = selected;
                state.vfo_b_sideband = sideband;
                let prefs = state.demod_preferences.get(selected);
                let bw = clamp_filter_bandwidth(selected, prefs.filter_bandwidth_hz);
                state.vfo_b_filter_bandwidth_hz = bw;
                match selected {
                    DemodMode::Cwu | DemodMode::Cwl => state.vfo_b_cw_pitch_hz = prefs.pitch_hz,
                    _ => state.vfo_b_ssb_pitch_hz = prefs.pitch_hz,
                }
                state.vfo_b_deemphasis_mode = prefs.deemphasis_mode;
                // Seed VFO B's (independent) grid-snap step from the operator's
                // per-mode default for the new mode.
                state.vfo_b_tuning_step_hz = state.tuning_step_preferences.get(selected);
                (bw, prefs.pitch_hz, prefs.deemphasis_mode)
            } else {
                return false;
            };
            self.send_radio_msg(ClientRadioMessage::SetVfoBDemodMode { mode: selected });
            if is_ssb {
                self.send_radio_msg(ClientRadioMessage::SetVfoBSideband { sideband });
            }
            self.send_radio_msg(ClientRadioMessage::SetVfoBFilterBandwidth { bandwidth_hz: bw });
            if pitch_limits(selected).is_some() {
                self.send_radio_msg(ClientRadioMessage::SetVfoBPitch { pitch_hz: pitch });
            }
            if default_deemphasis_mode(selected).is_some() {
                self.send_radio_msg(ClientRadioMessage::SetVfoBDeemphasisMode { mode: deemph });
            }
            return false;
        }

        if let Ok(mut state) = self.state.lock() {
            state.demod_mode = selected;
            state.sideband = sideband;
        }

        self.send_radio_msg(ClientRadioMessage::SetDemodMode { mode: selected });
        if is_ssb {
            self.send_radio_msg(ClientRadioMessage::SetSideband { sideband });
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

    // RX-routing tie: entering Data-USB enables RX Digital Output (so external
    // digital apps like WSJT-X can record); leaving it disables routing.  Only
    // the transition into/out of Data-USB touches routing — switching among
    // voice modes leaves it exactly as the user set it.  Linux-only: this drives
    // the PipeWire virtual-audio path; on macOS digital RX flows over the
    // always-on TCI tap instead, so there is nothing to toggle here.
    #[cfg(target_os = "linux")]
    {
        let was_dgt = state.last_demod_mode_for_controls == Some(DemodMode::DgtU);
        let is_dgt = mode == DemodMode::DgtU;
        if is_dgt && !was_dgt {
            state.digital_rx.set_enabled(true);
        } else if was_dgt && !is_dgt {
            state.digital_rx.set_enabled(false);
        }
    }

    state.last_demod_mode_for_controls = Some(mode);
}

/// Format a signal level (dBm) as an S-meter label.
///
/// HF convention: S9 = -73 dBm, 6 dB per S-unit.  Below S1 clamps to "S0";
/// above S9 shows "S9+N dB" (N rounded to the nearest 10 dB, as is customary).
pub(crate) fn s_meter_label(dbm: f32) -> String {
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
