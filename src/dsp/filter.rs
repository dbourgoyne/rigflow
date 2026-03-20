use num_complex::Complex32;
use std::f32::consts::PI;

/// Streaming low-pass FIR filter for complex IQ samples.
pub struct LowPassFir {
    taps: Vec<f32>,
    delay: Vec<Complex32>,
    pos: usize,
}

impl LowPassFir {
    /// Create a low-pass FIR using a windowed-sinc design.
    ///
    /// `sample_rate_hz` - input sample rate
    /// `cutoff_hz`      - low-pass cutoff frequency
    /// `num_taps`       - number of taps, should usually be odd
    pub fn new(sample_rate_hz: f32, cutoff_hz: f32, num_taps: usize) -> Self {
        assert!(sample_rate_hz > 0.0, "sample_rate_hz must be > 0");
        assert!(cutoff_hz > 0.0, "cutoff_hz must be > 0");
        assert!(
            cutoff_hz < sample_rate_hz * 0.5,
            "cutoff_hz must be below Nyquist"
        );
        assert!(num_taps >= 3, "num_taps must be at least 3");

        let taps = design_lowpass(sample_rate_hz, cutoff_hz, num_taps);

        Self {
            taps,
            delay: vec![Complex32::new(0.0, 0.0); num_taps],
            pos: 0,
        }
    }

    pub fn taps(&self) -> &[f32] {
        &self.taps
    }

    pub fn len(&self) -> usize {
        self.taps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.taps.is_empty()
    }

    pub fn reset(&mut self) {
        self.delay.fill(Complex32::new(0.0, 0.0));
        self.pos = 0;
    }

    /// Process one sample.
    pub fn process_sample(&mut self, sample: Complex32) -> Complex32 {
        self.delay[self.pos] = sample;

        let mut acc = Complex32::new(0.0, 0.0);
        let mut idx = self.pos;
        let len = self.taps.len();

        for &tap in &self.taps {
            acc += self.delay[idx] * tap;

            if idx == 0 {
                idx = len - 1;
            } else {
                idx -= 1;
            }
        }

        self.pos += 1;
        if self.pos == len {
            self.pos = 0;
        }

        acc
    }

    /// Process a block, returning a new output vector.
    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        input.iter().map(|&x| self.process_sample(x)).collect()
    }

    /// Process a block in place.
    pub fn process_in_place(&mut self, samples: &mut [Complex32]) {
        for sample in samples.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    }
}

/// Design a normalized low-pass FIR using windowed sinc + Hamming window.
fn design_lowpass(sample_rate_hz: f32, cutoff_hz: f32, num_taps: usize) -> Vec<f32> {
    let fc = cutoff_hz / sample_rate_hz; // normalized to sample rate
    let m = (num_taps - 1) as f32;
    let mid = m / 2.0;

    let mut taps = Vec::with_capacity(num_taps);

    for n in 0..num_taps {
        let x = n as f32 - mid;

        // Ideal low-pass impulse response:
        // h[n] = 2fc * sinc(2fc * (n - M/2))
        let sinc = if x.abs() < 1e-12 {
            2.0 * fc
        } else {
            let arg = 2.0 * PI * fc * x;
            arg.sin() / (PI * x)
        };

        // Hamming window
        let w = 0.54 - 0.46 * (2.0 * PI * n as f32 / m).cos();

        taps.push(sinc * w);
    }

    // Normalize DC gain to 1.0
    let sum: f32 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= sum;
    }

    taps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn taps_sum_to_one() {
        let fir = LowPassFir::new(48_000.0, 3_000.0, 101);
        let sum: f32 = fir.taps().iter().sum();
        assert!(approx_eq(sum, 1.0, 1e-4), "tap sum was {sum}");
    }

    #[test]
    fn preserves_dc_after_warmup() {
        let mut fir = LowPassFir::new(48_000.0, 3_000.0, 101);

        let input = vec![Complex32::new(1.0, 0.0); 4096];
        let output = fir.process(&input);

        let steady = &output[512..];
        let mean_re = steady.iter().map(|s| s.re).sum::<f32>() / steady.len() as f32;
        let mean_im = steady.iter().map(|s| s.im).sum::<f32>() / steady.len() as f32;

        assert!(approx_eq(mean_re, 1.0, 1e-3), "mean_re was {mean_re}");
        assert!(approx_eq(mean_im, 0.0, 1e-3), "mean_im was {mean_im}");
    }

    #[test]
    fn attenuates_high_frequency() {
        let sample_rate = 48_000.0;
        let cutoff = 3_000.0;
        let mut fir = LowPassFir::new(sample_rate, cutoff, 101);

        // Tone well above cutoff
        let tone_hz = 12_000.0;
        let input: Vec<Complex32> = (0..4096)
            .map(|n| {
                let phase = 2.0 * PI * tone_hz * n as f32 / sample_rate;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let output = fir.process(&input);

        let input_power =
            input.iter().map(|x| x.norm_sqr()).sum::<f32>() / input.len() as f32;
        let output_power =
            output[512..].iter().map(|x| x.norm_sqr()).sum::<f32>() / (output.len() - 512) as f32;

        assert!(
            output_power < input_power * 0.1,
            "expected strong attenuation, input_power={input_power}, output_power={output_power}"
        );
    }

    #[test]
    fn process_and_process_in_place_match() {
        let sample_rate = 48_000.0;
        let cutoff = 4_000.0;

        let input: Vec<Complex32> = (0..1024)
            .map(|n| {
                let phase = 2.0 * PI * 1_000.0 * n as f32 / sample_rate;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let mut fir_a = LowPassFir::new(sample_rate, cutoff, 63);
        let mut fir_b = LowPassFir::new(sample_rate, cutoff, 63);

        let out_a = fir_a.process(&input);

        let mut in_place = input.clone();
        fir_b.process_in_place(&mut in_place);

        for (i, (a, b)) in out_a.iter().zip(in_place.iter()).enumerate() {
            assert!(
                approx_eq(a.re, b.re, 1e-6) && approx_eq(a.im, b.im, 1e-6),
                "mismatch at {i}: a=({:.6}, {:.6}), b=({:.6}, {:.6})",
                a.re, a.im, b.re, b.im
            );
        }
    }
}
