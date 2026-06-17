//! Digital Audio Interface — Phase 2: RX audio routing.
//!
//! Optionally mirrors Rigflow's received audio into the `RigflowDigitalOutput`
//! virtual sink (created in Phase 1), so external programs (WSJT-X, …) can
//! record it from `RigflowDigitalOutput.monitor`.  This is **RX only** — no TX,
//! CAT, PTT, or mode logic.
//!
//! ## Tap point & volume
//!
//! The tap is the **decoded receive audio in `net::udp::handle_audio_packet`**
//! — the same mono/48 kHz samples that feed the speaker jitter buffer.  Rigflow
//! applies receive **Volume server-side** (before the UDP audio stream), so the
//! client only ever has post-volume audio; therefore the digital output
//! currently **tracks Rigflow Volume**.  A volume-independent tap would require
//! a separate pre-volume stream from the server and is deferred.  The speaker
//! path is untouched — we only read a copy of the samples.
//!
//! ## Playback into the named sink
//!
//! CPAL (ALSA host) can't target a PulseAudio/PipeWire sink by name, so we feed
//! the sink through `pacat` (Pulse) / `pw-cat` (PipeWire) — consistent with the
//! `pactl` approach used to create the devices.  A dedicated writer thread owns
//! the child process and drains a bounded ring buffer, so the real-time audio
//! path never blocks: the media thread does a non-blocking push (drop-oldest on
//! overrun) and the writer thread's blocking pipe write paces to the sink.

use std::collections::VecDeque;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::digital_audio::DIGITAL_OUTPUT_NAME;

/// ~0.5 s at 48 kHz — rides out bursts without unbounded growth.
const RING_MAX: usize = 24_000;

/// Routes received audio to the `RigflowDigitalOutput` sink when enabled.
/// Shared (`Arc`) between the media thread (push) and the UI (toggle/status).
#[derive(Debug)]
pub struct DigitalRxOutput {
    /// Operator toggle (UI checkbox).
    enabled: AtomicBool,
    /// True while the player process is running and accepting audio.
    available: AtomicBool,
    /// Outbound mono-f32 audio queue drained by the writer thread.
    ring: Mutex<VecDeque<f32>>,
}

impl DigitalRxOutput {
    /// Create the router and spawn its writer thread (idle until enabled).
    pub fn new() -> Arc<Self> {
        let me = Arc::new(Self {
            enabled: AtomicBool::new(false),
            available: AtomicBool::new(false),
            ring: Mutex::new(VecDeque::new()),
        });
        let worker = Arc::clone(&me);
        // Detached writer thread; ends when the process exits (which also closes
        // the player's stdin → the player exits cleanly).
        let _ = thread::Builder::new()
            .name("digital-rx".into())
            .spawn(move || writer_loop(worker));
        me
    }

    /// Enable/disable routing (UI).  Logged on transition.
    // Only called from the Linux-gated DATA-mode RX auto-route; on macOS the
    // digital RX path is the TCI tap, so this is unused there.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn set_enabled(&self, on: bool) {
        if self.enabled.swap(on, Ordering::Relaxed) != on {
            if on {
                log::info!("[digital-rx] RX digital output enabled");
            } else {
                log::info!("[digital-rx] RX digital output disabled");
                // Drop any queued audio so we don't replay stale samples later.
                if let Ok(mut r) = self.ring.lock() {
                    r.clear();
                }
            }
        }
    }

    /// True when enabled AND the sink player is actually running.
    pub fn is_active(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) && self.available.load(Ordering::Relaxed)
    }

    /// Push a copy of received mono-f32 audio (called from the media thread).
    /// No-op unless enabled.  Non-blocking: drops oldest on overrun so the
    /// audio pipeline is never stalled.
    pub fn push(&self, samples: &[f32]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.ring.lock() {
            r.extend(samples.iter().copied());
            if r.len() > RING_MAX {
                let drop = r.len() - RING_MAX;
                r.drain(..drop);
            }
        }
    }
}

/// Writer thread: owns the `pacat`/`pw-cat` child and pumps the ring into it.
fn writer_loop(shared: Arc<DigitalRxOutput>) {
    let mut child: Option<Child> = None;

    loop {
        // Disabled → stop the player and idle.
        if !shared.enabled.load(Ordering::Relaxed) {
            stop_child(&mut child);
            shared.available.store(false, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Enabled → ensure a player is running.
        if child.is_none() {
            match spawn_player() {
                Some(c) => {
                    child = Some(c);
                    shared.available.store(true, Ordering::Relaxed);
                    log::info!("[digital-rx] routing RX audio to {DIGITAL_OUTPUT_NAME}");
                }
                None => {
                    shared.available.store(false, Ordering::Relaxed);
                    log::warn!(
                        "[digital-rx] RX digital output unavailable (no pacat/pw-cat or sink missing)"
                    );
                    thread::sleep(Duration::from_millis(1000));
                    continue;
                }
            }
        }

        // Detect an early-exited player (e.g. bad sink name).
        if let Some(c) = child.as_mut() {
            if matches!(c.try_wait(), Ok(Some(_))) {
                child = None;
                shared.available.store(false, Ordering::Relaxed);
                log::warn!("[digital-rx] RX digital output player exited; retrying");
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        }

        // Drain a chunk of audio and write it (blocking write paces to the sink).
        let chunk: Vec<f32> = {
            let mut r = shared.ring.lock().unwrap();
            let n = r.len().min(4800); // up to ~100 ms
            r.drain(..n).collect()
        };
        if chunk.is_empty() {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        let mut bytes = Vec::with_capacity(chunk.len() * 4);
        for s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let write_ok = child
            .as_mut()
            .and_then(|c| c.stdin.as_mut())
            .map(|stdin| stdin.write_all(&bytes).is_ok())
            .unwrap_or(false);
        if !write_ok {
            stop_child(&mut child);
            shared.available.store(false, Ordering::Relaxed);
            log::warn!("[digital-rx] RX digital output write failed; retrying");
            thread::sleep(Duration::from_millis(500));
        }
    }
}

/// Spawn a raw mono/48k/float32 player feeding `RigflowDigitalOutput`, reading
/// PCM from stdin.  Tries `pacat` (Pulse/PipeWire-pulse) then `pw-cat`.
fn spawn_player() -> Option<Child> {
    // pacat reads raw PCM from stdin by default for --playback.
    let pacat = Command::new("pacat")
        .args([
            "--playback",
            "--raw",
            &format!("--device={DIGITAL_OUTPUT_NAME}"),
            "--rate=48000",
            "--channels=1",
            "--format=float32le",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(c) = pacat {
        return Some(c);
    }

    // PipeWire-native fallback; `-` reads stdin.
    Command::new("pw-cat")
        .args([
            "--playback",
            "--raw",
            "--target",
            DIGITAL_OUTPUT_NAME,
            "--rate",
            "48000",
            "--channels",
            "1",
            "--format",
            "f32",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn stop_child(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
