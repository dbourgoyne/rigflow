//! Digital Audio Interface — TX audio routing.
//!
//! The mirror image of [`crate::digital_rx`].  While the transmitter is keyed
//! over CAT (WSJT-X PTT), this captures the audio the digital app *plays into*
//! the `RigflowDigitalInput` virtual sink — read from its `.monitor` source —
//! and feeds it into Rigflow's existing mic-TX path so it is SSB-modulated and
//! transmitted.  This is **TX only**; it does no CAT, mode, or frequency logic.
//!
//! ## Path reuse
//!
//! Captured mono/48 kHz/float32 samples are pushed straight into
//! [`MicShared`]'s TX ring (`push_tx`) — the *same* ring the microphone
//! callback fills.  The media thread already drains that ring and streams it to
//! the server as `STREAM_TYPE_MIC_AUDIO`, where `tx_ssb_mic` (keyed by the
//! `StartMicTx` the CAT server sends alongside PTT) modulates it onto RF.  So
//! the only new piece here is getting the digital app's audio *out* of the
//! PipeWire/Pulse graph and into that ring.
//!
//! ```text
//! WSJT-X → RigflowDigitalInput (sink) → .monitor → parec/pw-record
//!        → MicShared.tx_ring → media thread → UDP MIC_AUDIO → server tx_ssb_mic → RF
//! ```
//!
//! ## Crossing the audio boundary
//!
//! CPAL (ALSA host) can't capture a PulseAudio/PipeWire monitor source by name,
//! so — consistent with `digital_rx`'s `pacat`/`pw-cat` playback — we read raw
//! PCM from `parec` (Pulse) / `pw-record` (PipeWire).  A dedicated capture
//! thread owns the child process; it only runs while PTT is keyed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
// PipeWire/Pulse capture (parec/pw-record) is Linux-only; gate the machinery so
// macOS/Windows never shell out to it (digital TX there flows over TCI instead).
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;

use crate::digital_audio::DIGITAL_INPUT_NAME;
use crate::mic::MicShared;

/// Routes the digital app's TX audio into the mic-TX path while PTT is keyed.
/// Shared (`Arc`) between the CAT server (set_active) and the capture thread.
#[derive(Debug)]
pub struct DigitalTxInput {
    /// True while PTT is keyed over CAT (capture should run).
    active: AtomicBool,
    /// True while the recorder process is running and producing audio.
    available: AtomicBool,
    /// Destination for captured audio — the shared mic-TX ring/state.
    mic_shared: Arc<MicShared>,
}

impl DigitalTxInput {
    /// Create the router and spawn its capture thread (idle until activated).
    pub fn new(mic_shared: Arc<MicShared>) -> Arc<Self> {
        let me = Arc::new(Self {
            active: AtomicBool::new(false),
            available: AtomicBool::new(false),
            mic_shared,
        });
        // The monitor-capture thread (and its parec/pw-record child) is Linux-only.
        #[cfg(target_os = "linux")]
        {
            let worker = Arc::clone(&me);
            let _ = thread::Builder::new()
                .name("digital-tx".into())
                .spawn(move || capture_loop(worker));
        }
        me
    }

    /// Start/stop routing the digital app's TX audio.  Called by the CAT server
    /// when PTT is keyed/unkeyed.  Enables mic-TX streaming so the media thread
    /// forwards the captured audio, then lets the capture thread spin up/down.
    pub fn set_active(&self, on: bool) {
        if self.active.swap(on, Ordering::Relaxed) != on {
            if on {
                // Claim the TX ring as the sole producer, then enable streaming so
                // the media thread forwards it.  Without claiming it, the always-on
                // mic capture would also push (≈2× the consume rate → overrun).
                self.mic_shared.set_external_tx_source(true);
                self.mic_shared.set_tx_streaming(true);
                log::info!("[digital-tx] routing {DIGITAL_INPUT_NAME}.monitor → TX");
            } else {
                self.mic_shared.set_tx_streaming(false);
                self.mic_shared.set_external_tx_source(false);
                log::info!("[digital-tx] TX audio routing stopped");
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// True when keyed AND the monitor recorder is actually running.
    #[allow(dead_code)]
    pub fn is_capturing(&self) -> bool {
        self.active.load(Ordering::Relaxed) && self.available.load(Ordering::Relaxed)
    }
}

/// Capture thread: owns the `parec`/`pw-record` child and pumps its stdout into
/// the mic-TX ring.  The blocking read paces us to real time (monitor sources
/// produce ~48 kHz continuously), so no extra sleeping is needed.
///
/// The recorder is kept **warm across overs**: `parec`'s PulseAudio connection
/// takes ~1–2 s to start streaming, so restarting it on every key-down put
/// silence at the front of each transmission (startup underruns).  Once we've
/// been keyed at least once we leave the recorder running and just *discard* its
/// samples while unkeyed — so the next key-down forwards live audio instantly.
#[cfg(target_os = "linux")]
fn capture_loop(shared: Arc<DigitalTxInput>) {
    let mut child: Option<Child> = None;
    // Holds a partial float across reads so 4-byte samples stay aligned.
    let mut leftover: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    // Set once the recorder has ever been started; from then on we keep it warm.
    let mut warmed = false;

    loop {
        let active = shared.active.load(Ordering::Relaxed);

        // Idle without spawning until the first key-down, so a never-used digital
        // interface doesn't hold an open recorder (or spam if the source is gone).
        if !active && !warmed {
            shared.available.store(false, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Ensure a recorder is running (kept alive between overs once warmed).
        if child.is_none() {
            match spawn_recorder() {
                Some(c) => {
                    child = Some(c);
                    warmed = true;
                    shared.available.store(true, Ordering::Relaxed);
                    log::info!("[digital-tx] capturing {DIGITAL_INPUT_NAME}.monitor");
                }
                None => {
                    shared.available.store(false, Ordering::Relaxed);
                    log::warn!(
                        "[digital-tx] TX capture unavailable (no parec/pw-record or source missing)"
                    );
                    thread::sleep(Duration::from_millis(1000));
                    continue;
                }
            }
        }

        // Detect an early-exited recorder (e.g. bad source name).
        if let Some(c) = child.as_mut() {
            if matches!(c.try_wait(), Ok(Some(_))) {
                child = None;
                shared.available.store(false, Ordering::Relaxed);
                log::warn!("[digital-tx] recorder exited; retrying");
                thread::sleep(Duration::from_millis(500));
                continue;
            }
        }

        // Read a chunk from stdout (blocks ~ real time).  We read whether or not
        // we're keyed, to keep the recorder's pipe drained and its stream live.
        let read = child
            .as_mut()
            .and_then(|c| c.stdout.as_mut())
            .map(|out| out.read(&mut buf));
        let n = match read {
            Some(Ok(0)) => {
                // EOF — recorder closed its pipe.
                stop_child(&mut child);
                shared.available.store(false, Ordering::Relaxed);
                continue;
            }
            Some(Ok(n)) => n,
            Some(Err(_)) => {
                stop_child(&mut child);
                shared.available.store(false, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            None => continue,
        };

        // Accumulate bytes; forward whole f32 samples only while keyed, otherwise
        // discard them (recorder stays warm but the ring isn't fed).
        leftover.extend_from_slice(&buf[..n]);
        let full = leftover.len() / 4;
        if full > 0 {
            if active {
                let mut samples = Vec::with_capacity(full);
                for chunk in leftover[..full * 4].chunks_exact(4) {
                    samples.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
                shared.mic_shared.push_tx(&samples);
            }
            leftover.drain(..full * 4);
        }
    }
}

/// Spawn a raw mono/48k/float32 recorder reading `RigflowDigitalInput.monitor`
/// and writing PCM to stdout.  Tries `parec` (Pulse/PipeWire-pulse) then
/// `pw-record` (PipeWire-native).
#[cfg(target_os = "linux")]
fn spawn_recorder() -> Option<Child> {
    let monitor = format!("{DIGITAL_INPUT_NAME}.monitor");

    let parec = Command::new("parec")
        .args([
            "--raw",
            &format!("--device={monitor}"),
            "--rate=48000",
            "--channels=1",
            "--format=float32le",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(c) = parec {
        return Some(c);
    }

    // PipeWire-native fallback; `-` writes raw PCM to stdout.
    Command::new("pw-record")
        .args([
            "--raw",
            "--target",
            &monitor,
            "--rate",
            "48000",
            "--channels",
            "1",
            "--format",
            "f32",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

#[cfg(target_os = "linux")]
fn stop_child(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
