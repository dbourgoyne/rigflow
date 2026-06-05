//! Process-global TX-audio diagnostics for SSB microphone transmit.
//!
//! **Diagnostics only** — nothing here changes transmitted audio.  Live levels
//! (RMS / peak / clip) are written by the mic-TX loop immediately before SSB
//! modulation; the underrun counter is bumped there too, and the overrun
//! counter by the inbound mic queue.  A single global mirrors the
//! `net::udp::mic_audio` MIC_QUEUE design — one client / one active TX worker
//! at a time — and is read by the worker's DSP thread to publish a
//! [`TxAudioDiag`] snapshot to the client via `RuntimeChanged`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rigflow_core::radio::tx_audio_diag::TxAudioDiag;

static RMS_BITS: AtomicU32 = AtomicU32::new(0);
static PEAK_BITS: AtomicU32 = AtomicU32::new(0);
static CLIPPING: AtomicBool = AtomicBool::new(false);
static GAIN_REDUCTION_BITS: AtomicU32 = AtomicU32::new(0);
static COMP_REDUCTION_BITS: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS: AtomicU64 = AtomicU64::new(0);
static OVERRUNS: AtomicU64 = AtomicU64::new(0);

/// Publish the live meters for the current measurement window (called ~20 Hz
/// from the mic-TX loop while keyed).  `clipping` is already held by the caller;
/// `limiter_gr_db` / `compressor_gr_db` are the limiter and compressor gain
/// reductions (≥0).
pub fn set_levels(rms: f32, peak: f32, clipping: bool, limiter_gr_db: f32, compressor_gr_db: f32) {
    RMS_BITS.store(rms.to_bits(), Ordering::Relaxed);
    PEAK_BITS.store(peak.to_bits(), Ordering::Relaxed);
    CLIPPING.store(clipping, Ordering::Relaxed);
    GAIN_REDUCTION_BITS.store(limiter_gr_db.to_bits(), Ordering::Relaxed);
    COMP_REDUCTION_BITS.store(compressor_gr_db.to_bits(), Ordering::Relaxed);
}

/// Drop the live meters to silence (called on key-up) so the meter falls to
/// zero between overs.  Counters are left untouched.
pub fn clear_levels() {
    RMS_BITS.store(0, Ordering::Relaxed);
    PEAK_BITS.store(0, Ordering::Relaxed);
    CLIPPING.store(false, Ordering::Relaxed);
    GAIN_REDUCTION_BITS.store(0, Ordering::Relaxed);
    COMP_REDUCTION_BITS.store(0, Ordering::Relaxed);
}

/// One TX-audio underrun event (modulator requested audio, buffer was empty).
pub fn incr_underruns() {
    UNDERRUNS.fetch_add(1, Ordering::Relaxed);
}

/// One TX-audio overrun event (producer outran the consumer; samples dropped).
pub fn incr_overruns() {
    OVERRUNS.fetch_add(1, Ordering::Relaxed);
}

/// Reset the underrun/overrun counters (operator "Reset Counters").
pub fn reset_counters() {
    UNDERRUNS.store(0, Ordering::Relaxed);
    OVERRUNS.store(0, Ordering::Relaxed);
}

/// Snapshot the current diagnostics for telemetry.
pub fn snapshot() -> TxAudioDiag {
    TxAudioDiag {
        rms: f32::from_bits(RMS_BITS.load(Ordering::Relaxed)),
        peak: f32::from_bits(PEAK_BITS.load(Ordering::Relaxed)),
        clipping: CLIPPING.load(Ordering::Relaxed),
        gain_reduction_db: f32::from_bits(GAIN_REDUCTION_BITS.load(Ordering::Relaxed)),
        compressor_reduction_db: f32::from_bits(COMP_REDUCTION_BITS.load(Ordering::Relaxed)),
        underruns: UNDERRUNS.load(Ordering::Relaxed),
        overruns: OVERRUNS.load(Ordering::Relaxed),
    }
}
