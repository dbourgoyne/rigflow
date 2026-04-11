use num_complex::Complex32;
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

/// Generates waterfall rows from IQ samples using FFT.
///
/// Pipeline:
/// 1. Apply window (Hann)
/// 2. FFT
/// 3. Magnitude spectrum
/// 4. FFT shift (center DC)
/// 5. Convert to dB
/// 6. Normalize to 0–255 grayscale
///
/// Output:
/// - One row of `u8` intensity values (0–255)
pub struct WaterfallGenerator {
    fft_size: usize,

    /// FFT plan (cached for performance)
    fft: Arc<dyn Fft<f32>>,

    /// Reusable FFT buffer (complex samples)
    buffer: Vec<Complex<f32>>,

    /// Window function (Hann)
    window: Vec<f32>,
}

impl WaterfallGenerator {
    /// Create a new waterfall generator.
    ///
    /// `fft_size` determines:
    /// - frequency resolution
    /// - output row width
    pub fn new(fft_size: usize) -> Self {
        assert!(fft_size > 0, "fft_size must be > 0");

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Precompute Hann window:
        // w[n] = 0.5 - 0.5 cos(2πn / (N-1))
        let window = (0..fft_size)
            .map(|i| {
                0.5
                    - 0.5
                        * (2.0 * std::f32::consts::PI * i as f32
                            / (fft_size as f32 - 1.0))
                            .cos()
            })
            .collect();

        Self {
            fft_size,
            fft,
            buffer: vec![Complex::new(0.0, 0.0); fft_size],
            window,
        }
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Generate a single waterfall row from IQ samples.
    ///
    /// Steps:
    /// - window + copy into FFT buffer
    /// - FFT
    /// - magnitude + fftshift
    /// - dB conversion
    /// - normalize to 0–255
    pub fn generate_row(&mut self, iq: &[Complex32]) -> Vec<u8> {
        let n = self.fft_size.min(iq.len());
        if n == 0 {
            return Vec::new();
        }

        // Clear buffer (important if iq.len() < fft_size)
        for v in &mut self.buffer {
            *v = Complex::new(0.0, 0.0);
        }

        // Copy + apply window
        for ((buf, iq_sample), &w) in self
            .buffer
            .iter_mut()
            .zip(iq.iter())
            .zip(self.window.iter())
        {
            buf.re = iq_sample.re * w;
            buf.im = iq_sample.im * w;
        }

        // Perform FFT in-place
        self.fft.process(&mut self.buffer);

        // Compute magnitude spectrum with FFT shift
        let mut mags = vec![0.0_f32; self.fft_size];
        let half = self.fft_size / 2;

        for (i, mag) in mags.iter_mut().enumerate() {
            let src = (i + half) % self.fft_size;
            *mag = self.buffer[src].norm();
        }

        // Convert to dB scale
        let eps = 1e-12_f32;
        let db: Vec<f32> = mags
            .iter()
            .map(|&x| 20.0 * (x + eps).log10())
            .collect();

        // Normalize to 0–255 range per row
        let min_db = db.iter().copied().fold(f32::INFINITY, f32::min);
        let max_db = db.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let span = (max_db - min_db).max(1e-6);

        db.iter()
            .map(|&x| (((x - min_db) / span) * 255.0).clamp(0.0, 255.0) as u8)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn generates_correct_length_row() {
        let mut wf = WaterfallGenerator::new(512);
        let iq = vec![Complex32::new(0.0, 0.0); 512];

        let row = wf.generate_row(&iq);
        assert_eq!(row.len(), 512);
    }

    #[test]
    fn detects_tone_peak_near_expected_bin() {
        let fft_size = 1024;
        let sample_rate = 48_000.0;
        let tone_hz = 6_000.0;

        let mut wf = WaterfallGenerator::new(fft_size);

        let iq: Vec<Complex32> = (0..fft_size)
            .map(|n| {
                let phase = 2.0 * PI * tone_hz * n as f32 / sample_rate;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let row = wf.generate_row(&iq);

        let peak_idx = row
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
            .unwrap();

        // After FFT shift:
        // DC is centered, positive frequencies to the right
        let expected_bin =
            (fft_size as f32 / 2.0 + tone_hz * fft_size as f32 / sample_rate).round() as usize;

        assert!(
            peak_idx.abs_diff(expected_bin) <= 3,
            "peak_idx={peak_idx}, expected_bin={expected_bin}"
        );
    }
}
