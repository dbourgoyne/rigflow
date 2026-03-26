pub struct DcBlocker {
    r: f32,
    prev_x: f32,
    prev_y: f32,
}

impl DcBlocker {
    pub fn new(r: f32) -> Self {
        assert!(r > 0.0 && r < 1.0, "r must be between 0 and 1");

        Self {
            r,
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
    }

    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = x - self.prev_x + self.r * self.prev_y;
        self.prev_x = x;
        self.prev_y = y;
        y
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_constant_offset() {
        let mut dc = DcBlocker::new(0.995);
        let input = vec![1.0_f32; 4096];
        let output = dc.process(&input);

        let tail = &output[1024..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;

        assert!(mean.abs() < 1e-2, "mean was {mean}");
    }

    #[test]
    fn process_and_in_place_match() {
        let mut a = DcBlocker::new(0.995);
        let mut b = DcBlocker::new(0.995);

        let input = vec![0.5, 1.0, -0.25, 0.75, -0.5, 0.0];
        let out_a = a.process(&input);

        let mut in_place = input.clone();
        b.process_in_place(&mut in_place);

        for (x, y) in out_a.iter().zip(in_place.iter()) {
            assert!((x - y).abs() < 1e-6);
        }
    }
}
