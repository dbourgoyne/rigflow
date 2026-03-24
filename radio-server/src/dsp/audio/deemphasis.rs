pub struct DeemphasisFilter {
    alpha: f32,
    y_prev: f32,
}

impl DeemphasisFilter {
    pub fn new(sample_rate_hz: f32, tau_seconds: f32) -> Self {
        let dt = 1.0 / sample_rate_hz;
        let alpha = dt / (tau_seconds + dt);

        Self {
            alpha,
            y_prev: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.y_prev = 0.0;
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for x in samples.iter_mut() {
            self.y_prev = self.y_prev + self.alpha * (*x - self.y_prev);
            *x = self.y_prev;
        }
    }
}
