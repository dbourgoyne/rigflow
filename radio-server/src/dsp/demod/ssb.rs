/// Very simple first-pass SSB demodulator.
///
/// Assumes the desired SSB signal has already been:
/// - tuned close to baseband
/// - low-pass filtered
/// - optionally decimated
///
/// This version extracts audio from the complex baseband stream.
/// It is intentionally simple and good for getting the pipeline working.
///
/// Later improvements can add:
/// - AGC
/// - DC blocking
/// - audio low-pass filtering
/// - better sideband image rejection
///
use num_complex::Complex32;

pub struct SsbDemodulator {
    audio_gain: f32,
}

impl SsbDemodulator {
    pub fn new(_sideband: crate::dsp::demod::Sideband) -> Self {
        Self { audio_gain: 1.0 }
    }

    pub fn with_gain(_sideband: crate::dsp::demod::Sideband, audio_gain: f32) -> Self {
        Self { audio_gain }
    }

    pub fn set_sideband(&mut self, _sideband: crate::dsp::demod::Sideband) {
        // no-op: sideband selection now happens in pipeline.rs
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.audio_gain = gain;
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        input.iter().map(|s| s.re * self.audio_gain).collect()
    }
}
