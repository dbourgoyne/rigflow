use rigflow_core::dsp::demod::{DemodMode, Sideband};

use crate::dsp::pipeline::DspPipeline;

#[derive(Debug, Clone, Copy)]
pub struct PipelineSettings {
    pub channel_cutoff_hz: f32,
    pub fir_taps: usize,
    pub audio_cutoff_hz: f32,
    pub audio_fir_taps: usize,
}

pub fn pipeline_settings_for_mode(mode: DemodMode) -> PipelineSettings {
    match mode {
        DemodMode::Wfm => PipelineSettings {
            channel_cutoff_hz: 100_000.0,
            fir_taps: 129,
            audio_cutoff_hz: 15_000.0,
            audio_fir_taps: 101,
        },
        DemodMode::Usb | DemodMode::Lsb => PipelineSettings {
            channel_cutoff_hz: 2_800.0,
            fir_taps: 129,
            audio_cutoff_hz: 2_700.0,
            audio_fir_taps: 101,
        },
    }
}

pub fn build_pipeline(
    center_freq_hz: f32,
    target_freq_hz: f32,
    input_sample_rate_hz: f32,
    decimation_factor: usize,
    mode: DemodMode,
) -> DspPipeline {
    let settings = pipeline_settings_for_mode(mode);

    let mut pipeline = DspPipeline::new(
        center_freq_hz,
        target_freq_hz,
        input_sample_rate_hz,
        settings.channel_cutoff_hz,
        settings.fir_taps,
        decimation_factor,
        settings.audio_cutoff_hz,
        settings.audio_fir_taps,
        48_000.0,
        mode,
    );

    match mode {
        DemodMode::Usb => pipeline.set_sideband(Sideband::Usb),
        DemodMode::Lsb => pipeline.set_sideband(Sideband::Lsb),
        DemodMode::Wfm => {}
    }

    pipeline
}

pub fn mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Usb => "usb".to_string(),
        DemodMode::Lsb => "lsb".to_string(),
        DemodMode::Wfm => "wfm".to_string(),
    }
}

pub fn sideband_to_string(sideband: Sideband) -> String {
    match sideband {
        Sideband::Usb => "usb".to_string(),
        Sideband::Lsb => "lsb".to_string(),
    }
}
