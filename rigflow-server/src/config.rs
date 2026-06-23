use std::env;

use rigflow_core::dsp::modes::DemodMode;

pub const WATERFALL_BINS: usize = 1024;
pub const WATERFALL_FRAME_RATE_HZ: f32 = 20.0;
/// Upper bound for a client-requested waterfall frame rate (Hz). The rate is
/// adjustable at runtime (0 = off) but clamped to this ceiling; also the bound a
/// future UI slider should use.
pub const WATERFALL_FRAME_RATE_MAX_HZ: f32 = 30.0;

// Keep this only if it is still referenced elsewhere.
pub const WATERFALL_EVERY_N_BLOCKS: usize = 1;

#[derive(Debug, Clone)]
pub enum SourceKind {
    Fake,
    Wav,
    RtlSdr,
    HermesLite2,
}

/// Server-wide startup configuration.
///
/// Note that in the newer multi-radio design, some of these values act more as
/// defaults or legacy startup settings than as globally authoritative runtime
/// state.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub demod: DemodMode,

    /// Directory for IQ recordings and WAV-file "radio" discovery.
    pub recordings_dir: String,

    pub fake_sample_rate_hz: f32,
    pub fake_tone_hz: f32,
    pub fake_center_freq_hz: f32,
    pub fake_target_freq_hz: f32,

    pub rtlsdr_sample_rate_hz: u32,
    pub rtlsdr_gain_tenths_db: Option<i32>,
    pub rtlsdr_ppm_correction: i32,
    pub rtlsdr_direct_sampling: bool,

    pub hl2_sample_rate_hz: u32,

    /// Hardrock-50 amplifier serial: `Some("auto")` (the default) auto-detects
    /// (narrow USB-serial ports by VID/PID, probe each with a read-only `HRRX;`,
    /// adopt only one that answers as an HR50, baud auto-scanned); `Some(path)`
    /// or `Some("path:baud")` opens that device directly (default baud 19200);
    /// `None` (`--hr50-serial none`) disables amplifier polling.
    pub hr50_serial: Option<String>,

    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            demod: DemodMode::Lsb,

            recordings_dir: "./".to_string(),

            // Use a high enough fake sample rate so the existing mode-dependent
            // pipeline cutoffs (especially WFM) remain valid without hitting
            // Nyquist assertions. This keeps the fake source compatible with
            // the current RTL-oriented DSP pipeline until cutoffs are derived
            // from sample rate + decimation more robustly.
            fake_sample_rate_hz: 1_024_000.0,
            fake_tone_hz: 1_500.0,
            fake_center_freq_hz: 101_100_000.0,
            fake_target_freq_hz: 101_100_000.0,

            rtlsdr_sample_rate_hz: 2_048_000,
            rtlsdr_gain_tenths_db: None,
            rtlsdr_ppm_correction: 0,
            rtlsdr_direct_sampling: false,

            hl2_sample_rate_hz: 384_000,

            hr50_serial: Some("auto".to_string()),

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
                "--recordings-dir" => {
                    cfg.recordings_dir = next_arg(&mut args, "--recordings-dir")?;
                }

                "--hr50-serial" => {
                    let value = next_arg(&mut args, "--hr50-serial")?;
                    // `none`/empty disables amplifier polling; otherwise the value
                    // is `auto`, `<path>`, or `<path>:baud` (parsed by the worker).
                    cfg.hr50_serial = if value.is_empty() || value.eq_ignore_ascii_case("none") {
                        None
                    } else {
                        Some(value)
                    };
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
  rigflow_server [options]

All sources (RTL-SDR, Hermes Lite 2, WAV recordings, and a built-in fake tone)
are discovered automatically; the client selects a radio from the list.

Options:
  --help, -h               print this help
  --recordings-dir PATH    directory for IQ recordings and WAV-file "radio"
                           discovery (default: ./)
  --hr50-serial VALUE      Hardrock-50 amplifier serial:
                             auto            detect by USB VID/PID + HR50 probe
                                             (default)
                             <path>[:baud]   open that device directly
                                             (e.g. /dev/ttyUSB0 or
                                             /dev/ttyUSB0:19200; default baud 19200)
                             none            disable amplifier polling
"#
        .to_string()
    }
}

pub fn choose_block_size(source: &SourceKind) -> usize {
    match source {
        SourceKind::Fake => 8192,
        SourceKind::Wav => 8192,
        SourceKind::RtlSdr => 16384,
        SourceKind::HermesLite2 => 4096,
    }
}

/// Choose decimation to target ~170 kHz post-decimation bandwidth regardless
/// of input sample rate. Keeps WFM/NFM channel widths consistent across rates.
pub fn choose_decimation(sample_rate_hz: f32) -> usize {
    ((sample_rate_hz / 170_000.0).round() as usize).max(1)
}

fn next_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
