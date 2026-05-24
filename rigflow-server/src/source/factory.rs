use crate::source::fake::FakeIqSource;
use crate::source::hermeslite2::HermesLite2Source;
use crate::source::rtlsdr::RtlSdrSource;
use crate::source::wav::IqWavReader;
use crate::source::IqSource;

/// Configuration used to construct a concrete IQ source.
pub enum SourceConfig {
    WavFile {
        path: String,
    },
    Fake {
        sample_rate_hz: f32,
        tone_hz: f32,
    },
    RtlSdr {
        device_index: usize,
        sample_rate_hz: u32,
        center_freq_hz: u32,
        gain_tenths_db: Option<i32>,
        ppm_correction: i32,
        direct_sampling: bool,
        block_complex_samples: usize,
    },
    HermesLite2 {
        addr: String,
        sample_rate_hz: f32,
        center_freq_hz: f32,
    },
}

/// Create a concrete IQ source from the provided configuration.
pub fn create_source(config: SourceConfig) -> Result<Box<dyn IqSource>, String> {
    match config {
        SourceConfig::WavFile { path } => {
            let source = IqWavReader::open(path)?;
            Ok(Box::new(source))
        }

        SourceConfig::Fake {
            sample_rate_hz,
            tone_hz,
        } => {
            let source = FakeIqSource::new(sample_rate_hz, tone_hz);
            Ok(Box::new(source))
        }

        SourceConfig::RtlSdr {
            device_index,
            sample_rate_hz,
            center_freq_hz,
            gain_tenths_db,
            ppm_correction,
            direct_sampling,
            block_complex_samples,
        } => {
            let source = RtlSdrSource::open(
                device_index,
                sample_rate_hz,
                center_freq_hz,
                gain_tenths_db,
                ppm_correction,
                direct_sampling,
                block_complex_samples,
            )?;

            Ok(Box::new(source))
        }

        SourceConfig::HermesLite2 {
            addr,
            sample_rate_hz,
            center_freq_hz,
        } => {
            let source = HermesLite2Source::open(&addr, sample_rate_hz, center_freq_hz)?;
            Ok(Box::new(source))
        }
    }
}
