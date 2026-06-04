//! Inbound microphone-audio queue (client → server, SSB Mic TX Phase 3).
//!
//! The UDP listener pushes decoded mono-f32 mic samples here; the radio worker
//! drains them while keying SSB.  A single process-global ring is enough — one
//! client / one active TX worker at a time — and avoids threading a queue
//! through the manager → worker.  Loss-tolerant: on overrun the oldest samples
//! are dropped; on underrun the worker pads with silence.

use std::collections::VecDeque;
use std::sync::Mutex;

/// ~0.5 s at 48 kHz — enough to ride out jitter without adding much latency.
const MAX_SAMPLES: usize = 24_000;

static MIC_QUEUE: Mutex<VecDeque<f32>> = Mutex::new(VecDeque::new());

/// Push received mic samples; drop oldest past the cap (overrun).
pub fn push_mic_samples(samples: &[f32]) {
    if let Ok(mut q) = MIC_QUEUE.lock() {
        q.extend(samples.iter().copied());
        if q.len() > MAX_SAMPLES {
            let drop = q.len() - MAX_SAMPLES;
            q.drain(..drop);
            log::debug!("[mic-rx] tx audio overrun — dropped {drop} samples");
        }
    }
}

/// Drain up to `max` samples into `out` (returns the number drained).  Fewer
/// than `max` means underrun — the caller should pad with silence.
pub fn drain_mic_samples(out: &mut Vec<f32>, max: usize) -> usize {
    let mut n = 0;
    if let Ok(mut q) = MIC_QUEUE.lock() {
        while n < max {
            match q.pop_front() {
                Some(s) => {
                    out.push(s);
                    n += 1;
                }
                None => break,
            }
        }
    }
    n
}

/// Discard any buffered mic audio (called when a key-up/stop occurs so stale
/// audio doesn't leak into the next transmission).
pub fn clear_mic_samples() {
    if let Ok(mut q) = MIC_QUEUE.lock() {
        q.clear();
    }
}
