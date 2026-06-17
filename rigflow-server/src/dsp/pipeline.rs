use std::f32::consts::PI;

use log::debug;
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
        debug!(
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
            DemodMode::Usb | DemodMode::Lsb | DemodMode::DgtU => {
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
            DemodMode::Usb | DemodMode::DgtU => self.sideband = Sideband::Usb,
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
        debug!("pipeline set_deemphasis_mode: {:?}", mode);
        self.deemphasis_mode = mode;
        self.rebuild_audio_path_for_mode();
        self.reset_audio_state();
        debug!(
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
            DemodMode::Usb | DemodMode::DgtU => self.demod_ssb(&iq, Sideband::Usb),
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
            DemodMode::DgtU => {
                // Digital (FT8) RX must be flat / fixed-gain: WSJT-X relies on the
                // relative levels of signals in the passband, and an AGC pumping
                // on a strong in-band signal amplitude-modulates the weak target
                // and corrupts that information.  Skip AGC entirely for DgtU; the
                // band-pass FIR still applies.
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
        debug!("pipeline set_ssb_pitch_hz: {}", pitch_hz);
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

#[cfg(test)]
mod ssb_tx_dsp_tests {
    //! Software measurement of the **transmit** SSB DSP — the part of the signal
    //! that is *ours*, not the HL2's. Sideband/carrier suppression and two-tone
    //! IMD are properties of the IQ our modulator generates (the HL2 just
    //! upconverts it), so they are measurable here precisely and repeatably,
    //! without the bench / the original TinySA's 3 kHz RBW limit.
    //!
    //! The chain mirrors `hermeslite2::tx_ssb_mic`:
    //! `DcBlocker → [SpeechCompressor] → [TxLimiter] → ×0.9 → ComplexSidebandFir`.

    use super::ComplexSidebandFir;
    use crate::dsp::audio::dc_blocker::DcBlocker;
    use crate::dsp::audio::speech_compressor::{ratio_for_level, SpeechCompressor};
    use crate::dsp::audio::tx_limiter::TxLimiter;
    use crate::dsp::demod::Sideband;
    use num_complex::Complex32;

    // Mirror of the tx_ssb_mic constants.
    const FS: f32 = 48_000.0;
    const AUDIO_BW_HZ: f32 = 2400.0;
    const AUDIO_PITCH_HZ: f32 = 300.0;
    const FIR_TAPS: usize = 127;
    const TX_AUDIO_SCALE: f32 = 0.9;

    /// Run audio through the real TX modulator chain → baseband IQ.
    fn modulate(
        audio: &[f32],
        usb: bool,
        compressor_level: Option<u8>,
        limiter_threshold: Option<f32>,
    ) -> Vec<Complex32> {
        let mut buf = audio.to_vec();
        DcBlocker::new(0.995).process_in_place(&mut buf);
        if let Some(level) = compressor_level {
            SpeechCompressor::new(FS, ratio_for_level(level), 10.0, 150.0)
                .process_in_place(&mut buf);
        }
        if let Some(th) = limiter_threshold {
            TxLimiter::new(FS, th, 2.0, 120.0).process_in_place(&mut buf);
        }
        let cin: Vec<Complex32> = buf
            .iter()
            .map(|&s| Complex32::new(s * TX_AUDIO_SCALE, 0.0))
            .collect();
        let mut fir = ComplexSidebandFir::new(
            FS,
            AUDIO_BW_HZ,
            AUDIO_PITCH_HZ,
            FIR_TAPS,
            if usb { Sideband::Usb } else { Sideband::Lsb },
        );
        let mut out = Vec::new();
        fir.process_into(&cin, &mut out);
        out
    }

    /// Hann-windowed, DC-removed single-bin DFT magnitude (complex-tone amplitude)
    /// at `f_hz` — same method as the on-air `log_tx_tone_rx_sideband` analyzer.
    fn bin_mag(samples: &[Complex32], f_hz: f64) -> f64 {
        use std::f64::consts::TAU;
        let n = samples.len();
        let mean_re = samples.iter().map(|s| s.re as f64).sum::<f64>() / n as f64;
        let mean_im = samples.iter().map(|s| s.im as f64).sum::<f64>() / n as f64;
        let win = |k: usize| 0.5 - 0.5 * (TAU * k as f64 / (n - 1).max(1) as f64).cos();
        let win_sum: f64 = (0..n).map(win).sum();
        let w = -TAU * f_hz / FS as f64;
        let (mut ar, mut ai) = (0.0f64, 0.0f64);
        for (k, s) in samples.iter().enumerate() {
            let re = (s.re as f64 - mean_re) * win(k);
            let im = (s.im as f64 - mean_im) * win(k);
            let (sinp, cosp) = (w * k as f64).sin_cos();
            ar += re * cosp - im * sinp;
            ai += re * sinp + im * cosp;
        }
        (ar * ar + ai * ai).sqrt() / win_sum
    }

    /// |DC| of the baseband IQ = the carrier (centre-spike) leakage.
    fn dc_mag(samples: &[Complex32]) -> f64 {
        let n = samples.len() as f64;
        let re = samples.iter().map(|s| s.re as f64).sum::<f64>() / n;
        let im = samples.iter().map(|s| s.im as f64).sum::<f64>() / n;
        (re * re + im * im).sqrt()
    }

    fn db(ratio: f64) -> f64 {
        20.0 * ratio.max(1e-15).log10()
    }

    fn tone(freq_hz: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|k| amp * (std::f32::consts::TAU * freq_hz * k as f32 / FS).sin())
            .collect()
    }

    const N: usize = 16_384;
    const SKIP: usize = 512; // discard FIR fill transient

    /// The modulator must place the tone on the wanted sideband and suppress the
    /// opposite sideband (image) and the carrier (DC) — this is pure Rigflow DSP.
    #[test]
    fn ssb_modulator_suppresses_image_and_carrier() {
        for usb in [true, false] {
            let f = 1500.0_f64; // mid-passband audio tone
            let audio = tone(f as f32, 0.5, N);
            let iq = modulate(&audio, usb, None, None);
            let s = &iq[SKIP..];

            // USB → energy at +f (above carrier); LSB → at −f.
            let (wanted, image) = if usb {
                (bin_mag(s, f), bin_mag(s, -f))
            } else {
                (bin_mag(s, -f), bin_mag(s, f))
            };
            let image_supp = db(wanted / image);
            let carrier_supp = db(wanted / dc_mag(s));

            let sb = if usb { "USB" } else { "LSB" };
            // Measured ≈ 74 dB image / 99 dB carrier; guard a generous floor.
            assert!(
                image_supp > 60.0,
                "{sb}: opposite-sideband suppression {image_supp:.1} dB (want > 60)"
            );
            assert!(
                carrier_supp > 80.0,
                "{sb}: carrier suppression {carrier_supp:.1} dB (want > 80)"
            );
            println!("{sb}: image {image_supp:.1} dB, carrier {carrier_supp:.1} dB");
        }
    }

    /// With no compression/limiting, the modulator is linear, so a two-tone input
    /// must produce **no** IMD3/IMD5 products — i.e. our SSB generation injects no
    /// intermod of its own. (Tones 700/1900 Hz → products at ∓500/+3100 (IMD3),
    /// ∓1700/+4300 (IMD5), USB convention.)
    #[test]
    fn ssb_modulator_two_tone_is_linear() {
        let a = tone(700.0, 0.4, N);
        let b = tone(1900.0, 0.4, N);
        let audio: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
        let iq = modulate(&audio, true, None, None);
        let s = &iq[SKIP..];

        let tone_ref = bin_mag(s, 700.0).max(bin_mag(s, 1900.0));
        let imd3 = bin_mag(s, -500.0).max(bin_mag(s, 3100.0));
        let imd5 = bin_mag(s, -1700.0).max(bin_mag(s, 4300.0));
        let imd3_dbc = db(imd3 / tone_ref);
        let imd5_dbc = db(imd5 / tone_ref);

        println!("linear path: IMD3 {imd3_dbc:.1} dBc, IMD5 {imd5_dbc:.1} dBc");
        // Measured ≈ −157/−173 dBc (numerical floor); guard a generous bound.
        assert!(
            imd3_dbc < -100.0,
            "linear modulator should add no IMD3, got {imd3_dbc:.1} dBc"
        );
        assert!(imd5_dbc < -100.0, "IMD5 {imd5_dbc:.1} dBc (want < -100)");
    }

    /// Full chain (compressor + limiter ON) at a **normal** drive level — peaks
    /// below the limiter threshold — must still be clean: the slow compressor/
    /// limiter can't track the 1.2 kHz two-tone beat, so they add negligible IMD.
    #[test]
    fn tx_chain_two_tone_clean_at_normal_level() {
        // Each tone 0.3 → envelope peak 0.6, ×0.9 = 0.54, below the 0.9 limiter
        // threshold → no hard limiting.
        let a = tone(700.0, 0.3, N);
        let b = tone(1900.0, 0.3, N);
        let audio: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
        let iq = modulate(&audio, true, Some(3), Some(0.9));
        let s = &iq[SKIP..];

        let tone_ref = bin_mag(s, 700.0).max(bin_mag(s, 1900.0));
        let imd3_dbc = db(bin_mag(s, -500.0).max(bin_mag(s, 3100.0)) / tone_ref);

        println!("processed (normal level): IMD3 {imd3_dbc:.1} dBc");
        // Measured ≈ −85 dBc; guard a generous bound.
        assert!(
            imd3_dbc < -60.0,
            "TX processing at normal level should stay clean, IMD3 {imd3_dbc:.1} dBc (want < -60)"
        );
    }

    // ── §E — CW key-click envelope ──────────────────────────────────────────

    /// Rectangular single-bin DFT magnitude (no window) — correct for the
    /// zero-padded, self-tapering CW element below.
    fn dft_mag(samples: &[Complex32], f_hz: f64) -> f64 {
        use std::f64::consts::TAU;
        let w = -TAU * f_hz / FS as f64;
        let (mut ar, mut ai) = (0.0f64, 0.0f64);
        for (k, s) in samples.iter().enumerate() {
            let (sinp, cosp) = (w * k as f64).sin_cos();
            ar += s.re as f64 * cosp - s.im as f64 * sinp;
            ai += s.re as f64 * sinp + s.im as f64 * cosp;
        }
        (ar * ar + ai * ai).sqrt() / samples.len() as f64
    }

    /// One keyed CW element as baseband IQ: raised-cosine rise/fall of `ramp`
    /// samples around a `sustain`-sample key-down, complex carrier at `pitch` Hz,
    /// zero-padded (centred) in `n`. `hard=true` skips shaping (instant on/off) to
    /// model the key-click failure case. Mirrors `hermeslite2::send_tx_cw_packet`
    /// (env = 0.5·(1−cos(π·level)), ENV_MS = 8 ms).
    fn cw_element(ramp: usize, sustain: usize, pitch: f64, hard: bool, n: usize) -> Vec<Complex32> {
        use std::f64::consts::{PI, TAU};
        let total = ramp * 2 + sustain;
        let start = (n - total) / 2;
        let mut out = vec![Complex32::new(0.0, 0.0); n];
        for i in 0..total {
            let level = if i < ramp {
                (i + 1) as f64 / ramp as f64
            } else if i < ramp + sustain {
                1.0
            } else {
                1.0 - (i - ramp - sustain + 1) as f64 / ramp as f64
            };
            let env = if hard {
                1.0
            } else {
                0.5 * (1.0 - (PI * level).cos())
            };
            let k = start + i;
            let phase = TAU * pitch * k as f64 / FS as f64;
            out[k] = Complex32::new((env * phase.cos()) as f32, (env * phase.sin()) as f32);
        }
        out
    }

    /// CW keying must be band-limited: the 8 ms raised-cosine rise/fall keeps the
    /// keying sidebands far down (no key clicks), and must clearly beat hard
    /// (rectangular) keying — a regression toward unshaped keying would splatter.
    #[test]
    fn cw_keying_limits_click_bandwidth() {
        const RAMP: usize = 384; // 8 ms @ 48 kHz (ENV_MS)
        const SUSTAIN: usize = 1440; // 30 ms key-down
        const NCW: usize = 32_768;
        let pitch = 600.0;

        // Worst keying sideband anywhere in the "click region" (≥ ±500 Hz from the
        // carrier). Swept finely so the result doesn't depend on where the
        // rectangle's sinc nulls happen to fall.
        let worst_sideband = |hard: bool| -> f64 {
            let s = cw_element(RAMP, SUSTAIN, pitch, hard, NCW);
            let carrier = dft_mag(&s, pitch);
            let mut worst = 0.0f64;
            let mut off = 500.0;
            while off <= 3000.0 {
                worst = worst.max(dft_mag(&s, pitch + off));
                worst = worst.max(dft_mag(&s, pitch - off));
                off += 10.0;
            }
            db(worst / carrier)
        };

        let shaped = worst_sideband(false);
        let hard = worst_sideband(true);
        println!("CW key-click sidebands (≥ ±500 Hz): shaped {shaped:.1} dBc, hard {hard:.1} dBc");
        assert!(
            shaped < -50.0,
            "shaped CW keying sidebands {shaped:.1} dBc (want < -50)"
        );
        assert!(
            shaped < hard - 20.0,
            "raised-cosine should beat hard keying by > 20 dB (shaped {shaped:.1}, hard {hard:.1})"
        );
    }

    // ── §F — digital (FT8) occupied bandwidth / splatter ────────────────────

    /// A constant-envelope FSK (FT8-like) through the real modulator must not gain
    /// far-out-of-band splatter — i.e. our digital TX path is transparent. Close-in
    /// spectrum is the signal's own (WSJT-X's) business; we check the far offsets
    /// that only *our* path could fill.
    #[test]
    fn digital_fsk_passes_without_splatter() {
        use std::f64::consts::TAU;
        const SPACING: f64 = 6.25; // FT8 tone spacing
        const SYM: usize = (0.16 * FS as f64) as usize; // 0.16 s symbol
        const BASE: f64 = 1500.0; // mid SSB passband
        let symbols = [3u32, 6, 1, 7, 0, 4, 2, 5, 1, 6, 3, 0, 7, 2, 5, 4];

        // Continuous-phase 8-FSK audio.
        let mut audio = Vec::with_capacity(symbols.len() * SYM);
        let mut phase = 0.0f64;
        for &sym in &symbols {
            let dphi = TAU * (BASE + sym as f64 * SPACING) / FS as f64;
            for _ in 0..SYM {
                audio.push(0.4 * phase.sin() as f32);
                phase += dphi;
            }
        }
        // Digital path: real modulator, no compression/limiting.
        let iq = modulate(&audio, true, None, None);
        let s = &iq[SKIP..];

        let centre = BASE + 3.5 * SPACING; // ≈ 1522 Hz, middle of the ~50 Hz band
        let in_band = (0..8)
            .map(|t| bin_mag(s, BASE + t as f64 * SPACING))
            .fold(0.0f64, f64::max);
        // Far offsets only (well beyond the ~50 Hz FT8 band) — added splatter.
        let mut splatter = 0.0f64;
        for off in [500.0, 900.0, 1500.0, 2500.0] {
            splatter = splatter.max(bin_mag(s, centre + off));
            splatter = splatter.max(bin_mag(s, centre - off));
        }
        let dbc = db(splatter / in_band);
        println!("digital FSK far-offset splatter (≥ ±500 Hz): {dbc:.1} dBc");
        assert!(
            dbc < -45.0,
            "digital path splatter {dbc:.1} dBc (want < -45)"
        );
    }
}
