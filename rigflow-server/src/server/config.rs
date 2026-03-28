use std::env;
use rigflow_core::dsp::demod::DemodMode;
use crate::source::factory::SourceConfig;

pub const WATERFALL_BINS: usize = 1024;
pub const WATERFALL_FRAME_RATE_HZ: f32 = 100.0;
pub const WATERFALL_EVERY_N_BLOCKS: usize = 1;

#[derive(Debug, Clone)]
pub enum SourceKind {
    Fake,
    Wav,
    RtlSdr,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub demod: DemodMode,
    pub source: SourceKind,

    pub wav_file: String,

    pub fake_sample_rate_hz: f32,
    pub fake_tone_hz: f32,

    pub rtlsdr_device_index: usize,
    pub rtlsdr_sample_rate_hz: u32,
    pub rtlsdr_gain_tenths_db: Option<i32>,
    pub rtlsdr_ppm_correction: i32,
    pub rtlsdr_direct_sampling: bool,

    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            demod: DemodMode::Lsb,

            source: SourceKind::Fake,

            wav_file: "input_iq.wav".to_string(),

            fake_sample_rate_hz: 48_000.0,
            fake_tone_hz: 1_500.0,

            rtlsdr_device_index: 0,
            rtlsdr_sample_rate_hz: 2_048_000,
            rtlsdr_gain_tenths_db: None,
            rtlsdr_ppm_correction: 0,
            rtlsdr_direct_sampling: false,

            center_freq_hz: 101_100_000.0,
            target_freq_hz: 101_100_000.0,
        }
    }
}

impl ServerConfig {
    pub fn from_args() -> Result<Self, String> {
        let mut cfg = Self::default();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demod" => {
                    let value = args.next().ok_or("--demod requires a value")?;
                    cfg.demod = match value.as_str() {
                        "usb" => DemodMode::Usb,
                        "lsb" => DemodMode::Lsb,
                        "wfm" => DemodMode::Wfm,
                        _ => return Err(format!("unknown demod '{value}'\n\n{}", Self::usage())),
                    };
                }

                "--source" => {
                    let value = args.next().ok_or("--source requires a value")?;
                    cfg.source = match value.as_str() {
                        "fake" => SourceKind::Fake,
                        "wav" => SourceKind::Wav,
                        "rtlsdr" => SourceKind::RtlSdr,
                        _ => return Err(format!("unknown source '{value}'\n\n{}", Self::usage())),
                    };
                }

                "--wav-file" => {
                    cfg.wav_file = args.next().ok_or("--wav-file requires a value")?;
                }

                "--fake-sample-rate" => {
                    cfg.fake_sample_rate_hz = args
                        .next()
                        .ok_or("--fake-sample-rate requires a value")?
                        .parse()
                        .map_err(|_| "invalid --fake-sample-rate".to_string())?;
                }

                "--fake-tone" => {
                    cfg.fake_tone_hz = args
                        .next()
                        .ok_or("--fake-tone requires a value")?
                        .parse()
                        .map_err(|_| "invalid --fake-tone".to_string())?;
                }

                "--rtl-device" => {
                    cfg.rtlsdr_device_index = args
                        .next()
                        .ok_or("--rtl-device requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-device".to_string())?;
                }

                "--rtl-sample-rate" => {
                    cfg.rtlsdr_sample_rate_hz = args
                        .next()
                        .ok_or("--rtl-sample-rate requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-sample-rate".to_string())?;
                }

                "--rtl-gain" => {
                    cfg.rtlsdr_gain_tenths_db = Some(
                        args.next()
                            .ok_or("--rtl-gain requires a value")?
                            .parse()
                            .map_err(|_| "invalid --rtl-gain".to_string())?,
                    );
                }

                "--rtl-auto-gain" => {
                    cfg.rtlsdr_gain_tenths_db = None;
                }

                "--rtl-ppm" => {
                    cfg.rtlsdr_ppm_correction = args
                        .next()
                        .ok_or("--rtl-ppm requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-ppm".to_string())?;
                }

                "--rtl-direct-sampling" => {
                    cfg.rtlsdr_direct_sampling = true;
                }

                "--center" => {
                    cfg.center_freq_hz = args
                        .next()
                        .ok_or("--center requires a value")?
                        .parse()
                        .map_err(|_| "invalid --center".to_string())?;
                }

                "--target" => {
                    cfg.target_freq_hz = args
                        .next()
                        .ok_or("--target requires a value")?
                        .parse()
                        .map_err(|_| "invalid --target".to_string())?;
                }

                "--help" | "-h" => {
                    return Err(Self::usage());
                }

                other => {
                    return Err(format!("unknown argument '{other}'\n\n{}", Self::usage()));
                }
            }
        }

        Ok(cfg)
    }

    pub fn usage() -> String {
        r#"Usage:
  rigflow_server --source fake [options]
  rigflow_server --source wav --wav-file input_iq.wav [options]
  rigflow_server --source rtlsdr [options]

Common options:
  --center HZ
  --target HZ
  --demod "wfm|lsb|usb"

Fake source:
  --fake-sample-rate HZ
  --fake-tone HZ

WAV source:
  --wav-file PATH

RTL-SDR source:
  --rtl-device INDEX
  --rtl-sample-rate HZ
  --rtl-gain TENTHS_DB
  --rtl-auto-gain
  --rtl-ppm PPM
  --rtl-direct-sampling
"#
        .to_string()
    }
}

pub fn make_source_config(cfg: &ServerConfig, block_size: usize) -> SourceConfig {
    match cfg.source {
        SourceKind::Fake => SourceConfig::Fake {
            sample_rate_hz: cfg.fake_sample_rate_hz,
            tone_hz: cfg.fake_tone_hz,
        },
        SourceKind::Wav => SourceConfig::WavFile {
            path: cfg.wav_file.clone(),
        },
        SourceKind::RtlSdr => SourceConfig::RtlSdr {
            device_index: cfg.rtlsdr_device_index,
            sample_rate_hz: cfg.rtlsdr_sample_rate_hz,
            center_freq_hz: cfg.center_freq_hz as u32,
            gain_tenths_db: cfg.rtlsdr_gain_tenths_db,
            ppm_correction: cfg.rtlsdr_ppm_correction,
            direct_sampling: cfg.rtlsdr_direct_sampling,
            block_complex_samples: block_size,
        },
    }
}

pub fn choose_block_size(source: &SourceKind) -> usize {
    match source {
        SourceKind::Fake => 8192,
        SourceKind::Wav => 8192,
        SourceKind::RtlSdr => 65536,
    }
}

pub fn choose_decimation(source: &SourceKind) -> usize {
    match source {
        SourceKind::Fake => 4,
        SourceKind::Wav => 16,
        SourceKind::RtlSdr => 12, //8,
    }
}
