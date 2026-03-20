pub struct Agc {
    target_level: f32,
    attack: f32,
    decay: f32,
    max_gain: f32,
    envelope: f32,
}

impl Agc {
    pub fn new(target_level: f32, attack: f32, decay: f32, max_gain: f32) -> Self {
        assert!(target_level > 0.0, "target_level must be > 0");
        assert!(attack > 0.0 && attack < 1.0, "attack must be between 0 and 1");
        assert!(decay > 0.0 && decay < 1.0, "decay must be between 0 and 1");
        assert!(max_gain > 0.0, "max_gain must be > 0");

        Self {
            target_level,
            attack,
            decay,
            max_gain,
            envelope: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    pub fn process_sample(&mut self, x: f32) -> f32 {
        let level = x.abs();

        if level > self.envelope {
            self.envelope = self.attack * self.envelope + (1.0 - self.attack) * level;
        } else {
            self.envelope = self.decay * self.envelope + (1.0 - self.decay) * level;
        }

        let gain = if self.envelope > 1e-9 {
            (self.target_level / self.envelope).min(self.max_gain)
        } else {
            self.max_gain
        };

        x * gain
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

    fn rms(x: &[f32]) -> f32 {
        let p = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
        p.sqrt()
    }

    #[test]
    fn boosts_small_signal() {
        let mut agc = Agc::new(0.5, 0.9, 0.999, 20.0);
        let input = vec![0.05_f32; 4096];
        let output = agc.process(&input);

        let in_rms = rms(&input[1024..]);
        let out_rms = rms(&output[1024..]);

        assert!(out_rms > in_rms, "expected AGC to boost signal");
    }

    #[test]
    fn limits_gain() {
        let mut agc = Agc::new(0.5, 0.9, 0.999, 4.0);
        let input = vec![0.001_f32; 4096];
        let output = agc.process(&input);

        let peak = output.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak <= 0.0041, "peak was {peak}");
    }

    #[test]
    fn process_and_in_place_match() {
        let mut a = Agc::new(0.5, 0.9, 0.999, 10.0);
        let mut b = Agc::new(0.5, 0.9, 0.999, 10.0);

        let input = vec![0.1, 0.2, -0.4, 0.05, -0.3, 0.6];
        let out_a = a.process(&input);

        let mut in_place = input.clone();
        b.process_in_place(&mut in_place);

        for (x, y) in out_a.iter().zip(in_place.iter()) {
            assert!((x - y).abs() < 1e-6);
        }
    }
}
