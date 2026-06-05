use std::f32::consts::PI;

use log::info;
use num_complex::Complex32;

use crate::dsp::audio::agc::Agc;
use crate::dsp::audio::audio_fir::AudioFir;
use crate::dsp::audio::dc_blocker::DcBlocker;
use crate::dsp::audio::deemphasis::DeemphasisFilter;
use crate::dsp::audio::resampler::AudioResampler;
use crate::dsp::decimator::PolyphaseDecimator;
use crate::dsp::demod::am::AmDemodulator;
use crate::dsp::demod::cw::CwDemodulator;
use crate::dsp::demod::fm::FmDemodulator;
use crate::dsp::demod::ssb::SsbDemodulator;
use crate::dsp::demod::{DemodMode, Sideband};
use crate::dsp::tuner::VirtualTuner;
use rigflow_core::dsp::modes::{clamp_filter_bandwidth, default_deemphasis_mode, DeemphasisMode};

#[derive(Debug, Clone)]
pub struct DspPipelineConfig {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub input_sample_rate_hz: f32,

    pub channel_cutoff_hz: f32,
    pub fir_taps: usize,
    pub decimation_factor: usize,

    pub audio_cutoff_hz: f32,
    pub audio_fir_taps: usize,

    pub client_output_sample_rate_hz: f32,
    pub mode: DemodMode,
}

const WFM_AUDIO_GAIN: f32 = 1.5;
const NFM_AUDIO_GAIN: f32 = 12.0;

/// Complex FIR used to isolate one sideband by modulating a low-pass prototype.
///
/// USB keeps roughly 0..B and LSB keeps roughly -B..0 in complex baseband.
pub(crate) struct ComplexSidebandFir {
    taps: Vec<Complex32>,
    delay: Vec<Complex32>,
    pos: usize,
}

impl ComplexSidebandFir {
    pub(crate) fn new(
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

        Self {
            taps,
            delay,
            pos: 0,
        }
    }

    fn reset(&mut self) {
        for sample in &mut self.delay {
            *sample = Complex32::new(0.0, 0.0);
        }
        self.pos = 0;
    }

    pub(crate) fn process_into(&mut self, input: &[Complex32], out: &mut Vec<Complex32>) {
        out.clear();
        out.reserve(input.len().saturating_sub(out.capacity()));

        let tap_count = self.taps.len();

        for &sample in input {
            self.delay[self.pos] = sample;

            let mut acc = Complex32::new(0.0, 0.0);
            let mut idx = self.pos;

            for tap in &self.taps {
                acc += self.delay[idx] * *tap;

                if idx == 0 {
                    idx = tap_count - 1;
                } else {
                    idx -= 1;
                }
            }

            out.push(acc);

            self.pos += 1;
            if self.pos >= tap_count {
                self.pos = 0;
            }
        }
    }
}

fn sinc(x: f32) -> f32 {
    if x.abs() < 1e-8 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

pub(crate) fn design_sideband_taps(
    sample_rate_hz: f32,
    audio_bandwidth_hz: f32,
    pitch_hz: f32,
    taps_len: usize,
    sideband: Sideband,
) -> Vec<Complex32> {
    let taps_len = taps_len.max(3) | 1;
    let mid = (taps_len - 1) as f32 / 2.0;

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
        let n = i as f32 - mid;

        let h_lp = 2.0 * fc * sinc(2.0 * fc * n);
        let window = 0.5 - 0.5 * (2.0 * PI * i as f32 / (taps_len as f32 - 1.0)).cos();

        let phase = 2.0 * PI * fshift * n;
        let osc = Complex32::new(phase.cos(), phase.sin());

        taps.push(osc * (h_lp * window));
    }

    let sum_mag: f32 = taps.iter().map(|tap| tap.norm()).sum();
    if sum_mag > 0.0 {
        for tap in &mut taps {
            *tap /= sum_mag / 2.0;
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
    am_demod: AmDemodulator,
    cw_demod: CwDemodulator,

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
    cw_pitch_hz: f32,
    filter_bandwidth_hz: f32,
    deemphasis_mode: DeemphasisMode,

    output_sample_rate_hz: f32,
    client_output_sample_rate_hz: f32,

    // Reused scratch buffer for tuner output.
    tuned_iq_scratch: Vec<Complex32>,

    // Reused scratch buffer for channelized/decimated IQ.
    channelized_iq_scratch: Vec<Complex32>,

    // Reused scratch buffer for SSB sideband filtering.
    ssb_filtered_scratch: Vec<Complex32>,

    // Mean channel power (|z|^2) of the most recent post-channel-filter,
    // pre-demod IQ block — the S-meter measurement point.  Mode-independent.
    last_channel_power: f32,
}

impl DspPipeline {
    pub fn new(cfg: DspPipelineConfig) -> Self {
        let output_sample_rate_hz = cfg.input_sample_rate_hz / cfg.decimation_factor as f32;

        let resampler = if (output_sample_rate_hz - cfg.client_output_sample_rate_hz).abs() > 1.0 {
            Some(AudioResampler::new(
                output_sample_rate_hz,
                cfg.client_output_sample_rate_hz,
            ))
        } else {
            None
        };

        let ssb_fir_taps = cfg.audio_fir_taps.max(31) | 1;
        let ssb_pitch_hz = 0.0;
        let cw_pitch_hz = 600.0;
        let deemphasis_mode = default_deemphasis_mode(cfg.mode).unwrap_or(DeemphasisMode::Off);

        let sideband = match cfg.mode {
            DemodMode::Usb => Sideband::Usb,
            DemodMode::Lsb => Sideband::Lsb,
            _ => Sideband::Usb,
        };

        let mut pipeline = Self {
            tuner: VirtualTuner::new(
                cfg.center_freq_hz,
                cfg.target_freq_hz,
                cfg.input_sample_rate_hz,
            ),
            channelizer: PolyphaseDecimator::new(
                cfg.input_sample_rate_hz,
                cfg.channel_cutoff_hz,
                cfg.fir_taps,
                cfg.decimation_factor,
            ),
            mode: cfg.mode,
            sideband,
            ssb_demod: SsbDemodulator::new(sideband),
            fm_demod: FmDemodulator::new(),
            am_demod: AmDemodulator::new(),
            cw_demod: CwDemodulator::new(output_sample_rate_hz, cw_pitch_hz),
            dc_blocker: DcBlocker::new(0.995),
            agc: Agc::new(0.3, 0.9, 0.999, 20.0),
            audio_fir: None,
            deemphasis: None,
            resampler,
            ssb_usb_fir: None,
            ssb_lsb_fir: None,
            ssb_bandwidth_hz: cfg.audio_cutoff_hz.max(300.0),
            ssb_fir_taps,
            ssb_pitch_hz,
            cw_pitch_hz,
            deemphasis_mode,
            filter_bandwidth_hz: clamp_filter_bandwidth(cfg.mode, cfg.audio_cutoff_hz),
            output_sample_rate_hz,
            client_output_sample_rate_hz: cfg.client_output_sample_rate_hz,
            tuned_iq_scratch: Vec::new(),
            channelized_iq_scratch: Vec::new(),
            ssb_filtered_scratch: Vec::new(),
            last_channel_power: 0.0,
        };

        pipeline.rebuild_audio_path_for_mode();
        pipeline.reset_audio_state();
        pipeline
    }

    fn rebuild_audio_path_for_mode(&mut self) {
        info!(
            "rebuild_audio_path_for_mode: demod={:?} deemphasis_mode={:?} tau={:?}",
            self.mode,
            self.deemphasis_mode,
            Self::deemphasis_tau_for(self.mode, self.deemphasis_mode)
        );
        let bandwidth_hz = clamp_filter_bandwidth(self.mode, self.filter_bandwidth_hz);
        self.filter_bandwidth_hz = bandwidth_hz;

        self.audio_fir = Some(AudioFir::new(
            self.output_sample_rate_hz,
            bandwidth_hz,
            self.ssb_fir_taps,
        ));

        self.deemphasis = match Self::deemphasis_tau_for(self.mode, self.deemphasis_mode) {
            Some(tau_seconds) => Some(DeemphasisFilter::new(
                self.output_sample_rate_hz,
                tau_seconds,
            )),
            None => None,
        };

        match self.mode {
            DemodMode::Usb | DemodMode::Lsb => {
                self.rebuild_ssb_filters(bandwidth_hz, self.ssb_fir_taps);
                self.ssb_demod = SsbDemodulator::new(self.sideband);
            }
            DemodMode::Cwu | DemodMode::Cwl => {
                self.cw_demod = CwDemodulator::new(self.output_sample_rate_hz, self.cw_pitch_hz);
            }
            DemodMode::Am | DemodMode::Nfm | DemodMode::Wfm => {}
        }
    }

    pub fn set_mode(&mut self, mode: DemodMode) {
        self.mode = mode;

        match mode {
            DemodMode::Usb => self.sideband = Sideband::Usb,
            DemodMode::Lsb => self.sideband = Sideband::Lsb,
            _ => {}
        }

        self.filter_bandwidth_hz = clamp_filter_bandwidth(self.mode, self.filter_bandwidth_hz);

        self.rebuild_audio_path_for_mode();
        self.reset_audio_state();
    }

    pub fn set_sideband(&mut self, sideband: Sideband) {
        self.sideband = sideband;
        self.reset_audio_state();
    }

    fn deemphasis_tau_for(demod_mode: DemodMode, deemphasis_mode: DeemphasisMode) -> Option<f32> {
        match demod_mode {
            DemodMode::Wfm | DemodMode::Nfm => deemphasis_mode.tau_seconds(),
            _ => None,
        }
    }

    pub fn set_deemphasis_mode(&mut self, mode: DeemphasisMode) {
        info!("pipeline set_deemphasis_mode: {:?}", mode);
        self.deemphasis_mode = mode;
        self.rebuild_audio_path_for_mode();
        self.reset_audio_state();
        info!(
            "pipeline deemphasis after rebuild: mode={:?} tau={:?}",
            self.deemphasis_mode,
            Self::deemphasis_tau_for(self.mode, self.deemphasis_mode)
        );
    }

    pub fn set_target_frequency(&mut self, target_freq_hz: f32) {
        self.tuner.set_target_frequency(target_freq_hz);
    }

    pub fn set_center_frequency(&mut self, center_freq_hz: f32) {
        self.tuner.set_center_frequency(center_freq_hz);
    }

    pub fn process_iq_into(&mut self, input: &[Complex32], output: &mut Vec<Complex32>) {
        self.tuned_iq_scratch.clear();
        self.tuned_iq_scratch
            .reserve(input.len().saturating_sub(self.tuned_iq_scratch.capacity()));

        self.tuner.process_into(input, &mut self.tuned_iq_scratch);
        self.channelizer
            .process_into(&self.tuned_iq_scratch, output);
    }

    pub fn process_iq(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut output = std::mem::take(&mut self.channelized_iq_scratch);
        self.process_iq_into(input, &mut output);
        let result = output.clone();
        self.channelized_iq_scratch = output;
        result
    }

    fn process_selected_ssb_filter(&mut self, iq: &[Complex32]) -> Vec<Complex32> {
        match self.sideband {
            Sideband::Usb => {
                if let Some(fir) = &mut self.ssb_usb_fir {
                    fir.process_into(iq, &mut self.ssb_filtered_scratch);
                    self.ssb_filtered_scratch.clone()
                } else {
                    iq.to_vec()
                }
            }
            Sideband::Lsb => {
                if let Some(fir) = &mut self.ssb_lsb_fir {
                    fir.process_into(iq, &mut self.ssb_filtered_scratch);
                    self.ssb_filtered_scratch.clone()
                } else {
                    iq.to_vec()
                }
            }
        }
    }

    fn demod_ssb(&mut self, iq: &[Complex32], sideband: Sideband) -> Vec<f32> {
        self.sideband = sideband;
        let filtered = self.process_selected_ssb_filter(iq);
        self.ssb_demod.process(&filtered)
    }

    fn process_fm_audio_post(&mut self, audio: &mut [f32], gain: f32) {
        if let Some(fir) = &mut self.audio_fir {
            fir.process_in_place(audio);
        }

        if let Some(deemphasis) = &mut self.deemphasis {
            deemphasis.process_in_place(audio);
        }

        for sample in audio {
            *sample *= gain;
            *sample = sample.tanh();
        }
    }

    pub fn process_audio(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut iq = std::mem::take(&mut self.channelized_iq_scratch);
        self.process_iq_into(input, &mut iq);

        // S-meter: mean channel power over the post-channel-filter, pre-demod IQ.
        // Computed before demod/AGC/NR2/squelch so it is independent of demod
        // mode and of any audio-domain processing.
        if !iq.is_empty() {
            let sum_sq: f32 = iq.iter().map(|z| z.norm_sqr()).sum();
            self.last_channel_power = sum_sq / iq.len() as f32;
        }

        let mut audio = match self.mode {
            DemodMode::Usb => self.demod_ssb(&iq, Sideband::Usb),
            DemodMode::Lsb => self.demod_ssb(&iq, Sideband::Lsb),
            DemodMode::Wfm | DemodMode::Nfm => self.fm_demod.process(&iq),
            DemodMode::Am => self.am_demod.process(&iq),
            // RX: CWU/CWL share the symmetric BFO demod for this phase (real-part
            // output → no upper/lower selectivity yet; documented).  Placement/TX
            // side is mode-driven; RX sounds the same for CWU and CWL.
            DemodMode::Cwu | DemodMode::Cwl => self.cw_demod.process(&iq),
        };

        self.dc_blocker.process_in_place(&mut audio);

        match self.mode {
            DemodMode::Wfm => self.process_fm_audio_post(&mut audio, WFM_AUDIO_GAIN),
            DemodMode::Nfm => self.process_fm_audio_post(&mut audio, NFM_AUDIO_GAIN),
            DemodMode::Am => {
                self.agc.process_in_place(&mut audio);

                if let Some(fir) = &mut self.audio_fir {
                    fir.process_in_place(&mut audio);
                }
            }
            DemodMode::Cwu | DemodMode::Cwl => {
                self.agc.process_in_place(&mut audio);

                if let Some(fir) = &mut self.audio_fir {
                    fir.process_in_place(&mut audio);
                }
            }
            DemodMode::Usb | DemodMode::Lsb => {
                self.agc.process_in_place(&mut audio);

                if let Some(fir) = &mut self.audio_fir {
                    fir.process_in_place(&mut audio);
                }
            }
        }

        self.channelized_iq_scratch = iq;

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
        self.am_demod.reset();
        self.cw_demod.reset();

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

        self.tuned_iq_scratch.clear();
        self.channelized_iq_scratch.clear();
        self.ssb_filtered_scratch.clear();
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

    /// Enable/disable the post-demod AGC (operator radio control).
    pub fn set_agc_enabled(&mut self, enabled: bool) {
        self.agc.set_enabled(enabled);
    }

    /// Set AGC strength in [0, 1] (operator radio control).
    pub fn set_agc_strength(&mut self, strength: f32) {
        self.agc.set_strength(strength);
    }

    /// Most recently applied AGC gain (diagnostics).
    pub fn agc_current_gain(&self) -> f32 {
        self.agc.current_gain()
    }

    /// Current AGC envelope estimate (diagnostics).
    pub fn agc_envelope(&self) -> f32 {
        self.agc.envelope()
    }

    /// Mean channel power (|z|^2, normalized full-scale units) of the most
    /// recent pre-demod IQ block — the S-meter measurement point.
    pub fn last_channel_power(&self) -> f32 {
        self.last_channel_power
    }

    pub fn rebuild_ssb_filters(&mut self, bandwidth_hz: f32, taps: usize) {
        let bandwidth_hz = bandwidth_hz.max(300.0);
        let taps = taps.max(31) | 1;

        self.ssb_bandwidth_hz = bandwidth_hz;
        self.ssb_fir_taps = taps;

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
        info!("pipeline set_ssb_pitch_hz: {}", pitch_hz);
        self.ssb_pitch_hz = pitch_hz;
        self.rebuild_ssb_filters(self.ssb_bandwidth_hz, self.ssb_fir_taps);
        self.reset_audio_state();
    }

    pub fn set_cw_pitch_hz(&mut self, pitch_hz: f32) {
        self.cw_pitch_hz = pitch_hz;

        if matches!(self.mode, DemodMode::Cwu | DemodMode::Cwl) {
            self.cw_demod = CwDemodulator::new(self.output_sample_rate_hz, self.cw_pitch_hz);
        }

        self.reset_audio_state();
    }

    pub fn set_filter_bandwidth_hz(&mut self, bandwidth_hz: f32) {
        self.filter_bandwidth_hz = clamp_filter_bandwidth(self.mode, bandwidth_hz);
        self.rebuild_audio_path_for_mode();
        self.reset_audio_state();
    }

    pub fn filter_bandwidth_hz(&self) -> f32 {
        self.filter_bandwidth_hz
    }
}
