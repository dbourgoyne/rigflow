use num_complex::Complex32;
use std::f32::consts::PI;

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

const WFM_DEEMPHASIS_TAU_SECONDS: f32 = 75e-6;
const WFM_AUDIO_GAIN: f32 = 1.5;

/// Complex sideband-selective FIR built by modulating a low-pass prototype.
/// USB keeps roughly 0..B, LSB keeps roughly -B..0.
struct ComplexSidebandFir {
    taps: Vec<Complex32>,
    delay: Vec<Complex32>,
    pos: usize,
}

impl ComplexSidebandFir {
    fn new(
        sample_rate_hz: f32,
        audio_bandwidth_hz: f32,
        pitch_hz: f32,
        taps_len: usize,
        sideband: Sideband,
    ) -> Self {
        let taps = design_sideband_taps(
            sample_rate_hz,
            audio_bandwidth_hz,
            pitch_hz,
            taps_len,
            sideband,
        );
        let delay = vec![Complex32::new(0.0, 0.0); taps_len];

        Self { taps, delay, pos: 0 }
    }

    fn reset(&mut self) {
        for x in &mut self.delay {
            *x = Complex32::new(0.0, 0.0);
        }
        self.pos = 0;
    }

    fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut out = Vec::with_capacity(input.len());
        let n = self.taps.len();

        for &x in input {
            self.delay[self.pos] = x;

            let mut acc = Complex32::new(0.0, 0.0);
            let mut idx = self.pos;

            for tap in &self.taps {
                acc += self.delay[idx] * *tap;

                if idx == 0 {
                    idx = n - 1;
                } else {
                    idx -= 1;
                }
            }

            out.push(acc);

            self.pos += 1;
            if self.pos >= n {
                self.pos = 0;
            }
        }

        out
    }
}

fn sinc(x: f32) -> f32 {
    if x.abs() < 1e-8 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

fn design_sideband_taps(
    sample_rate_hz: f32,
    audio_bandwidth_hz: f32,
    pitch_hz: f32,
    taps_len: usize,
    sideband: Sideband,
) -> Vec<Complex32> {
    let taps_len = taps_len.max(3) | 1;
    let m = (taps_len - 1) as f32 / 2.0;

    let lp_cutoff_hz = (audio_bandwidth_hz * 0.5).max(100.0);
    let shift_hz = audio_bandwidth_hz * 0.5 + pitch_hz;

    let sign = match sideband {
        Sideband::Usb => 1.0,
        Sideband::Lsb => -1.0,
    };

    let fc = lp_cutoff_hz / sample_rate_hz;
    let fshift = sign * shift_hz / sample_rate_hz;

    let mut taps = Vec::with_capacity(taps_len);

    for i in 0..taps_len {
        let n = i as f32 - m;

        let h_lp = 2.0 * fc * sinc(2.0 * fc * n);
        let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (taps_len as f32 - 1.0)).cos();

        let phase = 2.0 * PI * fshift * n;
        let osc = Complex32::new(phase.cos(), phase.sin());

        taps.push(osc * (h_lp * w));
    }

    let sum_mag: f32 = taps.iter().map(|t| t.norm()).sum();
    if sum_mag > 0.0 {
        for t in &mut taps {
            *t /= sum_mag / 2.0;
        }
    }

    taps
}

pub struct DspPipeline {
    tuner: VirtualTuner,
    channelizer: PolyphaseDecimator,
    mode: DemodMode,
    sideband: Sideband,

    ssb_demod: SsbDemodulator,
    fm_demod: FmDemodulator,

    dc_blocker: DcBlocker,
    agc: Agc,
    audio_fir: Option<AudioFir>,
    deemphasis: Option<DeemphasisFilter>,
    resampler: Option<AudioResampler>,

    ssb_usb_fir: Option<ComplexSidebandFir>,
    ssb_lsb_fir: Option<ComplexSidebandFir>,
    ssb_bandwidth_hz: f32,
    ssb_fir_taps: usize,
    ssb_pitch_hz: f32,
    
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
            DemodMode::Wfm => Some(DeemphasisFilter::new(
                output_sample_rate_hz,
                WFM_DEEMPHASIS_TAU_SECONDS,
            )),
            _ => None,
        };

        let audio_fir = if audio_cutoff_hz > 0.0 {
            Some(AudioFir::new(
                output_sample_rate_hz,
                audio_cutoff_hz,
                audio_fir_taps,
            ))
        } else {
            None
        };

        // For now, use audio_cutoff_hz as the SSB audio bandwidth too.
        // If you later split the config, give SSB its own bandwidth.
        let ssb_bandwidth_hz = audio_cutoff_hz.max(300.0);
        let ssb_fir_taps = audio_fir_taps.max(31) | 1;
	let ssb_pitch_hz = 0.0;

	let ssb_usb_fir = Some(ComplexSidebandFir::new(
	    output_sample_rate_hz,
	    ssb_bandwidth_hz,
	    ssb_pitch_hz,
	    ssb_fir_taps,
	    Sideband::Usb,
	));

	let ssb_lsb_fir = Some(ComplexSidebandFir::new(
	    output_sample_rate_hz,
	    ssb_bandwidth_hz,
	    ssb_pitch_hz,
	    ssb_fir_taps,
	    Sideband::Lsb,
	));

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
            sideband: Sideband::Usb,
            ssb_demod: SsbDemodulator::new(Sideband::Usb),
            fm_demod: FmDemodulator::new(),
            dc_blocker: DcBlocker::new(0.995),
            agc: Agc::new(0.3, 0.9, 0.999, 20.0),
            audio_fir,
            deemphasis,
            resampler,
            ssb_usb_fir,
            ssb_lsb_fir,
            ssb_bandwidth_hz,
            ssb_fir_taps,
	    ssb_pitch_hz,
            output_sample_rate_hz,
            client_output_sample_rate_hz,
        }
    }

    pub fn set_mode(&mut self, mode: DemodMode) {
        self.mode = mode;

        self.deemphasis = match mode {
            DemodMode::Wfm => Some(DeemphasisFilter::new(
                self.output_sample_rate_hz,
                WFM_DEEMPHASIS_TAU_SECONDS,
            )),
            _ => None,
        };

        self.reset_audio_state();
    }

    pub fn set_sideband(&mut self, sideband: Sideband) {
        self.sideband = sideband;
        self.reset_audio_state();
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
                self.sideband = Sideband::Usb;
                let filtered = self
                    .ssb_usb_fir
                    .as_mut()
                    .map(|f| f.process(&iq))
                    .unwrap_or_else(|| iq.clone());
                self.ssb_demod.process(&filtered)
            }

            DemodMode::Lsb => {
                self.sideband = Sideband::Lsb;
                let filtered = self
                    .ssb_lsb_fir
                    .as_mut()
                    .map(|f| f.process(&iq))
                    .unwrap_or_else(|| iq.clone());
                self.ssb_demod.process(&filtered)
            }

            DemodMode::Wfm => self.fm_demod.process(&iq),
        };

        self.dc_blocker.process_in_place(&mut audio);

        match self.mode {
            DemodMode::Wfm => {
                if let Some(fir) = &mut self.audio_fir {
                    fir.process_in_place(&mut audio);
                }

                if let Some(deemphasis) = &mut self.deemphasis {
                    deemphasis.process_in_place(&mut audio);
                }

                for s in &mut audio {
                    *s *= WFM_AUDIO_GAIN;
                    *s = s.tanh();
                }
            }

            DemodMode::Usb | DemodMode::Lsb => {
                self.agc.process_in_place(&mut audio);

                if let Some(fir) = &mut self.audio_fir {
                    fir.process_in_place(&mut audio);
                }
            }
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

        if let Some(fir) = &mut self.ssb_usb_fir {
            fir.reset();
        }

        if let Some(fir) = &mut self.ssb_lsb_fir {
            fir.reset();
        }
    }

    pub fn output_sample_rate(&self) -> f32 {
        self.output_sample_rate_hz
    }

    pub fn client_output_sample_rate(&self) -> f32 {
        self.client_output_sample_rate_hz
    }

    pub fn ssb_bandwidth_hz(&self) -> f32 {
        self.ssb_bandwidth_hz
    }

    pub fn rebuild_ssb_filters(&mut self, bandwidth_hz: f32, taps: usize) {
	self.ssb_bandwidth_hz = bandwidth_hz.max(300.0);
	self.ssb_fir_taps = taps.max(31) | 1;
	
	self.ssb_usb_fir = Some(ComplexSidebandFir::new(
            self.output_sample_rate_hz,
            self.ssb_bandwidth_hz,
            self.ssb_pitch_hz,
            self.ssb_fir_taps,
            Sideband::Usb,
	));
	
	self.ssb_lsb_fir = Some(ComplexSidebandFir::new(
            self.output_sample_rate_hz,
            self.ssb_bandwidth_hz,
            self.ssb_pitch_hz,
            self.ssb_fir_taps,
            Sideband::Lsb,
	));
    }

    pub fn set_ssb_pitch_hz(&mut self, pitch_hz: f32) {
	println!("pipeline set_ssb_pitch_hz: {}", pitch_hz);
	self.ssb_pitch_hz = pitch_hz;
	self.rebuild_ssb_filters(self.ssb_bandwidth_hz, self.ssb_fir_taps);
    }
    
}
