pub fn generate_audio_frame(size: usize) -> Vec<f32> {
(0..size).map(|i| (i as f32 * 0.01).sin()).collect()
}
