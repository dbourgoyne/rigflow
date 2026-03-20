use crate::input::iq_wav_reader::IqWavReader;
use crate::source::fake_iq::FakeIqSource;
use crate::source::IqSource;

pub enum SourceConfig {
    WavFile { path: String },
    Fake { sample_rate_hz: f32, tone_hz: f32 },
}

pub fn create_source(config: SourceConfig) -> Result<Box<dyn IqSource>, String> {
    match config {
        SourceConfig::WavFile { path } => Ok(Box::new(IqWavReader::open(path)?)),
        SourceConfig::Fake {
            sample_rate_hz,
            tone_hz,
        } => Ok(Box::new(FakeIqSource::new(sample_rate_hz, tone_hz))),
    }
}
