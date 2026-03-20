use num_complex::Complex32;

pub struct IQBuffer {
    pub data: Vec<Complex32>,
}

impl IQBuffer {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![Complex32::new(0.0, 0.0); size],
        }
    }
}
