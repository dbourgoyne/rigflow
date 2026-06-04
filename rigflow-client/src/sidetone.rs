//! Local CW sidetone state, shared lock-free between the UI thread (writer) and
//! the CPAL audio callback (reader).
//!
//! The sidetone is generated entirely in the client audio output callback and
//! mixed into the final speaker stream just before output — it is never sent to
//! the server or over UDP.  This struct carries only the small control state
//! (keyed / pitch / volume) as atomics so the real-time audio callback never has
//! to take a lock; the oscillator phase and envelope live inside the callback.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[derive(Debug)]
pub struct SidetoneShared {
    /// True while the operator is keying (Space held in CWU/CWL).
    keyed: AtomicBool,
    /// Sidetone frequency in Hz (= the CW pitch).  Stored as f32 bits.
    pitch_hz_bits: AtomicU32,
    /// Sidetone level 0.0–1.0 (CW Sidetone Volume / 100).  Stored as f32 bits.
    volume_bits: AtomicU32,
}

impl Default for SidetoneShared {
    fn default() -> Self {
        Self {
            keyed: AtomicBool::new(false),
            pitch_hz_bits: AtomicU32::new(600.0f32.to_bits()),
            volume_bits: AtomicU32::new(0.25f32.to_bits()),
        }
    }
}

impl SidetoneShared {
    pub fn set_keyed(&self, keyed: bool) {
        self.keyed.store(keyed, Ordering::Relaxed);
    }

    pub fn keyed(&self) -> bool {
        self.keyed.load(Ordering::Relaxed)
    }

    pub fn set_pitch_hz(&self, hz: f32) {
        self.pitch_hz_bits
            .store(hz.max(0.0).to_bits(), Ordering::Relaxed);
    }

    pub fn pitch_hz(&self) -> f32 {
        f32::from_bits(self.pitch_hz_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }
}
