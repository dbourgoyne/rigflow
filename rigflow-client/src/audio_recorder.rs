//! Client-side audio recording + local clip preview (all per-operator, on disk).
//!
//! Two small building blocks shared by the RX-audio recorder and the SSB
//! voice-keyer clip recorder:
//!
//! - [`AudioRecorder`] / [`AudioRecorderSink`] — a non-blocking mono / 48 kHz /
//!   i16 WAV writer.  `push` is safe to call from a real-time audio callback: it
//!   converts later (on the writer thread) and drops-and-counts when the writer
//!   falls behind, so it never blocks or fails the callback.
//! - [`ClipPreview`] — a small playback buffer the output callback mixes in, so a
//!   recorded clip can be auditioned through the speakers without transmitting.
//!
//! Neither touches the network or the TX path.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Instant;

/// Recorded WAV parameters (matches the decoded RX audio and the mic-TX rate).
const SAMPLE_RATE: u32 = 48_000;
/// Bounded writer queue: number of callback chunks buffered before drop-count.
const QUEUE_CAPACITY: usize = 256;

/// Snapshot of a recording's state, for the UI.  Inert when not recording.
#[derive(Debug, Clone, Default)]
pub struct AudioRecordingStatus {
    pub recording: bool,
    pub filename: Option<String>,
    pub elapsed_secs: u64,
    pub file_size_bytes: u64,
    pub dropped_chunks: u64,
}

/// Counters shared between the writer thread (bytes) and the callback (drops).
#[derive(Debug, Default)]
struct RecorderShared {
    dropped: AtomicU64,
    written_samples: AtomicU64,
}

/// The callback-side handle: cheap to clone, safe to call from a real-time audio
/// callback.  Hands captured samples to the writer thread without blocking.
#[derive(Debug, Clone)]
pub struct AudioRecorderSink {
    tx: SyncSender<Vec<f32>>,
    shared: Arc<RecorderShared>,
}

impl AudioRecorderSink {
    /// Hand one block of samples to the writer thread.  Non-blocking: a full
    /// queue (or a dead writer) drops the block and bumps the drop counter.
    pub fn push(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        if self.tx.try_send(samples.to_vec()).is_err() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The owning handle: holds the writer thread.  Create with [`AudioRecorder::start`],
/// install the returned [`AudioRecorderSink`] where the audio callback can reach
/// it, and call [`AudioRecorder::finalize`] after removing the sink to flush and
/// close the WAV.
pub struct AudioRecorder {
    writer: Option<JoinHandle<()>>,
    shared: Arc<RecorderShared>,
    filename: String,
    started: Instant,
}

impl AudioRecorder {
    /// Open `path` for writing and spawn the writer thread.  Returns the owner
    /// and the callback sink.  The WAV header is finalized in [`finalize`].
    pub fn start(path: PathBuf) -> Result<(AudioRecorder, AudioRecorderSink), String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;

        let (tx, rx) = sync_channel::<Vec<f32>>(QUEUE_CAPACITY);
        let shared = Arc::new(RecorderShared::default());
        let w_shared = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            // Blocks until a chunk arrives or every sender has dropped (stop).
            for chunk in rx.iter() {
                let n = chunk.len();
                for s in chunk {
                    let v = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    if writer.write_sample(v).is_err() {
                        break;
                    }
                }
                w_shared
                    .written_samples
                    .fetch_add(n as u64, Ordering::Relaxed);
            }
            let _ = writer.finalize();
        });

        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let sink = AudioRecorderSink {
            tx,
            shared: Arc::clone(&shared),
        };
        Ok((
            AudioRecorder {
                writer: Some(handle),
                shared,
                filename,
                started: Instant::now(),
            },
            sink,
        ))
    }

    /// Live status for the UI.
    pub fn status(&self) -> AudioRecordingStatus {
        AudioRecordingStatus {
            recording: self.writer.is_some(),
            filename: Some(self.filename.clone()),
            elapsed_secs: self.started.elapsed().as_secs(),
            // i16 mono → 2 bytes per sample.
            file_size_bytes: self.shared.written_samples.load(Ordering::Relaxed) * 2,
            dropped_chunks: self.shared.dropped.load(Ordering::Relaxed),
        }
    }

    /// Join the writer thread, flushing and finalizing the WAV.  The caller MUST
    /// drop every [`AudioRecorderSink`] first (e.g. clear the callback slot), or
    /// the channel never closes and this blocks.
    pub fn finalize(mut self) {
        if let Some(h) = self.writer.take() {
            let _ = h.join();
        }
    }
}

/// A local clip-preview buffer mixed into the speaker output by the audio
/// callback.  Used to audition a recorded voice-keyer clip without transmitting.
#[derive(Debug, Default)]
pub struct ClipPreview {
    buf: Mutex<VecDeque<f32>>,
    active: AtomicBool,
}

impl ClipPreview {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Queue a whole clip (48 kHz mono) for local monitoring, replacing any clip
    /// already playing.
    pub fn play(&self, samples: &[f32]) {
        if let Ok(mut b) = self.buf.lock() {
            b.clear();
            b.extend(samples.iter().copied());
        }
        self.active.store(true, Ordering::Relaxed);
    }

    /// Stop preview immediately and discard any remaining audio.
    pub fn stop(&self) {
        if let Ok(mut b) = self.buf.lock() {
            b.clear();
        }
        self.active.store(false, Ordering::Relaxed);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Mix queued (mono) preview audio into an interleaved `channels`-channel
    /// buffer, centered (one preview sample per frame added to every channel).
    /// Deactivates when the buffer empties.  No-op when inactive.
    pub fn mix_into_channels(&self, data: &mut [f32], channels: usize) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        let channels = channels.max(1);
        if let Ok(mut b) = self.buf.lock() {
            for frame in data.chunks_mut(channels) {
                match b.pop_front() {
                    Some(v) => {
                        for s in frame.iter_mut() {
                            *s = (*s + v).clamp(-1.0, 1.0);
                        }
                    }
                    None => break,
                }
            }
            if b.is_empty() {
                self.active.store(false, Ordering::Relaxed);
            }
        }
    }
}
