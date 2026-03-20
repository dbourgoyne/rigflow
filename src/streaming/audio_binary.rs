pub fn f32_samples_to_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);

    for &sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    out
}

pub fn f32_samples_to_i16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);

    for &sample in samples {
        let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }

    out
}
