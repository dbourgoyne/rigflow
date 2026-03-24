use num_complex::Complex32;

use crate::dsp::audio::agc::Agc;
use crate::dsp::audio::audio_fir::AudioFir;
use crate::dsp::audio::dc_blocker::DcBlocker;
use crate::dsp::audio::deemphasis::DeemphasisFilter;
use crate::dsp::audio::resampler::AudioResampler;
use crate::dsp::decimator::PolyphaseDecimator;
use crate::dsp::demod::fm::FmDemodulator;
use crate::dsp::demod::ssb::SsbDemodulator;
use crate::dsp::demod::{DemodMode, Sideband};
use crate::dsp::tuner::VirtualTuner;

pub struct DspPipeline {
    tuner: VirtualTuner,
    channelizer: PolyphaseDecimator,
    mode: DemodMode,
    ssb_demod: SsbDemodulator,
    fm_demod: FmDemodulator,
    dc_blocker: DcBlocker,
    agc: Agc,
    audio_fir: Option<AudioFir>,
    deemphasis: Option<DeemphasisFilter>,
    resampler: Option<AudioResampler>,
    output_sample_rate_hz: f32,
    client_output_sample_rate_hz: f32,
}

impl DspPipeline {
    pub fn new(
        center_freq_hz: f32,
        target_freq_hz: f32,
        input_sample_rate_hz: f32,
        channel_cutoff_hz: f32,
        fir_taps: usize,
        decimation_factor: usize,
        audio_cutoff_hz: f32,
        audio_fir_taps: usize,
        client_output_sample_rate_hz: f32,
        mode: DemodMode,
    ) -> Self {
        let output_sample_rate_hz = input_sample_rate_hz / decimation_factor as f32;

        let resampler = if (output_sample_rate_hz - client_output_sample_rate_hz).abs() > 1.0 {
            Some(AudioResampler::new(
                output_sample_rate_hz,
                client_output_sample_rate_hz,
            ))
        } else {
            None
        };

        let deemphasis = match mode {
            DemodMode::Wfm => Some(DeemphasisFilter::new(output_sample_rate_hz, 75e-6)),
            _ => None,
        };

        Self {
            tuner: VirtualTuner::new(
                center_freq_hz,
                target_freq_hz,
                input_sample_rate_hz,
            ),
            channelizer: PolyphaseDecimator::new(
                input_sample_rate_hz,
                channel_cutoff_hz,
                fir_taps,
                decimation_factor,
            ),
            mode,
            ssb_demod: SsbDemodulator::new(Sideband::Usb),
            fm_demod: FmDemodulator::new(),
            dc_blocker: DcBlocker::new(0.995),
            agc: Agc::new(0.3, 0.9, 0.999, 20.0),
            audio_fir: if audio_cutoff_hz > 0.0 {
                Some(AudioFir::new(
                    output_sample_rate_hz,
                    audio_cutoff_hz,
                    audio_fir_taps,
                ))
            } else {
                None
            },
            deemphasis,
            resampler,
            output_sample_rate_hz,
            client_output_sample_rate_hz,
        }
    }

    pub fn set_mode(&mut self, mode: DemodMode) {
        self.mode = mode;

        if matches!(mode, DemodMode::Wfm) {
            self.fm_demod.reset();
        }
    }

    pub fn set_sideband(&mut self, sideband: Sideband) {
        self.ssb_demod.set_sideband(sideband);
    }

    pub fn set_target_frequency(&mut self, target_freq_hz: f32) {
        self.tuner.set_target_frequency(target_freq_hz);
    }

    pub fn set_center_frequency(&mut self, center_freq_hz: f32) {
        self.tuner.set_center_frequency(center_freq_hz);
    }

    pub fn process_iq(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut shifted = input.to_vec();
        self.tuner.process_in_place(&mut shifted);
        self.channelizer.process(&shifted)
    }

    pub fn process_audio(&mut self, input: &[Complex32]) -> Vec<f32> {
        let iq = self.process_iq(input);

        let mut audio = match self.mode {
            DemodMode::Usb => {
                self.ssb_demod.set_sideband(Sideband::Usb);
                self.ssb_demod.process(&iq)
            }
            DemodMode::Lsb => {
                self.ssb_demod.set_sideband(Sideband::Lsb);
                self.ssb_demod.process(&iq)
            }
            DemodMode::Wfm => self.fm_demod.process(&iq),
        };

        self.dc_blocker.process_in_place(&mut audio);

        if let Some(deemphasis) = &mut self.deemphasis {
            deemphasis.process_in_place(&mut audio);
        }

        self.agc.process_in_place(&mut audio);

        if let Some(fir) = &mut self.audio_fir {
            fir.process_in_place(&mut audio);
        }

        if let Some(resampler) = &mut self.resampler {
            resampler.process(&audio)
        } else {
            audio
        }
    }

    pub fn reset_audio_state(&mut self) {
        self.dc_blocker.reset();
        self.agc.reset();
        self.fm_demod.reset();

        if let Some(fir) = &mut self.audio_fir {
            fir.reset();
        }

        if let Some(deemphasis) = &mut self.deemphasis {
            deemphasis.reset();
        }

        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }
    }

    pub fn output_sample_rate(&self) -> f32 {
        self.output_sample_rate_hz
    }

    pub fn client_output_sample_rate(&self) -> f32 {
        self.client_output_sample_rate_hz
    }
}
