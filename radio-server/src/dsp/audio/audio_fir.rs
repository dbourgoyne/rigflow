use std::f32::consts::PI;

/// Streaming low-pass FIR filter for real-valued audio samples.
pub struct AudioFir {
    taps: Vec<f32>,
    delay: Vec<f32>,
    pos: usize,
}

impl AudioFir {
    /// Create a low-pass FIR using a windowed-sinc design.
    ///
    /// `sample_rate_hz` - audio sample rate
    /// `cutoff_hz`      - low-pass cutoff
    /// `num_taps`       - usually odd
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
            delay: vec![0.0; num_taps],
            taps,
            pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.delay.fill(0.0);
        self.pos = 0;
    }

    pub fn process_sample(&mut self, sample: f32) -> f32 {
        self.delay[self.pos] = sample;

        let len = self.taps.len();
        let mut idx = self.pos;
        let mut acc = 0.0_f32;

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

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&x| self.process_sample(x)).collect()
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples.iter_mut() {
            *sample = self.process_sample(*sample);
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
    use std::f32::consts::PI;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn preserves_dc_after_warmup() {
        let mut fir = AudioFir::new(12_000.0, 3_000.0, 101);

        let input = vec![1.0_f32; 4096];
        let output = fir.process(&input);

        let steady = &output[512..];
        let mean = steady.iter().sum::<f32>() / steady.len() as f32;

        assert!(approx_eq(mean, 1.0, 1e-3), "mean was {mean}");
    }

    #[test]
    fn attenuates_high_frequency() {
        let sample_rate = 12_000.0;
        let cutoff = 3_000.0;
        let mut fir = AudioFir::new(sample_rate, cutoff, 101);

        let tone_hz = 5_000.0;
        let input: Vec<f32> = (0..4096)
            .map(|n| {
                let phase = 2.0 * PI * tone_hz * n as f32 / sample_rate;
                phase.sin()
            })
            .collect();

        let output = fir.process(&input);

        let input_power = input.iter().map(|x| x * x).sum::<f32>() / input.len() as f32;
        let output_power =
            output[512..].iter().map(|x| x * x).sum::<f32>() / (output.len() - 512) as f32;

        assert!(
            output_power < input_power * 0.2,
            "expected attenuation, input_power={input_power}, output_power={output_power}"
        );
    }

    #[test]
    fn process_and_in_place_match() {
        let mut a = AudioFir::new(12_000.0, 3_000.0, 63);
        let mut b = AudioFir::new(12_000.0, 3_000.0, 63);

        let input: Vec<f32> = (0..1024)
            .map(|n| {
                let phase = 2.0 * PI * 1_000.0 * n as f32 / 12_000.0;
                phase.sin()
            })
            .collect();

        let out_a = a.process(&input);

        let mut in_place = input.clone();
        b.process_in_place(&mut in_place);

        for (i, (x, y)) in out_a.iter().zip(in_place.iter()).enumerate() {
            assert!(approx_eq(*x, *y, 1e-6), "mismatch at {i}: x={x}, y={y}");
        }
    }
}
