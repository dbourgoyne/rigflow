//! Microphone capture + level metering (SSB Mic TX — Phase 1).
//!
//! Client-only and self-contained: it enumerates CPAL input devices, captures
//! mic audio continuously, converts to mono f32, applies a measurement gain,
//! and publishes peak level + clip state through a lock-free `MicShared` the UI
//! reads.  It does NOT touch the receive audio path, the TX chain, PTT, or the
//! network — no RF, no server interaction.  Designed to be reused by later
//! phases (monitor, SSB TX): `process_mono` is the single capture→measure point.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Clip threshold on the post-gain sample magnitude.
const CLIP_LEVEL: f32 = 0.99;

/// Lock-free mic state shared between the audio input callback (writer of
/// peak/clip) and the UI thread (reader; writer of gain).
#[derive(Debug)]
pub struct MicShared {
    /// Measurement gain (0.0–2.0), set by the UI.  f32 bits.
    gain_bits: AtomicU32,
    /// Peak |sample| since the UI last read it (fetch-max; UI swaps to 0).
    /// Non-negative, so the u32 bit pattern orders like the float.
    peak_bits: AtomicU32,
    /// Set by the callback when a post-gain sample clipped; UI reads + clears.
    clipped: AtomicBool,
}

impl Default for MicShared {
    fn default() -> Self {
        Self {
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            peak_bits: AtomicU32::new(0),
            clipped: AtomicBool::new(false),
        }
    }
}

impl MicShared {
    pub fn set_gain(&self, gain: f32) {
        self.gain_bits
            .store(gain.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }
    fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
    /// Take the peak level seen since the last read, resetting it to 0.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed))
    }
    /// Take the clip flag, clearing it.
    pub fn take_clipped(&self) -> bool {
        self.clipped.swap(false, Ordering::Relaxed)
    }
    fn report(&self, peak: f32, clipped: bool) {
        if peak > 0.0 {
            self.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
        }
        if clipped {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }
}

/// A running microphone capture stream.  Holding it keeps capture alive; drop
/// to stop.  `requested` is what the operator asked for ("" = default);
/// `device_name` is the device actually opened; `fell_back` is true when the
/// requested device was missing and we used the default.
pub struct MicCapture {
    _stream: cpal::Stream,
    pub requested: String,
    pub device_name: String,
    pub fell_back: bool,
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        log::debug!("[mic] microphone stream stopped ({})", self.device_name);
    }
}

/// Enumerate available input device names (best-effort).
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Start capturing from `requested` (empty = system default).  Falls back to the
/// default device if the named one is missing (`fell_back = true`).
pub fn start_capture(shared: Arc<MicShared>, requested: &str) -> Result<MicCapture, String> {
    let host = cpal::default_host();

    let (device, fell_back) = if requested.is_empty() {
        (
            host.default_input_device()
                .ok_or_else(|| "no default input device".to_string())?,
            false,
        )
    } else {
        match host
            .input_devices()
            .ok()
            .and_then(|mut ds| ds.find(|d| d.name().map(|n| n == requested).unwrap_or(false)))
        {
            Some(d) => (d, false),
            None => (
                host.default_input_device()
                    .ok_or_else(|| "no default input device".to_string())?,
                true,
            ),
        }
    };

    let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
    let supported = device
        .default_input_config()
        .map_err(|e| format!("default input config: {e}"))?;
    let sample_format = supported.sample_format();
    let channels = supported.channels().max(1) as usize;
    let config: cpal::StreamConfig = supported.into();

    let err_fn = |e| log::error!("[mic] input stream error: {e}");
    let cb = Arc::clone(&shared);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| process_f32(data, channels, &cb),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| process_i16(data, channels, &cb),
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| process_u16(data, channels, &cb),
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported input sample format {other:?}")),
    }
    .map_err(|e| format!("build input stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("play input stream: {e}"))?;

    log::info!("[mic] microphone device selected: {device_name} (requested {requested:?})");
    log::debug!("[mic] microphone stream started");

    Ok(MicCapture {
        _stream: stream,
        requested: requested.to_string(),
        device_name,
        fell_back,
    })
}

// ── Per-format capture: downmix to mono, apply gain, measure peak + clip ─────

fn process_f32(data: &[f32], channels: usize, shared: &MicShared) {
    process_mono(
        data.chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32),
        shared,
    );
}

fn process_i16(data: &[i16], channels: usize, shared: &MicShared) {
    process_mono(
        data.chunks(channels).map(|frame| {
            frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / frame.len() as f32
        }),
        shared,
    );
}

fn process_u16(data: &[u16], channels: usize, shared: &MicShared) {
    process_mono(
        data.chunks(channels).map(|frame| {
            frame
                .iter()
                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                .sum::<f32>()
                / frame.len() as f32
        }),
        shared,
    );
}

/// Apply the measurement gain to a mono f32 stream and report peak + clip.
/// This is the reusable capture→measure point for future monitor / SSB-TX work.
fn process_mono(samples: impl Iterator<Item = f32>, shared: &MicShared) {
    let gain = shared.gain();
    let mut peak = 0.0f32;
    let mut clipped = false;
    for s in samples {
        let v = s * gain;
        let mag = v.abs();
        if mag > peak {
            peak = mag;
        }
        if mag >= CLIP_LEVEL {
            clipped = true;
        }
    }
    if clipped {
        log::debug!("[mic] clipping detected (peak {peak:.3})");
    }
    shared.report(peak, clipped);
}
