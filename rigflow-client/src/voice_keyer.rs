//! SSB voice keyer: load a recorded clip and transmit it on the current
//! frequency by feeding the proven mic-TX path (`MicShared::push_tx` → the
//! dedicated mic-send thread → server SSB session).
//!
//! Safety is the whole point.  SSB mic TX has **no server-side watchdog**, so a
//! stuck keyer would key the radio indefinitely.  The single un-key path is
//! [`KeyingGuard`]'s `Drop`, which runs on every thread-exit (natural end,
//! abort, early return, panic).  Every external trigger (UI abort, Space-bar
//! override, disconnect, mode change) just sets `KeyerShared::abort`; the
//! playback thread observes it within ≤5 ms and the guard releases.  A hard
//! [`DEFAULT_MAX_DURATION`] cap in the loop is an independent backstop.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::mic::MicShared;
use crate::net::control::ControlCommand;
use rigflow_protocol::ClientRadioMessage;

/// Hard cap on a single keyed transmission, independent of clip length and abort
/// signaling.  Bounds an unintended key-down even if every other stop fails.
pub const DEFAULT_MAX_DURATION: Duration = Duration::from_secs(30);

/// Output (and clip) sample rate.
const RATE_HZ: f64 = 48_000.0;
/// Real-time push chunk: 20 ms at 48 kHz.  Small enough to stay far below the
/// ~0.5 s mic-TX ring, large enough to ride out scheduling jitter.
const CHUNK: usize = 960;

/// A clip loaded and normalized to 48 kHz mono f32, ready to transmit.
pub struct VoiceClip {
    pub samples: Vec<f32>,
}

/// Load a WAV clip and normalize to 48 kHz mono f32.  Reuses the TCI TX-audio
/// downmix + resample so there is a single implementation.
pub fn load_clip(path: &Path) -> Result<VoiceClip, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / scale)
                .collect()
        }
    };

    let mono = crate::tci_server::downmix(&interleaved, spec.channels as u32);
    let mono48 = crate::tci_server::resample(&mono, spec.sample_rate as f32, RATE_HZ as f32);

    // Protect the modulator/DAC from any NaN/Inf that slipped through.
    let samples: Vec<f32> = mono48
        .into_iter()
        .map(|s| if s.is_finite() { s } else { 0.0 })
        .collect();

    if samples.is_empty() {
        return Err("clip contains no audio".to_string());
    }

    Ok(VoiceClip { samples })
}

/// Lock-free state shared between the playback thread and the UI.
#[derive(Debug, Default)]
pub struct KeyerShared {
    /// Request a prompt stop; set by any abort path, observed by the thread.
    abort: AtomicBool,
    /// True from the start sequence until the playback thread fully releases.
    playing: AtomicBool,
    elapsed_samples: AtomicU64,
    total_samples: AtomicU64,
}

impl KeyerShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Request abort (UI button, Space override, disconnect, mode change…).
    pub fn request_abort(&self) {
        self.abort.store(true, Ordering::Relaxed);
    }

    /// Playback progress in `0.0..=1.0` (0 when idle / unknown).
    pub fn progress(&self) -> f32 {
        let total = self.total_samples.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.elapsed_samples.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

/// RAII keying guard — the single place TX is released.  Its `Drop` runs on
/// every playback-thread exit path, so the radio always un-keys.
struct KeyingGuard {
    ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    mic: Arc<MicShared>,
    shared: Arc<KeyerShared>,
}

impl Drop for KeyingGuard {
    fn drop(&mut self) {
        // Stop sequence — reverse of the start sequence the caller performed.
        let _ = self
            .ws_cmd_tx
            .send(ControlCommand::RadioMessage(ClientRadioMessage::StopMicTx));
        self.mic.set_tx_streaming(false);
        self.mic.set_external_tx_source(false);
        self.shared.playing.store(false, Ordering::Relaxed);
    }
}

/// Spawn the playback thread.  The caller MUST have already run the start
/// sequence — `set_external_tx_source(true)` → `set_tx_streaming(true)` →
/// send `StartMicTx` — so that keying and the ring are armed before audio flows.
/// The thread feeds the clip at real time and the guard releases on exit.
pub fn spawn(
    clip: VoiceClip,
    ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    mic: Arc<MicShared>,
    shared: Arc<KeyerShared>,
    max_duration: Duration,
) {
    shared.abort.store(false, Ordering::Relaxed);
    shared
        .total_samples
        .store(clip.samples.len() as u64, Ordering::Relaxed);
    shared.elapsed_samples.store(0, Ordering::Relaxed);
    shared.playing.store(true, Ordering::Relaxed);

    thread::spawn(move || {
        // Constructed first: its Drop is the guaranteed un-key on every exit.
        let _guard = KeyingGuard {
            ws_cmd_tx,
            mic: Arc::clone(&mic),
            shared: Arc::clone(&shared),
        };

        let samples = &clip.samples;
        let total = samples.len();
        let started = Instant::now();
        let mut idx = 0usize;

        while idx < total {
            if shared.abort.load(Ordering::Relaxed) {
                break;
            }
            if started.elapsed() >= max_duration {
                break; // hard cap — independent backstop
            }

            let end = (idx + CHUNK).min(total);
            mic.push_tx(&samples[idx..end]);
            idx = end;
            shared.elapsed_samples.store(idx as u64, Ordering::Relaxed);

            // Pace to wall clock by absolute target time (no cumulative drift).
            let target = started + Duration::from_secs_f64(idx as f64 / RATE_HZ);
            let now = Instant::now();
            if target > now {
                interruptible_sleep(target - now, &shared.abort);
            }
        }
        // `_guard` drops here → StopMicTx + tx_streaming(false) + external(false).
    });
}

/// Sleep `dur`, waking every few ms to check `abort`.  Returns `true` if aborted.
fn interruptible_sleep(dur: Duration, abort: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(5);
    let mut remaining = dur;
    while remaining > Duration::ZERO {
        if abort.load(Ordering::Relaxed) {
            return true;
        }
        let s = remaining.min(SLICE);
        thread::sleep(s);
        remaining = remaining.saturating_sub(s);
    }
    abort.load(Ordering::Relaxed)
}
