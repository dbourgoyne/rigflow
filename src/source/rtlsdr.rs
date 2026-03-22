use num_complex::Complex32;
use rtl_sdr_rs::{RtlSdr, TunerGain};

use crate::source::IqSource;

pub struct RtlSdrSource {
    dev: RtlSdr,
    sample_rate_hz: f32,
    center_freq_hz: u32,
    raw_buf: Vec<u8>,
}

impl RtlSdrSource {
    pub fn open(
        device_index: usize,
        sample_rate_hz: u32,
        center_freq_hz: u32,
        gain_tenths_db: Option<i32>,
        ppm_correction: i32,
        direct_sampling: bool,
        block_complex_samples: usize,
    ) -> Result<Self, String> {
        let mut dev = RtlSdr::open_with_index(device_index)
            .map_err(|e| format!("failed to open RTL-SDR device {device_index}: {e}"))?;

        dev.set_sample_rate(sample_rate_hz)
            .map_err(|e| format!("failed to set sample rate to {sample_rate_hz} Hz: {e}"))?;

        dev.set_center_freq(center_freq_hz)
            .map_err(|e| format!("failed to set center frequency to {center_freq_hz} Hz: {e}"))?;

        if ppm_correction != 0 {
            dev.set_freq_correction(ppm_correction)
                .map_err(|e| format!("failed to set PPM correction to {ppm_correction}: {e}"))?;
        }

        if direct_sampling {
            dev.set_direct_sampling(rtl_sdr_rs::DirectSampleMode::On)
                .map_err(|e| format!("failed to enable direct sampling: {e}"))?;
        }

        match gain_tenths_db {
            Some(gain) => dev
                .set_tuner_gain(TunerGain::Manual(gain))
                .map_err(|e| format!("failed to set manual tuner gain to {gain} tenths dB: {e}"))?,
            None => dev
                .set_tuner_gain(TunerGain::Auto)
                .map_err(|e| format!("failed to enable automatic tuner gain: {e}"))?,
        }

        dev.reset_buffer()
            .map_err(|e| format!("failed to reset RTL-SDR buffer: {e}"))?;

        let raw_len = block_complex_samples * 2;

	Ok(Self {
	    dev,
	    sample_rate_hz: sample_rate_hz as f32,
	    center_freq_hz,
	    raw_buf: vec![0u8; raw_len],
	})

    }

    pub fn set_center_frequency(&mut self, center_freq_hz: u32) -> Result<(), String> {
        self.dev
            .set_center_freq(center_freq_hz)
            .map_err(|e| format!("failed to retune RTL-SDR to {center_freq_hz} Hz: {e}"))?;
        self.center_freq_hz = center_freq_hz;
        Ok(())
    }

    pub fn center_frequency(&self) -> u32 {
        self.center_freq_hz
    }

    pub fn device_summary() -> Result<String, String> {
        let devices = RtlSdr::list_devices()
            .map_err(|e| format!("failed to enumerate RTL-SDR devices: {e}"))?;

        if devices.is_empty() {
            return Ok("No RTL-SDR devices found".to_string());
        }

        let mut out = String::new();
        for d in devices {
            use std::fmt::Write as _;
            let _ = writeln!(
                out,
                "index={} vendor={:04x} product={:04x} manufacturer='{}' product='{}' serial='{}'",
                d.index, d.vendor_id, d.product_id, d.manufacturer, d.product, d.serial
            );
        }
        Ok(out)
    }
}

impl IqSource for RtlSdrSource {
    fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        let needed_bytes = max_samples * 2;

        if self.raw_buf.len() != needed_bytes {
            self.raw_buf.resize(needed_bytes, 0u8);
        }

        let n = self
            .dev
            .read_sync(&mut self.raw_buf)
            .map_err(|e| format!("RTL-SDR read_sync failed: {e}"))?;

        if n == 0 {
            return Ok(Vec::new());
        }

        let valid = n - (n % 2);
        let bytes = &self.raw_buf[..valid];

        let mut out = Vec::with_capacity(bytes.len() / 2);

        // RTL-SDR returns unsigned 8-bit interleaved IQ centered at 127.5
        for pair in bytes.chunks_exact(2) {
            let i = (pair[0] as f32 - 127.5) / 128.0;
            let q = (pair[1] as f32 - 127.5) / 128.0;
            out.push(Complex32::new(i, q));
        }

        Ok(out)
    }
}
