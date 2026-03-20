use num_complex::Complex32;

use crate::dsp::audio::agc::Agc;
use crate::dsp::audio::dc_blocker::DcBlocker;
use crate::dsp::decimator::Decimator;
use crate::dsp::demod::ssb::SsbDemodulator;
use crate::dsp::demod::Sideband;
use crate::dsp::filter::LowPassFir;
use crate::dsp::tuner::VirtualTuner;

pub struct DspPipeline {
    tuner: VirtualTuner,
    fir: LowPassFir,
    decimator: Decimator,
    ssb_demod: SsbDemodulator,
    dc_blocker: DcBlocker,
    agc: Agc,
}

impl DspPipeline {
    pub fn new(
        center_freq_hz: f32,
        target_freq_hz: f32,
        input_sample_rate_hz: f32,
        channel_cutoff_hz: f32,
        fir_taps: usize,
        decimation_factor: usize,
    ) -> Self {
        Self {
            tuner: VirtualTuner::new(
                center_freq_hz,
                target_freq_hz,
                input_sample_rate_hz,
            ),
            fir: LowPassFir::new(
                input_sample_rate_hz,
                channel_cutoff_hz,
                fir_taps,
            ),
            decimator: Decimator::new(decimation_factor),
            ssb_demod: SsbDemodulator::new(Sideband::Usb),
            dc_blocker: DcBlocker::new(0.995),
            agc: Agc::new(
                0.3,   // target level
                0.9,   // attack
                0.999, // decay
                20.0,  // max gain
            ),
        }
    }

    pub fn set_sideband(&mut self, sideband: Sideband) {
        self.ssb_demod.set_sideband(sideband);
    }

    pub fn process_iq(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut shifted = input.to_vec();

        self.tuner.process_in_place(&mut shifted);
        self.fir.process_in_place(&mut shifted);

        self.decimator.process(&shifted)
    }

    pub fn process_audio(&mut self, input: &[Complex32]) -> Vec<f32> {
        let iq = self.process_iq(input);
        let mut audio = self.ssb_demod.process(&iq);

        self.dc_blocker.process_in_place(&mut audio);
        self.agc.process_in_place(&mut audio);

        audio
    }

    pub fn reset_audio_state(&mut self) {
        self.dc_blocker.reset();
        self.agc.reset();
    }

    pub fn output_sample_rate(&self, input_sample_rate_hz: f32) -> f32 {
        input_sample_rate_hz / self.decimator.factor() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn process_audio_preserves_length_after_decimation() {
        let mut pipeline = DspPipeline::new(
            10_000.0,
            12_000.0,
            48_000.0,
            3_000.0,
            101,
            4,
        );

        let input: Vec<Complex32> = (0..4096)
            .map(|n| {
                let rel_tone_hz = 2_000.0;
                let phase = 2.0 * PI * rel_tone_hz * n as f32 / 48_000.0;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let audio = pipeline.process_audio(&input);

        assert!(!audio.is_empty());
        assert!(audio.len() < input.len());
    }
}