use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

pub struct AudioWavWriter {
    writer: WavWriter<BufWriter<File>>,
}

impl AudioWavWriter {
    pub fn create<P: AsRef<Path>>(path: P, sample_rate_hz: u32) -> hound::Result<Self> {
        let spec = WavSpec {
            channels: 1,
            sample_rate: sample_rate_hz,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        let writer = WavWriter::create(path, spec)?;
        Ok(Self { writer })
    }

    pub fn write_samples(&mut self, samples: &[f32]) -> hound::Result<()> {
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let s = (clamped * i16::MAX as f32) as i16;
            self.writer.write_sample(s)?;
        }
        Ok(())
    }

    pub fn finalize(self) -> hound::Result<()> {
        self.writer.finalize()
    }
}
