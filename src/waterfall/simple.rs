use num_complex::Complex32;
use std::f32::consts::PI;

pub fn generate_waterfall_row(iq: &[Complex32], fft_size: usize) -> Vec<u8> {
    let n = fft_size.min(iq.len());
    if n == 0 {
        return Vec::new();
    }

    let mut mags = vec![0.0_f32; n];

    for k in 0..n {
        let mut acc = Complex32::new(0.0, 0.0);

        for (i, &sample) in iq.iter().take(n).enumerate() {
            let window = 0.5 - 0.5 * (2.0 * PI * i as f32 / (n as f32 - 1.0)).cos();
            let x = sample * window;

            let phase = -2.0 * PI * k as f32 * i as f32 / n as f32;
            let w = Complex32::new(phase.cos(), phase.sin());
            acc += x * w;
        }

        mags[k] = acc.norm();
    }

    // fftshift
    let mut shifted = vec![0.0_f32; n];
    let half = n / 2;
    shifted[..n - half].copy_from_slice(&mags[half..]);
    shifted[n - half..].copy_from_slice(&mags[..half]);

    let eps = 1e-12_f32;
    let db: Vec<f32> = shifted
        .iter()
        .map(|&x| 20.0 * (x + eps).log10())
        .collect();

    let min_db = db.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_db = db.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let span = (max_db - min_db).max(1e-6);

    db.iter()
        .map(|&x| {
            let y = ((x - min_db) / span * 255.0).clamp(0.0, 255.0);
            y as u8
        })
        .collect()
}
