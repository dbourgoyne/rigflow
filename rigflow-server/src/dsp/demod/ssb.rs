use num_complex::Complex32;

/// Very simple first-pass SSB demodulator.
///
/// Assumes the desired SSB signal has already been:
/// - tuned close to baseband
/// - sideband-selected in the pipeline
/// - low-pass filtered
/// - optionally decimated
///
/// Current behavior is intentionally simple:
/// - audio is taken from the real component only
/// - configurable gain is applied
///
/// Later improvements can add:
/// - direct `process_into(...)` support
/// - tighter integration with sideband-specific DSP
/// - more advanced audio shaping if needed
pub struct SsbDemodulator {
    audio_gain: f32,
}

impl SsbDemodulator {
    /// Construct a simple SSB demodulator.
    ///
    /// The sideband parameter is currently ignored because sideband selection
    /// now happens earlier in the pipeline.
    pub fn new(_sideband: crate::dsp::demod::Sideband) -> Self {
        Self { audio_gain: 1.0 }
    }

    /// Construct with an explicit output gain.
    ///
    /// The sideband parameter is currently ignored because sideband selection
    /// now happens earlier in the pipeline.
    pub fn with_gain(_sideband: crate::dsp::demod::Sideband, audio_gain: f32) -> Self {
        Self { audio_gain }
    }

    /// Sideband selection is currently handled upstream in `pipeline.rs`.
    pub fn set_sideband(&mut self, _sideband: crate::dsp::demod::Sideband) {
        // No-op by design.
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.audio_gain = gain;
    }

    /// Demodulate SSB audio from complex baseband.
    ///
    /// Current first-pass behavior uses only the real component after the
    /// pipeline has already isolated the desired sideband.
    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        input
            .iter()
            .map(|sample| sample.re * self.audio_gain)
            .collect()
    }
}
