//! Lock-free shared audio/latency metrics.
//!
//! The media runtime thread **publishes** into this struct (RX jitter-buffer
//! occupancy + health each loop, network clock-offset / one-way latency from the
//! UDP `TIME_SYNC` probe and per-packet audio send-stamps); the dedicated mic-send
//! thread publishes the TX-ring depth. The egui UI thread **reads** it each frame
//! for the Latency panel. All fields are atomics so neither side ever blocks.
//!
//! Displayed values are **smoothed at the producer** (slow EMA + a decaying recent
//! peak) so they change ~1×/s and are readable, even though the UI reads every
//! frame and the underlying buffers oscillate quickly.
//!
//! TX server-queue depth is read elsewhere from `UiState::tx_audio_diag.mic_queue_samples`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

/// EMA weight for the smoothed jitter average. Producers publish ~200×/s, so this
/// gives a ~0.5 s time constant → a steady, readable number.
const JITTER_AVG_ALPHA: f32 = 0.01;
/// Per-update decay for the recent jitter peak (~5 s window at ~200 Hz).
const JITTER_PEAK_DECAY: f32 = 0.999;
/// Per-update decay for the recent mic-ring peak (~0.5 s window at ~200 Hz) — TX is
/// bursty and should fall back quickly once keying stops.
const TX_RING_PEAK_DECAY: f32 = 0.99;

fn load_f32(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}
fn store_f32(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

#[derive(Debug)]
pub struct AudioMetrics {
    // RX jitter buffer — smoothed for display (ms).
    jitter_avg_ms: AtomicU32,  // slow EMA of current depth
    jitter_peak_ms: AtomicU32, // decaying recent peak
    have_jitter: AtomicBool,
    // RX health (cumulative).
    conceals: AtomicU64,
    late: AtomicU64,
    overflow: AtomicU64,
    resyncs: AtomicU64,

    // TX mic-ring depth — decaying recent peak for display (ms).
    tx_ring_peak_ms: AtomicU32,

    // Network timing (from the clock-offset probe + per-packet audio send-stamp).
    offset_ns: AtomicI64, // server − client clock offset
    rtt_ns: AtomicI64,    // last accepted probe round-trip
    one_way_ns: AtomicI64,
    have_offset: AtomicBool,
    have_one_way: AtomicBool,
    min_rtt_ns: AtomicI64, // running-min RTT used to gate clean offset samples
}

impl Default for AudioMetrics {
    fn default() -> Self {
        Self {
            jitter_avg_ms: AtomicU32::new(0),
            jitter_peak_ms: AtomicU32::new(0),
            have_jitter: AtomicBool::new(false),
            conceals: AtomicU64::new(0),
            late: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            resyncs: AtomicU64::new(0),
            tx_ring_peak_ms: AtomicU32::new(0),
            offset_ns: AtomicI64::new(0),
            rtt_ns: AtomicI64::new(0),
            one_way_ns: AtomicI64::new(0),
            have_offset: AtomicBool::new(false),
            have_one_way: AtomicBool::new(false),
            min_rtt_ns: AtomicI64::new(i64::MAX),
        }
    }
}

impl AudioMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // ---- producer side ---------------------------------------------------

    /// Publish the current RX jitter-buffer occupancy + cumulative health. Called
    /// from the media thread (~200×/s). The displayed average is a slow EMA and the
    /// peak decays over a few seconds, so the readout is steady. Single producer →
    /// the read-modify-write on the smoothed atomics is race-free.
    pub fn publish_jitter(
        &self,
        samples: usize,
        conceals: u64,
        late: u64,
        overflow: u64,
        resyncs: u64,
    ) {
        let cur_ms = samples as f32 / SAMPLE_RATE_HZ * 1000.0;

        if self.have_jitter.load(Ordering::Relaxed) {
            let avg = load_f32(&self.jitter_avg_ms);
            store_f32(&self.jitter_avg_ms, avg + JITTER_AVG_ALPHA * (cur_ms - avg));
            let peak = load_f32(&self.jitter_peak_ms) * JITTER_PEAK_DECAY;
            store_f32(&self.jitter_peak_ms, cur_ms.max(peak));
        } else {
            store_f32(&self.jitter_avg_ms, cur_ms);
            store_f32(&self.jitter_peak_ms, cur_ms);
            self.have_jitter.store(true, Ordering::Relaxed);
        }

        self.conceals.store(conceals, Ordering::Relaxed);
        self.late.store(late, Ordering::Relaxed);
        self.overflow.store(overflow, Ordering::Relaxed);
        self.resyncs.store(resyncs, Ordering::Relaxed);
    }

    /// Note the current TX mic-ring depth (samples drained this cycle), called from
    /// the mic-send thread (~200×/s, including 0 while idle so the peak decays).
    pub fn note_tx_ring(&self, samples: usize) {
        let ms = samples as f32 / SAMPLE_RATE_HZ * 1000.0;
        let peak = load_f32(&self.tx_ring_peak_ms) * TX_RING_PEAK_DECAY;
        store_f32(&self.tx_ring_peak_ms, ms.max(peak));
    }

    /// Feed one clock-offset probe result (offset + RTT, both ns) from a
    /// `TIME_SYNC` response. Accepts the sample only when its RTT is near the
    /// running minimum (low queuing ⇒ cleanest offset), then EMA-updates the
    /// published offset; the min relaxes slowly upward to track drift.
    pub fn record_probe(&self, offset_ns: i64, rtt_ns: i64) {
        if rtt_ns <= 0 {
            return;
        }
        let prev_min = self.min_rtt_ns.load(Ordering::Relaxed);
        let relaxed = if prev_min == i64::MAX {
            rtt_ns
        } else {
            prev_min + prev_min / 64
        };
        let min_rtt = rtt_ns.min(relaxed);
        self.min_rtt_ns.store(min_rtt, Ordering::Relaxed);
        self.rtt_ns.store(rtt_ns, Ordering::Relaxed);

        // Reject network spikes: only trust RTTs within 1.5× of the cleanest.
        if rtt_ns > min_rtt + min_rtt / 2 {
            return;
        }

        let next = if self.have_offset.load(Ordering::Relaxed) {
            let prev = self.offset_ns.load(Ordering::Relaxed);
            prev + (offset_ns - prev) / 4 // EMA, α = 1/4
        } else {
            self.have_offset.store(true, Ordering::Relaxed);
            offset_ns
        };
        self.offset_ns.store(next, Ordering::Relaxed);
    }

    /// Feed one audio packet's timestamps: compute & EMA-smooth the one-way
    /// network latency (`recv_client − send_server + offset`). No-op until an
    /// offset is known; absurd values (clock jumps) are ignored.
    pub fn record_audio_one_way(&self, recv_ns: u64, send_ns: u64) {
        if !self.have_offset.load(Ordering::Relaxed) {
            return;
        }
        let offset = self.offset_ns.load(Ordering::Relaxed) as i128;
        let one_way = (recv_ns as i128 - send_ns as i128 + offset) as i64;
        if !(-1_000_000_000..=5_000_000_000).contains(&one_way) {
            return;
        }
        let next = if self.have_one_way.load(Ordering::Relaxed) {
            let prev = self.one_way_ns.load(Ordering::Relaxed);
            prev + (one_way - prev) / 8 // slower EMA for a steady readout
        } else {
            self.have_one_way.store(true, Ordering::Relaxed);
            one_way
        };
        self.one_way_ns.store(next, Ordering::Relaxed);
    }

    // ---- reader side (UI) ------------------------------------------------

    /// Smoothed RX jitter-buffer depth in ms (slow EMA).
    pub fn jitter_ms(&self) -> f32 {
        load_f32(&self.jitter_avg_ms)
    }
    /// Recent (decaying) RX jitter-buffer peak in ms.
    pub fn jitter_peak_ms(&self) -> f32 {
        load_f32(&self.jitter_peak_ms)
    }
    /// Recent (decaying) TX mic-ring depth in ms.
    pub fn tx_ring_peak_ms(&self) -> f32 {
        load_f32(&self.tx_ring_peak_ms)
    }
    pub fn conceals(&self) -> u64 {
        self.conceals.load(Ordering::Relaxed)
    }
    pub fn late(&self) -> u64 {
        self.late.load(Ordering::Relaxed)
    }
    pub fn overflow(&self) -> u64 {
        self.overflow.load(Ordering::Relaxed)
    }
    pub fn resyncs(&self) -> u64 {
        self.resyncs.load(Ordering::Relaxed)
    }

    /// Smoothed RX network one-way latency in ms, once known.
    pub fn rx_one_way_ms(&self) -> Option<f32> {
        self.have_one_way
            .load(Ordering::Relaxed)
            .then(|| self.one_way_ns.load(Ordering::Relaxed) as f32 / 1e6)
    }
    /// Last accepted probe round-trip in ms, once a probe has succeeded.
    pub fn rtt_ms(&self) -> Option<f32> {
        self.have_offset
            .load(Ordering::Relaxed)
            .then(|| self.rtt_ns.load(Ordering::Relaxed) as f32 / 1e6)
    }
    /// Estimated server−client clock offset in ms, once known.
    pub fn clock_offset_ms(&self) -> Option<f32> {
        self.have_offset
            .load(Ordering::Relaxed)
            .then(|| self.offset_ns.load(Ordering::Relaxed) as f32 / 1e6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2880 samples @ 48 kHz = 60 ms.
    const MS60_SAMPLES: usize = 2_880;

    #[test]
    fn jitter_first_publish_initializes_then_ema_converges() {
        let m = AudioMetrics::default();
        m.publish_jitter(MS60_SAMPLES, 0, 0, 0, 0);
        // First publish seeds avg + peak directly.
        assert!((m.jitter_ms() - 60.0).abs() < 0.01);
        assert!((m.jitter_peak_ms() - 60.0).abs() < 0.01);
        // Drive toward 30 ms; the slow EMA should move part-way, not jump.
        for _ in 0..50 {
            m.publish_jitter(MS60_SAMPLES / 2, 0, 0, 0, 0);
        }
        let avg = m.jitter_ms();
        assert!(
            avg > 30.0 && avg < 60.0,
            "avg moved but not instantly: {avg}"
        );
    }

    #[test]
    fn jitter_peak_decays_after_a_spike() {
        let m = AudioMetrics::default();
        m.publish_jitter(MS60_SAMPLES * 2, 0, 0, 0, 0); // 120 ms spike
        let p0 = m.jitter_peak_ms();
        assert!((p0 - 120.0).abs() < 0.1);
        for _ in 0..2000 {
            m.publish_jitter(0, 0, 0, 0, 0); // low for a while
        }
        assert!(m.jitter_peak_ms() < p0, "recent peak decays");
    }

    #[test]
    fn tx_ring_peak_tracks_then_decays() {
        let m = AudioMetrics::default();
        m.note_tx_ring(480); // 10 ms burst
        assert!((m.tx_ring_peak_ms() - 10.0).abs() < 0.1);
        for _ in 0..2000 {
            m.note_tx_ring(0);
        }
        assert!(m.tx_ring_peak_ms() < 1.0, "decays toward 0 while idle");
    }

    #[test]
    fn probe_gating_and_offset_ema() {
        let m = AudioMetrics::default();
        m.record_probe(1000, 100);
        assert_eq!(m.clock_offset_ms(), Some(1000.0 / 1e6));
        m.record_probe(9999, 10_000); // spike rejected
        assert_eq!(m.clock_offset_ms(), Some(1000.0 / 1e6));
        m.record_probe(2000, 100); // EMA α = 1/4
        assert_eq!(m.clock_offset_ms(), Some(1250.0 / 1e6));
    }

    #[test]
    fn one_way_needs_offset_then_smooths() {
        let m = AudioMetrics::default();
        m.record_audio_one_way(1_000, 900);
        assert_eq!(m.rx_one_way_ms(), None);
        m.record_probe(0, 50);
        m.record_audio_one_way(1_100, 1_000);
        assert_eq!(m.rx_one_way_ms(), Some(100.0 / 1e6));
    }
}
