use num_complex::Complex32;
use rigflow_core::radio::source_control::{
    DirectSamplingMode, GainMode, SourceCapabilities, SourceControlState,
};
use rtl_sdr_rs::{RtlSdr, TunerGain};

use crate::source::IqSource;

/// RTL-SDR IQ source backed by librtlsdr.
///
/// Produces interleaved IQ samples converted to Complex32.
pub struct RtlSdrSource {
    dev: RtlSdr,
    sample_rate_hz: f32,
    center_freq_hz: u32,
    raw_buf: Vec<u8>,

    gain_mode: GainMode,
    gain_db: f32,
    ppm_correction: i32,
    direct_sampling: DirectSamplingMode,
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

        // Configure gain (manual or automatic)
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

        let (gain_mode, gain_db) = match gain_tenths_db {
            Some(gain) => (GainMode::Manual, gain as f32 / 10.0),
            None => (GainMode::Auto, 0.0),
        };

        let direct_sampling_mode = if direct_sampling {
            DirectSamplingMode::I
        } else {
            DirectSamplingMode::Off
        };

        Ok(Self {
            dev,
            sample_rate_hz: sample_rate_hz as f32,
            center_freq_hz,
            raw_buf: vec![0u8; raw_len],

            gain_mode,
            gain_db,
            ppm_correction,
            direct_sampling: direct_sampling_mode,
        })
    }

    /// Retune the device center frequency.
    pub fn set_center_frequency_hz(&mut self, center_freq_hz: u32) -> Result<(), String> {
        self.dev
            .set_center_freq(center_freq_hz)
            .map_err(|e| format!("failed to retune RTL-SDR to {center_freq_hz} Hz: {e}"))?;

        self.center_freq_hz = center_freq_hz;
        Ok(())
    }

    pub fn center_frequency(&self) -> u32 {
        self.center_freq_hz
    }

    /// Returns a human-readable list of connected RTL-SDR devices.
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

/// Minimal per-device info from RTL-SDR enumeration, used by radio discovery.
pub struct RtlDeviceInfo {
    /// Enumeration index — the value to pass to [`RtlSdrSource::open`] /
    /// `RtlSdr::open_with_index`.
    pub index: usize,
    /// USB serial string (may be empty / non-unique on cheap dongles).
    pub serial: String,
    /// USB product string (may be empty).
    pub product: String,
}

/// Enumerate the RTL-SDR devices currently present (empty when none).
///
/// Wraps `rtl_sdr_rs::RtlSdr::list_devices` so the `rtl_sdr_rs` dependency stays
/// contained in this source module; discovery builds `RadioDescriptor`s from it.
pub fn list_rtl_devices() -> Result<Vec<RtlDeviceInfo>, String> {
    let devices = RtlSdr::list_devices().map_err(|e| format!("RTL-SDR enumeration failed: {e}"))?;
    Ok(devices
        .into_iter()
        .map(|d| RtlDeviceInfo {
            index: d.index,
            serial: d.serial,
            product: d.product,
        })
        .collect())
}

impl IqSource for RtlSdrSource {
    fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        let needed_bytes = max_samples * 2;

        // Ensure buffer is correctly sized
        if self.raw_buf.len() != needed_bytes {
            self.raw_buf.resize(needed_bytes, 0);
        }

        let n = self
            .dev
            .read_sync(&mut self.raw_buf)
            .map_err(|e| format!("RTL-SDR read_sync failed: {e}"))?;

        if n == 0 {
            return Ok(Vec::new());
        }

        // Ensure we only process complete IQ pairs
        let valid = n - (n % 2);
        let bytes = &self.raw_buf[..valid];

        let mut out = Vec::with_capacity(bytes.len() / 2);

        for pair in bytes.chunks_exact(2) {
            // Convert unsigned 8-bit IQ to [-1.0, 1.0] float range
            let i = (pair[0] as f32 - 127.5) / 128.0;
            let q = (pair[1] as f32 - 127.5) / 128.0;

            out.push(Complex32::new(i, q));
        }

        Ok(out)
    }

    fn set_center_frequency(&mut self, center_freq_hz: f32) -> Result<(), String> {
        let hz = center_freq_hz.round() as u32;

        if self.direct_sampling == DirectSamplingMode::Off {
            self.dev
                .set_center_freq(hz)
                .map_err(|e| format!("failed to retune RTL-SDR to {hz} Hz: {e}"))?;
        }

        self.center_freq_hz = hz;
        Ok(())
    }

    fn is_realtime(&self) -> bool {
        true
    }

    fn source_capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            supports_sample_rate: true,
            sample_rates_hz: vec![1_024_000, 1_536_000, 2_048_000, 2_400_000],

            supports_gain_mode: true,
            supports_gain: true,

            // TODO: replace with device query later
            gain_values_db: vec![
                0.0, 0.9, 1.4, 2.7, 3.7, 7.7, 8.7, 12.5, 14.4, 15.7, 16.6, 19.7, 20.7, 22.9, 25.4,
                28.0, 29.7, 32.8, 33.8, 36.4, 37.2, 38.6, 40.2, 42.1, 43.4, 43.9, 44.5, 48.0, 49.6,
            ],

            supports_ppm_correction: true,
            ppm_min: -100,
            ppm_max: 100,

            supports_direct_sampling: true,
            direct_sampling_modes: vec![
                DirectSamplingMode::Off,
                DirectSamplingMode::I,
                DirectSamplingMode::Q,
            ],
            direct_sampling_freq_hz_max: 30_000_000,

            tuner_freq_hz_min: 24_000_000,
            tuner_freq_hz_max: 1_766_000_000,

            supports_tx_tune_test: false,
            supports_band_control: false,
            supports_fdx: false,
        }
    }

    fn source_control_state(&self) -> SourceControlState {
        SourceControlState {
            sample_rate_hz: self.sample_rate_hz as u32,
            gain_mode: self.gain_mode,
            gain_db: self.gain_db,
            ppm_correction: self.ppm_correction,
            direct_sampling: self.direct_sampling,
            // RTL-SDR has no transmit; report the default (TX unsupported).
            ..SourceControlState::default()
        }
    }

    fn set_gain_mode(&mut self, mode: GainMode) -> Result<(), String> {
        match mode {
            GainMode::Auto => {
                self.dev
                    .set_tuner_gain(TunerGain::Auto)
                    .map_err(|e| format!("failed to enable RTL-SDR automatic gain: {e}"))?;

                self.gain_mode = GainMode::Auto;
                Ok(())
            }

            GainMode::Manual => {
                let gain_tenths_db = (self.gain_db * 10.0).round() as i32;

                self.dev
                    .set_tuner_gain(TunerGain::Manual(gain_tenths_db))
                    .map_err(|e| {
                        format!(
                            "failed to enable RTL-SDR manual gain at {:.1} dB: {e}",
                            self.gain_db
                        )
                    })?;

                self.gain_mode = GainMode::Manual;
                Ok(())
            }
        }
    }

    fn set_gain_db(&mut self, gain_db: f32) -> Result<(), String> {
        let gain_tenths_db = (gain_db * 10.0).round() as i32;

        self.dev
            .set_tuner_gain(TunerGain::Manual(gain_tenths_db))
            .map_err(|e| format!("failed to set RTL-SDR gain to {:.1} dB: {e}", gain_db))?;

        self.gain_mode = GainMode::Manual;
        self.gain_db = gain_db;

        Ok(())
    }

    fn set_sample_rate(&mut self, sample_rate_hz: u32) -> Result<(), String> {
        self.dev.set_sample_rate(sample_rate_hz).map_err(|e| {
            format!("failed to set RTL-SDR sample rate to {sample_rate_hz} Hz: {e}")
        })?;

        self.sample_rate_hz = sample_rate_hz as f32;

        self.dev
            .reset_buffer()
            .map_err(|e| format!("failed to reset RTL-SDR buffer after sample rate change: {e}"))?;

        Ok(())
    }

    fn set_ppm_correction(&mut self, ppm: i32) -> Result<(), String> {
        self.dev
            .set_freq_correction(ppm)
            .map_err(|e| format!("failed to set RTL-SDR PPM correction to {ppm}: {e}"))?;

        self.ppm_correction = ppm;
        Ok(())
    }

    fn set_direct_sampling(&mut self, mode: DirectSamplingMode) -> Result<(), String> {
        let rtl_mode = match mode {
            DirectSamplingMode::Off => rtl_sdr_rs::DirectSampleMode::Off,
            DirectSamplingMode::I => rtl_sdr_rs::DirectSampleMode::On,
            DirectSamplingMode::Q => rtl_sdr_rs::DirectSampleMode::OnSwap,
        };

        self.dev
            .set_direct_sampling(rtl_mode)
            .map_err(|e| format!("failed to set RTL-SDR direct sampling to {mode:?}: {e}"))?;

        self.direct_sampling = mode;

        // Restore the tuner LO when leaving direct sampling so center_freq_hz
        // is accurate on the hardware again.
        if mode == DirectSamplingMode::Off {
            self.dev.set_center_freq(self.center_freq_hz).map_err(|e| {
                format!(
                    "failed to restore RTL-SDR LO to {} Hz after disabling direct sampling: {e}",
                    self.center_freq_hz
                )
            })?;
        }

        // DC spike note: direct sampling produces a large DC artifact at the
        // spectrum center (0 Hz offset). No DC blocker is applied; the spike
        // is expected and visible on the waterfall. A future IIR DC blocker
        // gated on direct_sampling != Off would suppress it.
        self.dev.reset_buffer().map_err(|e| {
            format!("failed to reset RTL-SDR buffer after direct sampling change: {e}")
        })?;

        Ok(())
    }
}
