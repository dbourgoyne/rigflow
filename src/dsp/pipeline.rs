use num_complex::Complex32;

use crate::dsp::decimator::Decimator;
use crate::dsp::demod::Sideband;
use crate::dsp::demod::ssb::SsbDemodulator;
use crate::dsp::filter::LowPassFir;
use crate::dsp::tuner::VirtualTuner;

pub struct DspPipeline {
    tuner: VirtualTuner,
    fir: LowPassFir,
    decimator: Decimator,
    ssb_demod: SsbDemodulator,
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
        self.ssb_demod.process(&iq)
    }

    pub fn output_sample_rate(&self, input_sample_rate_hz: f32) -> f32 {
        input_sample_rate_hz / self.decimator.factor() as f32
    }
}