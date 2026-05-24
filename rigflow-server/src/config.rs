use std::env;

use rigflow_core::dsp::modes::DemodMode;

use crate::source::factory::SourceConfig;

pub const WATERFALL_BINS: usize = 1024;
pub const WATERFALL_FRAME_RATE_HZ: f32 = 20.0;

// Keep this only if it is still referenced elsewhere.
pub const WATERFALL_EVERY_N_BLOCKS: usize = 1;

#[derive(Debug, Clone)]
pub enum SourceKind {
    Fake,
    Wav,
    RtlSdr,
}

/// Server-wide startup configuration.
///
/// Note that in the newer multi-radio design, some of these values act more as
/// defaults or legacy startup settings than as globally authoritative runtime
/// state.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub demod: DemodMode,
    pub source: SourceKind,

    pub wav_file: String,
    pub wav_dir: String,

    pub fake_sample_rate_hz: f32,
    pub fake_tone_hz: f32,
    pub fake_center_freq_hz: f32,
    pub fake_target_freq_hz: f32,

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
            wav_dir: "./".to_string(),

	    // Use a high enough fake sample rate so the existing mode-dependent
	    // pipeline cutoffs (especially WFM) remain valid without hitting
	    // Nyquist assertions. This keeps the fake source compatible with
	    // the current RTL-oriented DSP pipeline until cutoffs are derived
	    // from sample rate + decimation more robustly.
            fake_sample_rate_hz: 1_024_000.0,
            fake_tone_hz: 1_500.0,
            fake_center_freq_hz: 101_100_000.0,
            fake_target_freq_hz: 101_100_000.0,

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
    /// Parses simple command-line flags into a ServerConfig.
    ///
    /// This keeps the current hand-rolled CLI behavior intact rather than
    /// introducing a parser crate during refactor.
    pub fn from_args() -> Result<Self, String> {
        let mut cfg = Self::default();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demod" => {
                    let value = next_arg(&mut args, "--demod")?;
		    cfg.demod = value.parse::<DemodMode>()
			.map_err(|_| format!("invalid demod mode: {value}"))?;
                }

                "--wav-dir" => {
                    cfg.wav_dir = next_arg(&mut args, "--wav-dir")?;
                }

                "--source" => {
                    let value = next_arg(&mut args, "--source")?;
                    cfg.source = parse_source_kind(&value)
                        .ok_or_else(|| format!("unknown source '{value}'\n\n{}", Self::usage()))?;
                }

                "--wav-file" => {
                    cfg.wav_file = next_arg(&mut args, "--wav-file")?;
                }

                "--fake-sample-rate" => {
                    cfg.fake_sample_rate_hz =
                        parse_next_arg(&mut args, "--fake-sample-rate", "invalid --fake-sample-rate")?;
                }

                "--fake-tone" => {
                    cfg.fake_tone_hz =
                        parse_next_arg(&mut args, "--fake-tone", "invalid --fake-tone")?;
                }

                "--rtl-device" => {
                    cfg.rtlsdr_device_index =
                        parse_next_arg(&mut args, "--rtl-device", "invalid --rtl-device")?;
                }

                "--rtl-sample-rate" => {
                    cfg.rtlsdr_sample_rate_hz =
                        parse_next_arg(&mut args, "--rtl-sample-rate", "invalid --rtl-sample-rate")?;
                }

                "--rtl-gain" => {
                    cfg.rtlsdr_gain_tenths_db = Some(parse_next_arg(
                        &mut args,
                        "--rtl-gain",
                        "invalid --rtl-gain",
                    )?);
                }

                "--rtl-auto-gain" => {
                    cfg.rtlsdr_gain_tenths_db = None;
                }

                "--rtl-ppm" => {
                    cfg.rtlsdr_ppm_correction =
                        parse_next_arg(&mut args, "--rtl-ppm", "invalid --rtl-ppm")?;
                }

                "--rtl-direct-sampling" => {
                    cfg.rtlsdr_direct_sampling = true;
                }

                "--center" => {
                    cfg.center_freq_hz =
                        parse_next_arg(&mut args, "--center", "invalid --center")?;
                }

                "--target" => {
                    cfg.target_freq_hz =
                        parse_next_arg(&mut args, "--target", "invalid --target")?;
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
  --wav-dir PATH

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

/// Builds a source config from the legacy server config.
///
/// Keep this only if something still calls it. In the newer radio worker path,
/// source creation may already happen elsewhere.
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
        SourceKind::RtlSdr => 16384,
    }
}

/// Choose decimation to target ~170 kHz post-decimation bandwidth regardless
/// of input sample rate. Keeps WFM/NFM channel widths consistent across rates.
pub fn choose_decimation(sample_rate_hz: f32) -> usize {
    ((sample_rate_hz / 170_000.0).round() as usize).max(1)
}

fn next_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_next_arg<T>(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
    parse_err: &str,
) -> Result<T, String>
where
    T: std::str::FromStr,
{
    next_arg(args, flag)?
        .parse()
        .map_err(|_| parse_err.to_string())
}

fn parse_source_kind(value: &str) -> Option<SourceKind> {
    match value {
        "fake" => Some(SourceKind::Fake),
        "wav" => Some(SourceKind::Wav),
        "rtlsdr" => Some(SourceKind::RtlSdr),
        _ => None,
    }
}
