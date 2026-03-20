pub fn generate_frame(data: &[f32]) -> Vec<u8> {
    data.iter().map(|x| (x.abs() * 255.0) as u8).collect()
}
