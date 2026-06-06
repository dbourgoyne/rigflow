use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::net::control::ControlCommand;
use rigflow_protocol::radio_control::ClientRadioMessage;

use crate::persistence::PersistenceStore;
use crate::sidetone::SidetoneShared;

use crate::ui::state::UiState;

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    pub persistence_store: PersistenceStore,
    pub waterfall_texture: Option<egui::TextureHandle>,

    /// Text-to-CW sender control (shared with its timer thread): `cw_text_abort`
    /// requests a prompt stop; `cw_text_sending` is true while a message plays.
    pub cw_text_abort: Arc<AtomicBool>,
    pub cw_text_sending: Arc<AtomicBool>,

    /// Active microphone capture stream (kept alive here; `None` if not running).
    pub mic: Option<crate::mic::MicCapture>,
    /// The device name we last attempted to open, so we only (re)start capture
    /// when the selection actually changes (and don't retry a failure every
    /// frame).  `""` = system default.
    pub mic_requested: Option<String>,

    /// Virtual digital-mode audio endpoints.  Held purely for its `Drop` (an
    /// RAII guard that unloads the devices this process created on exit), so the
    /// field is never read directly.
    #[allow(dead_code)]
    pub digital_audio: crate::digital_audio::DigitalAudio,
}

impl RigflowApp {
    pub fn new(
        state: Arc<Mutex<UiState>>,
        ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
        waterfall_buffer: Arc<Mutex<Vec<u32>>>,
        spectrum_db: Arc<Mutex<Vec<f32>>>,
        persistence_store: PersistenceStore,
    ) -> Self {
        // Create the virtual digital-audio endpoints once, at startup.
        let digital_audio = crate::digital_audio::DigitalAudio::start();
        let digital_output_available = digital_audio.output_available();
        let digital_rx_available = digital_audio.rx_available();
        let digital_input_available = digital_audio.input_available();

        let app = Self {
            state,
            ws_cmd_tx,
            waterfall_buffer,
            spectrum_db,
            persistence_store,
            waterfall_texture: None,
            cw_text_abort: Arc::new(AtomicBool::new(false)),
            cw_text_sending: Arc::new(AtomicBool::new(false)),
            mic: None,
            mic_requested: None,
            digital_audio,
        };

        // Enumerate input devices once for the dropdown (one-time; cheap enough
        // and avoids re-enumerating every frame).
        let devices = crate::mic::list_input_devices();
        if let Ok(mut state) = app.state.lock() {
            state.mic_devices = devices;
            state.digital_output_available = digital_output_available;
            state.digital_rx_available = digital_rx_available;
            state.digital_input_available = digital_input_available;
        }

        app
    }

    /// (Re)start microphone capture when the selected device changes; push the
    /// current mic gain to the capture each frame.  Capture runs continuously
    /// and never touches the RX/TX/network paths.
    fn ensure_mic(&mut self) {
        let (desired, gain, shared) = {
            let state = self.state.lock().unwrap();
            (
                state.mic_device.clone(),
                state.mic_gain_percent,
                Arc::clone(&state.mic_shared),
            )
        };

        // Gain is applied via the shared atomic — no stream restart needed.
        shared.set_gain(gain as f32 / 100.0);

        // Only (re)start when the requested device differs from our last attempt
        // (this also prevents retrying a failed open on every frame).
        if self.mic_requested.as_deref() == Some(desired.as_str()) {
            return;
        }
        self.mic_requested = Some(desired.clone());

        match crate::mic::start_capture(shared, &desired) {
            Ok(cap) => {
                let warning = if cap.fell_back && !desired.is_empty() {
                    format!("input '{desired}' not found; using {}", cap.device_name)
                } else {
                    String::new()
                };
                if let Ok(mut state) = self.state.lock() {
                    state.mic_status = warning;
                }
                self.mic = Some(cap);
            }
            Err(err) => {
                log::warn!("[mic] capture start failed: {err}");
                if let Ok(mut state) = self.state.lock() {
                    state.mic_status = format!("microphone unavailable: {err}");
                }
                self.mic = None;
            }
        }
    }

    fn snapshot_state(&self) -> UiState {
        let state = self.state.lock().unwrap();
        state.clone()
    }

    pub(crate) fn send_radio_msg(&self, msg: ClientRadioMessage) {
        let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(msg));
    }

    /// Space-bar CW keying (CW TX Phase 1).  Space held = CW key down, released
    /// = key up.  Only active when a radio is acquired, the source supports TX,
    /// and the current mode is CW.  Edge-detected against `cw_key_down` so a
    /// single Start/Stop is sent per press (no auto-repeat spam).  When a text
    /// edit has keyboard focus we treat Space as "not keying" so it isn't stolen
    /// from text widgets (and any in-progress key is released).
    fn handle_cw_keying(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        use rigflow_core::dsp::modes::DemodMode;

        let typing = ctx.wants_keyboard_input();
        let space_held = !typing && ctx.input(|i| i.key_down(egui::Key::Space));

        let cw_ready = snapshot.radio_acquired
            && snapshot.source_capabilities.supports_tx_tune_test
            && matches!(snapshot.demod_mode, DemodMode::Cwu | DemodMode::Cwl);
        let want_key = space_held && cw_ready;

        // Keep the lock-free sidetone control current every frame so CW Pitch
        // and Sidetone Volume changes take effect immediately.  The Arc is
        // shared with the audio callback; writing via the snapshot clone hits
        // the same inner state.
        snapshot.sidetone.set_pitch_hz(snapshot.pitch_hz);
        snapshot
            .sidetone
            .set_volume(snapshot.cw_sidetone_volume as f32 / 100.0);

        // Only act on a transition; this also releases the key if the operator
        // leaves CW mode, releases the radio, or focuses a text field mid-hold.
        if want_key != snapshot.cw_key_down {
            if want_key {
                self.send_radio_msg(ClientRadioMessage::StartCwKey);
            } else {
                self.send_radio_msg(ClientRadioMessage::StopCwKey);
            }
            // Start/stop the local sidetone immediately (no server round-trip).
            snapshot.sidetone.set_keyed(want_key);
            if let Ok(mut state) = self.state.lock() {
                state.cw_key_down = want_key;
            }
        }
    }

    /// Start the client-side Text-to-CW sender for `text` (used by the Send
    /// button, the macro buttons, and the F1–F4 shortcuts).  Spawns the timer
    /// thread; the server sees only StartCwKey/StopCwKey.
    pub(crate) fn trigger_cw_text(&self, text: String, wpm: u32, sidetone: Arc<SidetoneShared>) {
        crate::cw_text::spawn_send(
            text,
            wpm,
            self.ws_cmd_tx.clone(),
            sidetone,
            Arc::clone(&self.cw_text_abort),
            Arc::clone(&self.cw_text_sending),
        );
    }

    /// F1–F4 fire CW memory macros via Text-to-CW.  Only when a radio is
    /// acquired, the source supports TX, the mode is CWU/CWL, no text field has
    /// focus, and no message is already sending.  Empty macros do nothing.
    fn handle_cw_macros(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        use rigflow_core::dsp::modes::DemodMode;
        use std::sync::atomic::Ordering;

        if ctx.wants_keyboard_input()
            || !snapshot.radio_acquired
            || !snapshot.source_capabilities.supports_tx_tune_test
            || !matches!(snapshot.demod_mode, DemodMode::Cwu | DemodMode::Cwl)
            || self.cw_text_sending.load(Ordering::Relaxed)
        {
            return;
        }

        let idx = ctx.input(|i| {
            if i.key_pressed(egui::Key::F1) {
                Some(0)
            } else if i.key_pressed(egui::Key::F2) {
                Some(1)
            } else if i.key_pressed(egui::Key::F3) {
                Some(2)
            } else if i.key_pressed(egui::Key::F4) {
                Some(3)
            } else {
                None
            }
        });

        if let Some(i) = idx {
            let text = snapshot.cw_macros[i].text.clone();
            if text.trim().is_empty() {
                return;
            }
            let wpm = snapshot.cw_speed_wpm;
            // Mirror the macro into the message field, then send.
            if let Ok(mut state) = self.state.lock() {
                state.cw_message = text.clone();
            }
            self.trigger_cw_text(text, wpm, Arc::clone(&snapshot.sidetone));
            self.save_cw_message_to_current_operator();
        }
    }

    /// Space-bar SSB mic PTT (USB/LSB).  Space held = transmit; the server keys
    /// PTT and either modulates the mic UDP stream or, when the two-tone test is
    /// enabled, generates the tones server-side.  Gated like CW keying
    /// (acquired, TX-capable, not typing), edge-detected against `ssb_ptt_down`.
    /// Mic capture streams only when NOT running the two-tone test, so the mic
    /// queue can't overrun (and bump the diag counter) while tones are sourced.
    fn handle_ssb_ptt(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        use rigflow_core::dsp::modes::DemodMode;

        let typing = ctx.wants_keyboard_input();
        let space_held = !typing && ctx.input(|i| i.key_down(egui::Key::Space));

        let ssb_ready = snapshot.radio_acquired
            && snapshot.source_capabilities.supports_tx_tune_test
            && matches!(snapshot.demod_mode, DemodMode::Usb | DemodMode::Lsb);
        let want_tx = space_held && ssb_ready;

        if want_tx != snapshot.ssb_ptt_down {
            if want_tx {
                if !snapshot.two_tone_enabled {
                    snapshot.mic_shared.set_tx_streaming(true);
                }
                self.send_radio_msg(ClientRadioMessage::StartMicTx);
            } else {
                self.send_radio_msg(ClientRadioMessage::StopMicTx);
                snapshot.mic_shared.set_tx_streaming(false);
            }
            if let Ok(mut state) = self.state.lock() {
                state.ssb_ptt_down = want_tx;
            }
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let mut center_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift {
                1_000_000.0
            } else {
                25_000.0
            };

            if input.key_pressed(egui::Key::ArrowUp) {
                center_delta_hz += step;
            }

            if input.key_pressed(egui::Key::ArrowDown) {
                center_delta_hz -= step;
            }
        });

        if center_delta_hz != 0.0 {
            let mut send_center: Option<u64> = None;

            if let Ok(mut state) = self.state.lock() {
                let limits = crate::ui::freq_limits::active_freq_limits(&state);
                let new_center = crate::ui::freq_limits::clamp_center(
                    state.center_freq_hz + center_delta_hz,
                    &limits,
                );
                state.center_freq_hz = new_center;

                if state.radio_acquired {
                    send_center = Some(new_center as u64);
                }
            }

            if let Some(hz) = send_center {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetCenterFrequency {
                        center_freq_hz: hz as u64,
                    },
                ));
            }
        }

        let mut target_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift { 1_000.0 } else { 10.0 };

            if input.key_pressed(egui::Key::ArrowRight) {
                target_delta_hz += step;
            }

            if input.key_pressed(egui::Key::ArrowLeft) {
                target_delta_hz -= step;
            }
        });

        if target_delta_hz != 0.0 {
            let mut send_target: Option<u64> = None;

            if let Ok(mut state) = self.state.lock() {
                let limits = crate::ui::freq_limits::active_freq_limits(&state);
                let new_target = crate::ui::freq_limits::clamp_target(
                    state.target_freq_hz + target_delta_hz,
                    state.center_freq_hz,
                    state.input_sample_rate_hz,
                    &limits,
                );
                state.target_freq_hz = new_target;

                if state.radio_acquired {
                    send_target = Some(new_target as u64);
                }
            }

            if let Some(hz) = send_target {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                        target_freq_hz: hz as u64,
                    },
                ));
            }
        }
    }
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.snapshot_state();
        let config_mode = !snapshot.server_connected;

        self.ensure_mic();
        self.handle_keyboard_shortcuts(ctx);
        self.handle_cw_keying(ctx, &snapshot);
        self.handle_cw_macros(ctx, &snapshot);
        self.handle_ssb_ptt(ctx, &snapshot);
        self.draw_left_panel(ctx, &snapshot, config_mode);
        self.draw_center_panel(ctx, &snapshot);
        self.draw_add_operator_dialog(ctx);
        self.draw_add_bookmark_dialog(ctx);
        self.draw_delete_operator_dialog(ctx);
        self.draw_swr_sweep_window(ctx);

        ctx.request_repaint();
    }
}
