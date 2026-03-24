use num_complex::Complex32;
use std::f32::consts::PI;

/// FIR decimator for complex IQ samples.
///
/// This combines:
/// - low-pass FIR filtering
/// - decimation by `factor`
///
/// It only computes one FIR output for every `factor` input samples,
/// which is much more efficient than filtering every sample and then
/// discarding most of them.
pub struct PolyphaseDecimator {
    taps: Vec<f32>,
    delay: Vec<Complex32>,
    write_pos: usize,
    factor: usize,
    phase: usize,
}

impl PolyphaseDecimator {
    pub fn new(
        input_sample_rate_hz: f32,
        cutoff_hz: f32,
        num_taps: usize,
        factor: usize,
    ) -> Self {
        assert!(input_sample_rate_hz > 0.0, "input_sample_rate_hz must be > 0");
        assert!(cutoff_hz > 0.0, "cutoff_hz must be > 0");
        assert!(num_taps >= 3, "num_taps must be at least 3");
        assert!(factor >= 1, "decimation factor must be >= 1");
        assert!(
            cutoff_hz < input_sample_rate_hz * 0.5,
            "cutoff_hz must be below Nyquist"
        );

        let taps = design_lowpass(input_sample_rate_hz, cutoff_hz, num_taps);

        Self {
            delay: vec![Complex32::new(0.0, 0.0); num_taps],
            taps,
            write_pos: 0,
            factor,
            phase: 0,
        }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn reset(&mut self) {
        self.delay.fill(Complex32::new(0.0, 0.0));
        self.write_pos = 0;
        self.phase = 0;
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut output = Vec::with_capacity(input.len().div_ceil(self.factor));

        let len = self.delay.len();

        for &sample in input {
            self.delay[self.write_pos] = sample;

            self.write_pos += 1;
            if self.write_pos == len {
                self.write_pos = 0;
            }

            if self.phase == 0 {
                let mut acc = Complex32::new(0.0, 0.0);

                // Oldest/newest indexing through the circular buffer
                let mut idx = if self.write_pos == 0 { len - 1 } else { self.write_pos - 1 };

                for &tap in &self.taps {
                    acc += self.delay[idx] * tap;

                    if idx == 0 {
                        idx = len - 1;
                    } else {
                        idx -= 1;
                    }
                }

                output.push(acc);
            }

            self.phase += 1;
            if self.phase == self.factor {
                self.phase = 0;
            }
        }

        output
    }

    pub fn process_into(&mut self, input: &[Complex32], output: &mut Vec<Complex32>) {
        output.clear();
        output.reserve(input.len().div_ceil(self.factor));

        let len = self.delay.len();

        for &sample in input {
            self.delay[self.write_pos] = sample;

            self.write_pos += 1;
            if self.write_pos == len {
                self.write_pos = 0;
            }

            if self.phase == 0 {
                let mut acc = Complex32::new(0.0, 0.0);

                let mut idx = if self.write_pos == 0 { len - 1 } else { self.write_pos - 1 };

                for &tap in &self.taps {
                    acc += self.delay[idx] * tap;

                    if idx == 0 {
                        idx = len - 1;
                    } else {
                        idx -= 1;
                    }
                }

                output.push(acc);
            }

            self.phase += 1;
            if self.phase == self.factor {
                self.phase = 0;
            }
        }
    }
}

fn design_lowpass(sample_rate_hz: f32, cutoff_hz: f32, num_taps: usize) -> Vec<f32> {
    let fc = cutoff_hz / sample_rate_hz;
    let m = (num_taps - 1) as f32;
    let mid = m / 2.0;

    let mut taps = Vec::with_capacity(num_taps);

    for n in 0..num_taps {
        let x = n as f32 - mid;

        let sinc = if x.abs() < 1e-12 {
            2.0 * fc
        } else {
            let arg = 2.0 * PI * fc * x;
            arg.sin() / (PI * x)
        };

        let w = 0.54 - 0.46 * (2.0 * PI * n as f32 / m).cos();
        taps.push(sinc * w);
    }

    let sum: f32 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= sum;
    }

    taps
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_complex::Complex32;
    use std::f32::consts::PI;

    #[test]
    fn factor_1_outputs_same_length() {
        let mut dec = PolyphaseDecimator::new(48_000.0, 4_000.0, 63, 1);
        let input = vec![Complex32::new(1.0, 0.0); 256];
        let output = dec.process(&input);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn decimates_length_approximately() {
        let mut dec = PolyphaseDecimator::new(48_000.0, 4_000.0, 63, 4);
        let input = vec![Complex32::new(1.0, 0.0); 1000];
        let output = dec.process(&input);
        assert!(output.len() >= 249 && output.len() <= 251);
    }

    #[test]
    fn preserves_dc_after_warmup() {
        let mut dec = PolyphaseDecimator::new(48_000.0, 4_000.0, 101, 4);
        let input = vec![Complex32::new(1.0, 0.0); 4096];
        let output = dec.process(&input);

        let steady = &output[128..];
        let mean_re = steady.iter().map(|s| s.re).sum::<f32>() / steady.len() as f32;
        let mean_im = steady.iter().map(|s| s.im).sum::<f32>() / steady.len() as f32;

        assert!((mean_re - 1.0).abs() < 1e-2, "mean_re={mean_re}");
        assert!(mean_im.abs() < 1e-2, "mean_im={mean_im}");
    }

    #[test]
    fn attenuates_high_frequency() {
        let sample_rate = 48_000.0;
        let mut dec = PolyphaseDecimator::new(sample_rate, 3_000.0, 101, 4);

        let tone_hz = 12_000.0;
        let input: Vec<Complex32> = (0..4096)
            .map(|n| {
                let phase = 2.0 * PI * tone_hz * n as f32 / sample_rate;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let output = dec.process(&input);

        let out_power =
            output[64..].iter().map(|x| x.norm_sqr()).sum::<f32>() / (output.len() - 64) as f32;

        assert!(out_power < 0.1, "expected attenuation, got {out_power}");
    }
}
