pub fn fake_fft(size: usize) -> Vec<f32> {
(0..size).map(|i| (i as f32).sin()).collect()
}
