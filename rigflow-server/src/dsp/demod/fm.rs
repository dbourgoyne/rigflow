use num_complex::Complex32;

pub struct FmDemodulator {
    prev: Complex32,
    have_prev: bool,
}

impl Default for FmDemodulator {
    fn default() -> Self {
        Self::new()
    }
}

impl FmDemodulator {
    pub fn new() -> Self {
        Self {
            prev: Complex32::new(0.0, 0.0),
            have_prev: false,
        }
    }

    pub fn reset(&mut self) {
        self.prev = Complex32::new(0.0, 0.0);
        self.have_prev = false;
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());

        for &x in input {
            if !self.have_prev {
                self.prev = x;
                self.have_prev = true;
                out.push(0.0);
                continue;
            }

            let d = x * self.prev.conj();
            let y = d.im.atan2(d.re);

            out.push(y);
            self.prev = x;
        }

        out
    }
}
