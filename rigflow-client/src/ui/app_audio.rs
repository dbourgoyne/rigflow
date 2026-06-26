//! Per-operator audio recording + SSB voice keyer control (client-local).
//!
//! Glue between the `&self` UI panels (which only set request flags in
//! `UiState`) and the owning [`RigflowApp`], which holds the recorder handles
//! and drives playback.  All files live under the operator's config directory
//! (`operators/<ID>/{rx_recordings,voice_keyer_clips}/`); nothing here touches
//! the server protocol.  The voice keyer's only un-key path is the
//! [`crate::voice_keyer::KeyingGuard`]; this module just arms/triggers it and
//! enforces the abort conditions each frame.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use rigflow_protocol::ClientRadioMessage;

use super::app::RigflowApp;
use super::state::UiState;

impl RigflowApp {
    /// Run any UI-requested recording / keyer action (request flags set by the
    /// `&self` panels and cleared here).
    pub(crate) fn process_audio_requests(&mut self, snapshot: &UiState) {
        let (rx_req, clip_req, preview_req, delete_req, play_req, abort_req, clip_dirty) = {
            let mut s = match self.state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            (
                s.rx_rec_request.take(),
                s.clip_rec_request.take(),
                s.clip_preview_request.take(),
                std::mem::take(&mut s.clip_delete_request),
                std::mem::take(&mut s.voice_keyer_play_request),
                std::mem::take(&mut s.voice_keyer_abort_request),
                std::mem::take(&mut s.voice_keyer_clip_dirty),
            )
        };

        if let Some(on) = rx_req {
            if on {
                self.start_rx_recording();
            } else {
                self.stop_rx_recording();
            }
        }
        if let Some(on) = clip_req {
            if on {
                self.start_clip_recording();
            } else {
                self.stop_clip_recording();
            }
        }
        if let Some(on) = preview_req {
            if on {
                self.preview_selected_clip();
            } else {
                self.stop_preview();
            }
        }
        if delete_req {
            self.delete_selected_clip();
        }
        if play_req {
            self.trigger_voice_keyer(snapshot);
        }
        if abort_req {
            snapshot.voice_keyer.request_abort();
        }
        if clip_dirty {
            self.save_voice_keyer_clip_to_current_operator();
        }
    }

    /// On an operator switch, ensure the per-operator data dirs exist and refresh
    /// the clip list; every frame, mirror live recorder status into `UiState`.
    pub(crate) fn sync_audio_recording_state(&mut self, snapshot: &UiState) {
        let op = snapshot.operator_id.clone();
        if self.clips_listed_for.as_deref() != Some(op.as_str()) {
            if !op.trim().is_empty() {
                let _ = self.persistence_store.ensure_operator_data_layout(&op);
            }
            self.refresh_voice_keyer_clips();
            self.clips_listed_for = Some(op);
        }

        let rx_status = self
            .rx_recorder
            .as_ref()
            .map(|r| r.status())
            .unwrap_or_default();
        let clip_status = self
            .clip_recorder
            .as_ref()
            .map(|r| r.status())
            .unwrap_or_default();
        if let Ok(mut s) = self.state.lock() {
            s.rx_audio_rec_status = rx_status;
            s.clip_rec_status = clip_status;
        }
    }

    // ── RX audio recording ────────────────────────────────────────────────

    fn start_rx_recording(&mut self) {
        if self.rx_recorder.is_some() {
            return;
        }
        let (op, freq, mode, slot) = {
            let s = self.state.lock().unwrap();
            (
                s.operator_id.clone(),
                s.target_freq_hz.max(0.0) as u64,
                format!("{:?}", s.demod_mode),
                Arc::clone(&s.rx_rec_slot),
            )
        };
        if op.trim().is_empty() {
            self.set_keyer_error("set an operator before recording".to_string());
            return;
        }
        if let Err(e) = self.persistence_store.ensure_operator_data_layout(&op) {
            self.set_keyer_error(format!("recording dir: {e}"));
            return;
        }
        let name = format!("rx_{}_{}Hz_{}.wav", timestamp_compact(), freq, mode);
        let path = self.persistence_store.rx_recordings_dir(&op).join(name);
        match crate::audio_recorder::AudioRecorder::start(path) {
            Ok((rec, sink)) => {
                if let Ok(mut g) = slot.lock() {
                    *g = Some(sink);
                }
                self.rx_recorder = Some(rec);
            }
            Err(e) => self.set_keyer_error(format!("record: {e}")),
        }
    }

    fn stop_rx_recording(&mut self) {
        // Remove the callback sink first (closes the channel), then finalize.
        let slot = { self.state.lock().ok().map(|s| Arc::clone(&s.rx_rec_slot)) };
        if let Some(slot) = slot {
            if let Ok(mut g) = slot.lock() {
                *g = None;
            }
        }
        if let Some(rec) = self.rx_recorder.take() {
            rec.finalize();
        }
        if let Ok(mut s) = self.state.lock() {
            s.rx_audio_rec_status = Default::default();
        }
    }

    // ── Voice-keyer clip recording (mic → WAV, no transmit) ────────────────

    fn start_clip_recording(&mut self) {
        if self.clip_recorder.is_some() {
            return;
        }
        let (op, raw_name, mic) = {
            let s = self.state.lock().unwrap();
            (
                s.operator_id.clone(),
                s.clip_name_input.clone(),
                Arc::clone(&s.mic_shared),
            )
        };
        if op.trim().is_empty() {
            self.set_keyer_error("set an operator first".to_string());
            return;
        }
        let safe = sanitize_clip_name(&raw_name);
        if safe.is_empty() {
            self.set_keyer_error("enter a clip name".to_string());
            return;
        }
        if let Err(e) = self.persistence_store.ensure_operator_data_layout(&op) {
            self.set_keyer_error(format!("clip dir: {e}"));
            return;
        }
        let path = self
            .persistence_store
            .voice_keyer_clips_dir(&op)
            .join(format!("{safe}.wav"));
        match crate::audio_recorder::AudioRecorder::start(path) {
            Ok((rec, sink)) => {
                mic.set_clip_recorder(Some(sink));
                self.clip_recorder = Some(rec);
                if let Ok(mut s) = self.state.lock() {
                    s.clip_recording = true;
                    s.voice_keyer_error = None;
                }
            }
            Err(e) => self.set_keyer_error(format!("record: {e}")),
        }
    }

    fn stop_clip_recording(&mut self) {
        if let Ok(s) = self.state.lock() {
            s.mic_shared.set_clip_recorder(None);
        }
        if let Some(rec) = self.clip_recorder.take() {
            rec.finalize();
        }
        // Compute the just-recorded filename so we can auto-select it.
        let recorded = {
            let mut s = self.state.lock().unwrap();
            s.clip_recording = false;
            s.clip_rec_status = Default::default();
            let safe = sanitize_clip_name(&s.clip_name_input);
            if safe.is_empty() {
                None
            } else {
                Some(format!("{safe}.wav"))
            }
        };
        self.refresh_voice_keyer_clips();
        if let Some(name) = recorded {
            let select = {
                let mut s = self.state.lock().unwrap();
                if s.voice_keyer_clips.contains(&name) {
                    s.voice_keyer_clip = name;
                    true
                } else {
                    false
                }
            };
            if select {
                self.save_voice_keyer_clip_to_current_operator();
            }
        }
    }

    // ── Clip preview (local monitor, no transmit) ─────────────────────────

    fn preview_selected_clip(&mut self) {
        let (op, clip, preview) = {
            let s = self.state.lock().unwrap();
            (
                s.operator_id.clone(),
                s.voice_keyer_clip.clone(),
                Arc::clone(&s.clip_preview),
            )
        };
        if clip.is_empty() {
            self.set_keyer_error("select a clip to preview".to_string());
            return;
        }
        let path = self
            .persistence_store
            .voice_keyer_clips_dir(&op)
            .join(&clip);
        match crate::voice_keyer::load_clip(&path) {
            Ok(c) => {
                preview.play(&c.samples);
                self.set_keyer_error_clear();
            }
            Err(e) => self.set_keyer_error(format!("preview: {e}")),
        }
    }

    fn stop_preview(&mut self) {
        if let Ok(s) = self.state.lock() {
            s.clip_preview.stop();
        }
    }

    // ── Delete ────────────────────────────────────────────────────────────

    fn delete_selected_clip(&mut self) {
        let (op, clip) = {
            let s = self.state.lock().unwrap();
            (s.operator_id.clone(), s.voice_keyer_clip.clone())
        };
        if clip.is_empty() {
            return;
        }
        let path = self
            .persistence_store
            .voice_keyer_clips_dir(&op)
            .join(&clip);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if let Ok(mut s) = self.state.lock() {
                    s.voice_keyer_clip.clear();
                }
                self.save_voice_keyer_clip_to_current_operator();
            }
            Err(e) => self.set_keyer_error(format!("delete: {e}")),
        }
        self.refresh_voice_keyer_clips();
    }

    // ── Voice keyer transmit ──────────────────────────────────────────────

    fn trigger_voice_keyer(&mut self, snapshot: &UiState) {
        use rigflow_core::dsp::modes::DemodMode;

        let keyer = Arc::clone(&snapshot.voice_keyer);
        if keyer.is_playing() {
            return;
        }
        // Refusal guards (mutual exclusion + TX safety).
        if snapshot.ssb_ptt_down
            || snapshot.cw_key_down
            || snapshot.two_tone_enabled
            || snapshot.clip_recording
        {
            self.set_keyer_error("busy: stop other TX / recording first".to_string());
            return;
        }
        if !snapshot.radio_acquired || !snapshot.source_capabilities.supports_tx_tune_test {
            self.set_keyer_error("no TX-capable radio acquired".to_string());
            return;
        }
        if !matches!(
            snapshot.demod_mode,
            DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU
        ) {
            self.set_keyer_error("voice keyer needs USB / LSB / Data mode".to_string());
            return;
        }

        let (op, clip, mic) = {
            let s = self.state.lock().unwrap();
            (
                s.operator_id.clone(),
                s.voice_keyer_clip.clone(),
                Arc::clone(&s.mic_shared),
            )
        };
        if clip.is_empty() {
            self.set_keyer_error("select a clip".to_string());
            return;
        }
        let path = self
            .persistence_store
            .voice_keyer_clips_dir(&op)
            .join(&clip);
        let loaded = match crate::voice_keyer::load_clip(&path) {
            Ok(c) => c,
            Err(e) => {
                self.set_keyer_error(format!("clip: {e}"));
                return;
            }
        };

        // Start sequence — must precede spawn so keying + the ring are armed
        // before audio flows (matches the TCI / Space-PTT ordering).
        mic.set_external_tx_source(true);
        mic.set_tx_streaming(true);
        self.send_radio_msg(ClientRadioMessage::StartMicTx);
        crate::voice_keyer::spawn(
            loaded,
            self.ws_cmd_tx.clone(),
            mic,
            keyer,
            crate::voice_keyer::DEFAULT_MAX_DURATION,
        );
        self.set_keyer_error_clear();
    }

    /// Per-frame voice-keyer safety enforcer.  Called before `handle_ssb_ptt` so
    /// a fresh Space press aborts the keyer rather than fighting it.  Every stop
    /// condition reduces to setting `abort`; the playback thread's guard releases.
    pub(crate) fn enforce_keyer_safety(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        use rigflow_core::dsp::modes::DemodMode;

        if !snapshot.voice_keyer.is_playing() {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let space = focused
            && !ctx.wants_keyboard_input()
            && !self.ptt_needs_fresh_press
            && ctx.input(|i| i.key_down(egui::Key::Space));
        let mode_ok = matches!(
            snapshot.demod_mode,
            DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU
        );
        if space || !snapshot.radio_acquired || !snapshot.server_connected || !mode_ok {
            snapshot.voice_keyer.request_abort();
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    /// Refresh the list of voice-keyer clip filenames for the current operator.
    fn refresh_voice_keyer_clips(&mut self) {
        let op = self
            .state
            .lock()
            .map(|s| s.operator_id.clone())
            .unwrap_or_default();
        let mut clips: Vec<String> = if op.trim().is_empty() {
            Vec::new()
        } else {
            let dir = self.persistence_store.voice_keyer_clips_dir(&op);
            std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter_map(|e| {
                            let p = e.path();
                            let is_wav = p
                                .extension()
                                .and_then(|x| x.to_str())
                                .map(|x| x.eq_ignore_ascii_case("wav"))
                                .unwrap_or(false);
                            if is_wav {
                                p.file_name().map(|n| n.to_string_lossy().into_owned())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        clips.sort();
        if let Ok(mut s) = self.state.lock() {
            s.voice_keyer_clips = clips;
        }
    }

    /// Persist the selected voice-keyer clip filename for the current operator
    /// (mirrors `save_mic_settings_to_current_operator`).
    fn save_voice_keyer_clip_to_current_operator(&mut self) {
        let (operator_id, voice_keyer_clip) = {
            let s = self.state.lock().unwrap();
            (s.operator_id.clone(), s.voice_keyer_clip.clone())
        };
        if operator_id.trim().is_empty() {
            return;
        }
        if let Ok(mut operator_settings) = self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            operator_settings.voice_keyer_clip = voice_keyer_clip;
            let _ = self
                .persistence_store
                .save_operator_settings(&operator_settings);
        }
    }

    fn set_keyer_error(&self, msg: String) {
        if let Ok(mut s) = self.state.lock() {
            s.voice_keyer_error = Some(msg);
        }
    }

    fn set_keyer_error_clear(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.voice_keyer_error = None;
        }
    }
}

/// Keep only filename-safe characters; everything else becomes `_`.
fn sanitize_clip_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

/// `YYYY-MM-DD_HH-MM-SS` (UTC) for filenames (no external date dependency).
fn timestamp_compact() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s) = (sod / 3_600, (sod % 3_600) / 60, sod % 60);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch → (Y, M, D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
