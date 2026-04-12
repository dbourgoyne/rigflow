/// Simple streaming linear interpolation resampler.
///
/// Converts audio from `input_rate_hz` to `output_rate_hz` using
/// first-order (linear) interpolation.
///
/// Characteristics:
/// - very low CPU cost
/// - minimal latency
/// - moderate frequency response quality (good enough for voice/FM audio)
///
/// This is a **stateful streaming resampler**:
/// - maintains fractional position between calls
/// - preserves continuity across blocks via `prev_input`
pub struct AudioResampler {
    input_rate_hz: f32,
    output_rate_hz: f32,

    /// Step size in input-sample units per output sample
    /// (input_rate / output_rate)
    step: f32,

    /// Current fractional position in input sample space
    position: f32,

    /// Last input sample from previous block (for continuity)
    prev_input: Option<f32>,
}

impl AudioResampler {
    /// Create a new resampler.
    ///
    /// Parameters:
    /// - `input_rate_hz`: source sample rate
    /// - `output_rate_hz`: desired output sample rate
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

    /// Reset internal state.
    ///
    /// Should be called when:
    /// - switching radios
    /// - stream discontinuities occur
    pub fn reset(&mut self) {
        self.position = 0.0;
        self.prev_input = None;
    }

    /// Resample input audio and return a new output buffer.
    ///
    /// Algorithm:
    /// 1. Build a working buffer including previous sample (for continuity)
    /// 2. Step through input space using fractional position
    /// 3. Linearly interpolate between adjacent samples
    /// 4. Carry fractional position forward for next call
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }

        // Build working buffer:
        // prepend last sample from previous block to ensure continuity
        let mut work = Vec::with_capacity(input.len() + 1);

        if let Some(prev) = self.prev_input {
            work.push(prev);
        } else {
            // First call: duplicate first sample to bootstrap interpolation
            work.push(input[0]);
        }

        work.extend_from_slice(input);

        let mut output = Vec::new();

        // position is expressed in input-sample space (within `work`)
        while self.position + 1.0 < work.len() as f32 {
            let i0 = self.position.floor() as usize;
            let frac = self.position - i0 as f32;

            let s0 = work[i0];
            let s1 = work[i0 + 1];

            // Linear interpolation
            let y = s0 + frac * (s1 - s0);
            output.push(y);

            self.position += self.step;
        }

        // Shift position so it remains relative to the next block
        self.position -= (work.len() - 1) as f32;

        // Save last input sample for continuity across blocks
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

        assert!(
            (in_rms - out_rms).abs() < 0.1,
            "in_rms={in_rms}, out_rms={out_rms}"
        );
    }
}
