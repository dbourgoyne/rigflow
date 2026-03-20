use num_complex::Complex32;

/// Simple streaming decimator for complex samples.
///
/// Assumes the input has already been low-pass filtered
/// so decimation will not introduce aliasing.
pub struct Decimator {
    factor: usize,
    phase: usize,
}

impl Decimator {
    pub fn new(factor: usize) -> Self {
        assert!(factor >= 1, "decimation factor must be >= 1");

        Self { factor, phase: 0 }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn reset(&mut self) {
        self.phase = 0;
    }

    /// Decimate a block and return a new Vec.
    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        if self.factor == 1 {
            return input.to_vec();
        }

        let mut output = Vec::with_capacity(input.len().div_ceil(self.factor));

        for &sample in input {
            if self.phase == 0 {
                output.push(sample);
            }

            self.phase += 1;
            if self.phase == self.factor {
                self.phase = 0;
            }
        }

        output
    }

    /// Decimate into a caller-provided output buffer.
    pub fn process_into(&mut self, input: &[Complex32], output: &mut Vec<Complex32>) {
        output.clear();

        if self.factor == 1 {
            output.extend_from_slice(input);
            return;
        }

        output.reserve(input.len().div_ceil(self.factor));

        for &sample in input {
            if self.phase == 0 {
                output.push(sample);
            }

            self.phase += 1;
            if self.phase == self.factor {
                self.phase = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_complex::Complex32;

    #[test]
    fn factor_1_returns_all_samples() {
        let mut dec = Decimator::new(1);
        let input = vec![
            Complex32::new(1.0, 0.0),
            Complex32::new(2.0, 0.0),
            Complex32::new(3.0, 0.0),
        ];

        let output = dec.process(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn factor_2_keeps_every_other_sample() {
        let mut dec = Decimator::new(2);
        let input = vec![
            Complex32::new(0.0, 0.0),
            Complex32::new(1.0, 0.0),
            Complex32::new(2.0, 0.0),
            Complex32::new(3.0, 0.0),
            Complex32::new(4.0, 0.0),
            Complex32::new(5.0, 0.0),
        ];

        let output = dec.process(&input);

        assert_eq!(
            output,
            vec![
                Complex32::new(0.0, 0.0),
                Complex32::new(2.0, 0.0),
                Complex32::new(4.0, 0.0),
            ]
        );
    }

    #[test]
    fn preserves_phase_across_multiple_blocks() {
        let mut dec = Decimator::new(3);

        let block1 = vec![Complex32::new(0.0, 0.0), Complex32::new(1.0, 0.0)];
        let block2 = vec![
            Complex32::new(2.0, 0.0),
            Complex32::new(3.0, 0.0),
            Complex32::new(4.0, 0.0),
            Complex32::new(5.0, 0.0),
        ];

        let out1 = dec.process(&block1);
        let out2 = dec.process(&block2);

        assert_eq!(out1, vec![Complex32::new(0.0, 0.0)]);
        assert_eq!(out2, vec![Complex32::new(3.0, 0.0),]);
    }

    #[test]
    fn process_into_matches_process() {
        let mut dec_a = Decimator::new(4);
        let mut dec_b = Decimator::new(4);

        let input: Vec<Complex32> = (0..20).map(|x| Complex32::new(x as f32, 0.0)).collect();

        let out_a = dec_a.process(&input);

        let mut out_b = Vec::new();
        dec_b.process_into(&input, &mut out_b);

        assert_eq!(out_a, out_b);
    }
}
