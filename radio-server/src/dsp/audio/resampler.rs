pub struct AudioResampler {
    input_rate_hz: f32,
    output_rate_hz: f32,
    step: f32,
    position: f32,
    prev_input: Option<f32>,
}

impl AudioResampler {
    pub fn new(input_rate_hz: f32, output_rate_hz: f32) -> Self {
        assert!(input_rate_hz > 0.0, "input_rate_hz must be > 0");
        assert!(output_rate_hz > 0.0, "output_rate_hz must be > 0");

        Self {
            input_rate_hz,
            output_rate_hz,
            step: input_rate_hz / output_rate_hz,
            position: 0.0,
            prev_input: None,
        }
    }

    pub fn input_rate(&self) -> f32 {
        self.input_rate_hz
    }

    pub fn output_rate(&self) -> f32 {
        self.output_rate_hz
    }

    pub fn reset(&mut self) {
        self.position = 0.0;
        self.prev_input = None;
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }

        // Build a temporary working buffer that starts with the previous
        // sample from the last block so interpolation stays continuous.
        let mut work = Vec::with_capacity(input.len() + 1);

        if let Some(prev) = self.prev_input {
            work.push(prev);
        } else {
            work.push(input[0]);
        }

        work.extend_from_slice(input);

        let mut output = Vec::new();

        // position is in units of input samples within `work`
        while self.position + 1.0 < work.len() as f32 {
            let i0 = self.position.floor() as usize;
            let frac = self.position - i0 as f32;

            let s0 = work[i0];
            let s1 = work[i0 + 1];

            let y = s0 + frac * (s1 - s0);
            output.push(y);

            self.position += self.step;
        }

        // Shift position so next block continues correctly.
        self.position -= (work.len() - 1) as f32;

        // Keep last real input sample for continuity.
        self.prev_input = input.last().copied();

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn rms(x: &[f32]) -> f32 {
        let p = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
        p.sqrt()
    }

    #[test]
    fn same_rate_keeps_length_close() {
        let mut r = AudioResampler::new(48_000.0, 48_000.0);
        let input = vec![0.0_f32; 1024];
        let output = r.process(&input);
        assert!(output.len() >= 1023 && output.len() <= 1025);
    }

    #[test]
    fn upsamples() {
        let mut r = AudioResampler::new(12_000.0, 48_000.0);
        let input = vec![0.0_f32; 1000];
        let output = r.process(&input);
        assert!(output.len() >= 3995 && output.len() <= 4005);
    }

    #[test]
    fn downsamples() {
        let mut r = AudioResampler::new(48_000.0, 12_000.0);
        let input = vec![0.0_f32; 1000];
        let output = r.process(&input);
        assert!(output.len() >= 245 && output.len() <= 255);
    }

    #[test]
    fn preserves_tone_reasonably() {
        let in_rate = 12_000.0;
        let out_rate = 48_000.0;
        let tone_hz = 1_000.0;

        let input: Vec<f32> = (0..4000)
            .map(|n| {
                let phase = 2.0 * PI * tone_hz * n as f32 / in_rate;
                phase.sin()
            })
            .collect();

        let in_rms = rms(&input[100..]);

        let mut r = AudioResampler::new(in_rate, out_rate);
        let output = r.process(&input);
        let out_rms = rms(&output[400..]);

        assert!((in_rms - out_rms).abs() < 0.1, "in_rms={in_rms}, out_rms={out_rms}");
    }
}
