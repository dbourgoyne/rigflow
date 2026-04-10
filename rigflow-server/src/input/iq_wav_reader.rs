use hound::{SampleFormat, WavReader};
use num_complex::Complex32;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use crate::source::wav_metadata::parse_center_freq_hz_from_filename;

pub struct IqWavReader {
    reader: WavReader<BufReader<File>>,
    sample_format: SampleFormat,
    bits_per_sample: u16,
    channels: u16,
    sample_rate: u32,
    center_freq_hz: Option<u64>,
}

impl IqWavReader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
	let path_ref = path.as_ref();

	let reader = WavReader::open(path_ref)
            .map_err(|e| format!("failed to open wav: {e}"))?;
	let spec = reader.spec();

	if spec.channels != 2 {
            return Err(format!(
		"expected stereo IQ WAV (2 channels), found {} channels",
		spec.channels
            ));
	}

	let center_freq_hz = parse_center_freq_hz_from_filename(path_ref);

	Ok(Self {
            reader,
            sample_format: spec.sample_format,
            bits_per_sample: spec.bits_per_sample,
            channels: spec.channels,
            sample_rate: spec.sample_rate,
            center_freq_hz, // <-- store it
	})
    }

    pub fn center_frequency_hz(&self) -> Option<u64> {
	self.center_freq_hz
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    pub fn read_block(&mut self, max_complex_samples: usize) -> Result<Vec<Complex32>, String> {
        match (self.sample_format, self.bits_per_sample) {
            (SampleFormat::Int, 16) => self.read_block_i16(max_complex_samples),
            (SampleFormat::Float, 32) => self.read_block_f32(max_complex_samples),
            _ => Err(format!(
                "unsupported WAV format: {:?} {}-bit",
                self.sample_format, self.bits_per_sample
            )),
        }
    }

    fn read_block_i16(&mut self, max_complex_samples: usize) -> Result<Vec<Complex32>, String> {
        let mut out = Vec::with_capacity(max_complex_samples);
        let mut samples = self.reader.samples::<i16>();

        for _ in 0..max_complex_samples {
            let i = match samples.next() {
                Some(Ok(v)) => v,
                Some(Err(e)) => return Err(format!("error reading I sample: {e}")),
                None => break,
            };

            let q = match samples.next() {
                Some(Ok(v)) => v,
                Some(Err(e)) => return Err(format!("error reading Q sample: {e}")),
                None => break,
            };

            let i_f = i as f32 / i16::MAX as f32;
            let q_f = q as f32 / i16::MAX as f32;

            out.push(Complex32::new(i_f, q_f));
        }

        Ok(out)
    }

    fn read_block_f32(&mut self, max_complex_samples: usize) -> Result<Vec<Complex32>, String> {
        let mut out = Vec::with_capacity(max_complex_samples);
        let mut samples = self.reader.samples::<f32>();

        for _ in 0..max_complex_samples {
            let i = match samples.next() {
                Some(Ok(v)) => v,
                Some(Err(e)) => return Err(format!("error reading I sample: {e}")),
                None => break,
            };

            let q = match samples.next() {
                Some(Ok(v)) => v,
                Some(Err(e)) => return Err(format!("error reading Q sample: {e}")),
                None => break,
            };

            out.push(Complex32::new(i, q));
        }

        Ok(out)
    }
}

impl crate::source::IqSource for IqWavReader {
    fn sample_rate(&self) -> f32 {
        self.sample_rate as f32
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        IqWavReader::read_block(self, max_samples)
    }

    fn set_center_frequency(&mut self, _center_freq_hz: f32) -> Result<(), String> {
        Ok(())
    }

    fn is_realtime(&self) -> bool {
        false
    }
}

