use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use tokio::sync::mpsc;

use crate::net::control::ControlCommand;
use rigflow_protocol::radio_control::ClientRadioMessage;

use crate::persistence::PersistenceStore;
use crate::sidetone::SidetoneShared;

use crate::ui::state::UiState;

/// Graceful-exit state for the window-[X] path: release the radio + disconnect,
/// hold the window open briefly so those flush over the WebSocket, then close.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitPhase {
    Running,
    ShuttingDown,
    Closing,
}

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    /// VFO B (dual-watch) spectrum + waterfall buffers, drawn in the stacked
    /// lower pane when dual-watch is active.
    pub spectrum_db_b: Arc<Mutex<Vec<f32>>>,
    pub waterfall_buffer_b: Arc<Mutex<Vec<u32>>>,
    pub persistence_store: PersistenceStore,
    pub waterfall_texture: Option<egui::TextureHandle>,
    /// Texture for VFO B's waterfall (separate from VFO A's).
    pub waterfall_texture_b: Option<egui::TextureHandle>,

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

    /// Space-PTT focus latch: set when the window loses focus (we can't observe
    /// the key-up while unfocused), cleared on a fresh Space press — so transmit
    /// never resumes just because egui still reports Space "down" after refocus.
    pub(crate) ptt_needs_fresh_press: bool,

    /// Graceful-exit state machine (window-[X] path).  See `handle_exit`.
    exit_phase: ExitPhase,
    shutdown_started_at: Option<Instant>,

    /// Per-radio settings autosave (debounced diff).  `radio_settings_last` is
    /// the bundle seen last frame; `radio_settings_stable_since` is when it last
    /// stopped changing.  See `autosave_radio_settings`.
    pub(crate) radio_settings_last: Option<crate::persistence::models::RadioSettingsFile>,
    pub(crate) radio_settings_stable_since: Option<Instant>,

    /// Held peak of the estimated TX-total latency over the current/last keying
    /// (Latency panel).  Reset on each PTT key-down edge and accumulated while
    /// keyed, so the over's worst case stays readable after unkey; `latency_tx_keyed`
    /// tracks the rising edge.
    pub(crate) latency_tx_peak_ms: f32,
    pub(crate) latency_tx_keyed: bool,
    /// Held "TX total" latency: updated live while keyed, frozen at the last
    /// over's final value after unkey (reset on the next key-down edge), so the
    /// readout doesn't drift while receiving.
    pub(crate) latency_tx_total_ms: f32,

    /// Owning handle for the in-progress RX-audio recording (`None` when idle).
    pub(crate) rx_recorder: Option<crate::audio_recorder::AudioRecorder>,
    /// Owning handle for the in-progress voice-keyer clip recording.
    pub(crate) clip_recorder: Option<crate::audio_recorder::AudioRecorder>,
    /// Operator whose voice-keyer clip list is currently cached, so the list is
    /// refreshed (and the per-operator data dirs ensured) on an operator switch.
    pub(crate) clips_listed_for: Option<String>,
}

impl RigflowApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Arc<Mutex<UiState>>,
        ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
        waterfall_buffer: Arc<Mutex<Vec<u32>>>,
        spectrum_db: Arc<Mutex<Vec<f32>>>,
        waterfall_buffer_b: Arc<Mutex<Vec<u32>>>,
        spectrum_db_b: Arc<Mutex<Vec<f32>>>,
        persistence_store: PersistenceStore,
    ) -> Self {
        // Create the virtual digital-audio endpoints once, at startup.
        let digital_audio = crate::digital_audio::DigitalAudio::start();
        let digital_output_available = digital_audio.output_available();
        let digital_rx_available = digital_audio.rx_available();
        let digital_input_available = digital_audio.input_available();
        let digital_output_reason = digital_audio.output_reason();
        let digital_rx_reason = digital_audio.rx_reason();
        let digital_input_reason = digital_audio.input_reason();

        let app = Self {
            state,
            ws_cmd_tx,
            waterfall_buffer,
            spectrum_db,
            spectrum_db_b,
            waterfall_buffer_b,
            persistence_store,
            waterfall_texture: None,
            waterfall_texture_b: None,
            cw_text_abort: Arc::new(AtomicBool::new(false)),
            cw_text_sending: Arc::new(AtomicBool::new(false)),
            mic: None,
            mic_requested: None,
            digital_audio,
            ptt_needs_fresh_press: false,
            exit_phase: ExitPhase::Running,
            shutdown_started_at: None,
            radio_settings_last: None,
            radio_settings_stable_since: None,
            latency_tx_peak_ms: 0.0,
            latency_tx_keyed: false,
            latency_tx_total_ms: 0.0,
            rx_recorder: None,
            clip_recorder: None,
            clips_listed_for: None,
        };

        // Enumerate input devices once for the dropdown (one-time; cheap enough
        // and avoids re-enumerating every frame).
        let devices = crate::mic::list_input_devices();
        if let Ok(mut state) = app.state.lock() {
            state.mic_devices = devices;
            state.digital_output_available = digital_output_available;
            state.digital_rx_available = digital_rx_available;
            state.digital_input_available = digital_input_available;
            state.digital_output_reason = digital_output_reason;
            state.digital_rx_reason = digital_rx_reason;
            state.digital_input_reason = digital_input_reason;
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

        // Tear down the existing capture BEFORE opening the new device.  Both
        // streams push into the same shared TX ring, so if the old one is still
        // live when the new one starts (it was only dropped by the assignment
        // below, after `start_capture` returned), two producers feed the ring at
        // once → the server sees ~2× 48 kHz and reports mic-TX overruns.  On
        // macOS a switched-away stream lingered this way; dropping first
        // guarantees a single producer.
        self.mic = None;

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

    /// Maintain the Space-PTT focus latch once per frame (call before the PTT
    /// handlers).  egui never clears `keys_down` on focus loss and emits no
    /// synthetic key events on refocus, so after a blur it still reports Space
    /// "down" — which would resume transmit on refocus.  We latch on blur and
    /// only clear on a real, fresh Space press, so a deliberate press is required
    /// to key again.  Shared by the SSB and CW handlers (Space is one key).
    fn update_ptt_focus_latch(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if !focused {
            self.ptt_needs_fresh_press = true;
        } else if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.ptt_needs_fresh_press = false;
        }
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
        // A hold-to-talk key can't be observed while the window is unfocused, so
        // treat "not focused" as key-up — fail safe to RX instead of latching the
        // transmitter when the user holds Space and switches windows.  After a
        // blur, `ptt_needs_fresh_press` also requires a deliberate new press so
        // TX doesn't resume from egui's stale key state on refocus.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let needs_fresh = self.ptt_needs_fresh_press;
        let space_held =
            focused && !typing && !needs_fresh && ctx.input(|i| i.key_down(egui::Key::Space));

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
        // Fail safe to RX if the window loses focus mid-hold, and require a fresh
        // press after a blur so TX doesn't resume from a stale key on refocus
        // (see handle_cw_keying / update_ptt_focus_latch).
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let needs_fresh = self.ptt_needs_fresh_press;
        let space_held =
            focused && !typing && !needs_fresh && ctx.input(|i| i.key_down(egui::Key::Space));

        let ssb_ready = snapshot.radio_acquired
            && snapshot.source_capabilities.supports_tx_tune_test
            && matches!(
                snapshot.demod_mode,
                DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU
            )
            // The voice keyer owns the mic-TX path while playing, and a clip
            // recording must never key the radio: suppress manual PTT for both.
            && !snapshot.voice_keyer.is_playing()
            && !snapshot.clip_recording;
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
        use crate::ui::tuning_steps::{TuneTier, center_step_hz, target_step_hz};

        // Gather arrow presses + modifiers in one input pass.
        let (up, down, left, right, shift, alt) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.shift,
                i.modifiers.alt,
            )
        });

        if !(up || down || left || right) {
            return;
        }

        // Steps are mode-aware and only apply to an acquired radio (matches the
        // mouse wheel).
        let snapshot = self.snapshot_state();
        if !snapshot.radio_acquired {
            return;
        }
        let mode = snapshot.demod_mode;

        // ↑/↓ — center / LO step (mode-aware; Shift = coarse).
        let center_dir = (up as i32) - (down as i32);
        if center_dir != 0 {
            let delta = center_dir as f32 * center_step_hz(mode, shift);
            let mut send_center: Option<u64> = None;
            if let Ok(mut state) = self.state.lock() {
                let limits = crate::ui::freq_limits::active_freq_limits(&state);
                let new_center =
                    crate::ui::freq_limits::clamp_center(state.center_freq_hz + delta, &limits);
                state.center_freq_hz = new_center;
                send_center = Some(new_center as u64);
            }
            if let Some(hz) = send_center {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetCenterFrequency { center_freq_hz: hz },
                ));
            }
        }

        // ←/→ — target step, identical to the wheel (mode-aware; Shift = medium,
        // Alt = coarse) including soft-edge LO panning.
        let target_dir = (right as i32) - (left as i32);
        if target_dir != 0 {
            let tier = if shift {
                TuneTier::Medium
            } else if alt {
                TuneTier::Coarse
            } else {
                TuneTier::Fine
            };
            let delta = target_dir as f32 * target_step_hz(mode, tier);
            self.tune_target_relative(&snapshot, delta);
        }
    }

    /// Window-close ([X]) graceful exit: release the radio (which un-keys it
    /// server-side) and disconnect, holding the window open just long enough for
    /// those to flush over the WebSocket, then close.  A returning user who is
    /// already disconnected exits immediately.  Kill signals (SIGINT/SIGTERM) are
    /// handled separately in `main` and do the same release-then-disconnect.
    fn handle_exit(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        let close_requested = ctx.input(|i| i.viewport().close_requested());

        match self.exit_phase {
            ExitPhase::Running => {
                if close_requested && snapshot.server_connected {
                    // Abort any voice-keyer transmission first; its guard sends
                    // StopMicTx, which flushes within the shutdown hold below.
                    snapshot.voice_keyer.request_abort();
                    if snapshot.radio_acquired {
                        let _ = self.ws_cmd_tx.send(ControlCommand::ReleaseRadio);
                    }
                    let _ = self.ws_cmd_tx.send(ControlCommand::Disconnect);
                    self.exit_phase = ExitPhase::ShuttingDown;
                    self.shutdown_started_at = Some(Instant::now());
                    // Hold the window open until the cleanup flushes (or times out).
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                // Not connected: nothing to clean up — let the close proceed.
            }
            ExitPhase::ShuttingDown => {
                if close_requested {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                let timed_out = self
                    .shutdown_started_at
                    .map(|t| t.elapsed() >= Duration::from_millis(1500))
                    .unwrap_or(true);
                if !snapshot.server_connected || timed_out {
                    self.exit_phase = ExitPhase::Closing;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // `request_repaint` at the end of `update` keeps us ticking until done.
            }
            ExitPhase::Closing => {
                // Final close is in flight — let it through.
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
        self.update_ptt_focus_latch(ctx);
        self.handle_cw_keying(ctx, &snapshot);
        self.handle_cw_macros(ctx, &snapshot);
        self.enforce_keyer_safety(ctx, &snapshot);
        self.handle_ssb_ptt(ctx, &snapshot);
        self.draw_left_panel(ctx, &snapshot, config_mode);
        self.draw_center_panel(ctx, &snapshot);
        self.draw_add_operator_dialog(ctx);
        self.draw_add_bookmark_dialog(ctx);
        self.draw_delete_operator_dialog(ctx);
        self.draw_swr_sweep_window(ctx);
        self.draw_wsjtx_setup_window(ctx);

        // Per-operator audio recording + voice keyer: ensure dirs / refresh the
        // clip list on an operator switch, run any UI-requested action, and
        // mirror live recorder status into UiState for the panels.
        self.sync_audio_recording_state(&snapshot);
        self.process_audio_requests(&snapshot);

        // Persist per-radio settings: diff the live per-radio state against the
        // saved bucket and save ~600 ms after it stops changing.  Debounced so a
        // slider/frequency drag doesn't thrash the file, and catches every change
        // path (tuning, band, all controls).
        self.autosave_radio_settings();

        self.handle_exit(ctx, &snapshot);

        ctx.request_repaint();
    }
}
