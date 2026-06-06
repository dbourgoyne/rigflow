use num_complex::Complex32;
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

/// Lowest dB value emitted by the generator.
///
/// This floor avoids `-inf` values when a bin magnitude is extremely small.
const DB_FLOOR: f32 = -140.0;

/// Generates waterfall rows from IQ samples using FFT.
///
/// Pipeline:
/// 1. Apply Hann window
/// 2. FFT
/// 3. Magnitude spectrum
/// 4. FFT shift (center DC)
/// 5. Normalize magnitude by window gain
/// 6. Convert to dB
///
/// Important:
/// - This type now returns raw spectral dB values.
/// - It does **not** perform display normalization or grayscale mapping.
pub struct WaterfallGenerator {
    fft_size: usize,

    /// FFT plan (cached for performance)
    fft: Arc<dyn Fft<f32>>,

    /// Reusable FFT buffer (complex samples)
    buffer: Vec<Complex<f32>>,

    /// Window function (Hann)
    window: Vec<f32>,

    /// Sum of window coefficients.
    ///
    /// This is used as a simple amplitude normalization factor so that
    /// a constant signal does not wildly change apparent dB level just
    /// because FFT size or windowing changes.
    window_sum: f32,
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
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (fft_size as f32 - 1.0)).cos()
            })
            .collect();

        let window_sum = window.iter().copied().sum::<f32>();

        Self {
            fft_size,
            fft,
            buffer: vec![Complex::new(0.0, 0.0); fft_size],
            window,
            window_sum,
        }
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Generate a single waterfall row of spectral dB values from IQ samples.
    ///
    /// Steps:
    /// - window + copy into FFT buffer
    /// - FFT
    /// - magnitude + fftshift
    /// - normalize magnitude by window gain
    /// - dB conversion
    pub fn generate_row_db(&mut self, iq: &[Complex32]) -> Vec<f32> {
        let n = self.fft_size.min(iq.len());
        if n == 0 {
            return Vec::new();
        }

        // Clear buffer (important if iq.len() < fft_size).
        for value in &mut self.buffer {
            *value = Complex::new(0.0, 0.0);
        }

        // Copy + apply window.
        for ((buf, iq_sample), &w) in self
            .buffer
            .iter_mut()
            .zip(iq.iter())
            .zip(self.window.iter())
            .take(n)
        {
            buf.re = iq_sample.re * w;
            buf.im = iq_sample.im * w;
        }

        // Perform FFT in-place.
        self.fft.process(&mut self.buffer);

        // Compute magnitude spectrum with FFT shift.
        //
        // Normalize by window sum so the dB scale is much more stable across
        // rows and across different signal types.
        let mut mags = vec![0.0_f32; self.fft_size];
        let half = self.fft_size / 2;
        let norm = self.window_sum.max(1e-12);

        for (i, mag) in mags.iter_mut().enumerate() {
            let src = (i + half) % self.fft_size;
            *mag = self.buffer[src].norm() / norm;
        }

        // Convert to dB and clamp to a finite floor.
        let mag_floor = 10.0_f32.powf(DB_FLOOR / 20.0);

        mags.into_iter()
            .map(|mag| {
                let mag = mag.max(mag_floor);
                20.0 * mag.log10()
            })
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

        let row = wf.generate_row_db(&iq);
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

        let row = wf.generate_row_db(&iq);

        let peak_idx = row
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        // After FFT shift:
        // DC is centered, positive frequencies to the right.
        let expected_bin =
            (fft_size as f32 / 2.0 + tone_hz * fft_size as f32 / sample_rate).round() as usize;

        assert!(
            peak_idx.abs_diff(expected_bin) <= 3,
            "peak_idx={peak_idx}, expected_bin={expected_bin}"
        );
    }

    #[test]
    fn row_values_are_finite() {
        let mut wf = WaterfallGenerator::new(512);
        let iq = vec![Complex32::new(0.0, 0.0); 512];

        let row = wf.generate_row_db(&iq);
        assert!(row.iter().all(|value| value.is_finite()));
        assert!(row.iter().all(|value| *value >= DB_FLOOR));
    }
}
