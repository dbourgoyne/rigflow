//! Microphone capture + level metering (SSB Mic TX — Phase 1).
//!
//! Client-only and self-contained: it enumerates CPAL input devices, captures
//! mic audio continuously, converts to mono f32, applies a measurement gain,
//! and publishes peak level + clip state through a lock-free `MicShared` the UI
//! reads.  It does NOT touch the receive audio path, the TX chain, PTT, or the
//! network — no RF, no server interaction.  Designed to be reused by later
//! phases (monitor, SSB TX): `process_mono` is the single capture→measure point.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Clip threshold on the post-gain sample magnitude.
const CLIP_LEVEL: f32 = 0.99;

/// Mic audio transport rate (mono f32 to the server for SSB TX).
const TX_RATE_HZ: u32 = 48_000;

/// Cap on the outbound mic-TX ring (~0.5 s at 48 kHz); drop oldest on overflow.
const TX_RING_MAX: usize = 24_000;

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
    /// True while SSB mic TX is keyed: the callback buffers gained 48 kHz mono
    /// audio into `tx_ring` for the media thread to send to the server.
    tx_streaming: AtomicBool,
    /// True when an *external* source (e.g. the digital TX router capturing
    /// WSJT-X audio) is feeding `tx_ring` directly.  The mic callback then
    /// suppresses its own push so the ring has a single producer — otherwise the
    /// always-on capture and the external source both fill it (≈2× the consume
    /// rate), pinning the server queue and dropping ~half the samples.
    external_tx_source: AtomicBool,
    /// Outbound mic-TX audio ring (48 kHz mono f32).
    tx_ring: Mutex<VecDeque<f32>>,
}

impl Default for MicShared {
    fn default() -> Self {
        Self {
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            peak_bits: AtomicU32::new(0),
            clipped: AtomicBool::new(false),
            tx_streaming: AtomicBool::new(false),
            external_tx_source: AtomicBool::new(false),
            tx_ring: Mutex::new(VecDeque::new()),
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

    /// Enable/disable buffering of mic audio for SSB TX.  Clears the ring on a
    /// transition so stale audio doesn't leak into the next over.
    pub fn set_tx_streaming(&self, on: bool) {
        let was = self.tx_streaming.swap(on, Ordering::Relaxed);
        if was != on {
            if let Ok(mut r) = self.tx_ring.lock() {
                r.clear();
            }
        }
    }
    pub fn tx_streaming(&self) -> bool {
        self.tx_streaming.load(Ordering::Relaxed)
    }
    /// Mark/unmark that an external source owns the TX ring (digital TX router).
    /// While set, the mic capture callback does not push, leaving the external
    /// source as the ring's sole producer.
    pub fn set_external_tx_source(&self, on: bool) {
        self.external_tx_source.store(on, Ordering::Relaxed);
    }
    pub fn external_tx_source(&self) -> bool {
        self.external_tx_source.load(Ordering::Relaxed)
    }
    /// Append captured 48 kHz mono samples (callback side); drop oldest on
    /// overflow.  No-op unless streaming.  Also used by the digital TX router
    /// to inject WSJT-X audio into the same mic-TX ring.
    pub fn push_tx(&self, samples: &[f32]) {
        if !self.tx_streaming.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.tx_ring.lock() {
            r.extend(samples.iter().copied());
            if r.len() > TX_RING_MAX {
                let drop = r.len() - TX_RING_MAX;
                r.drain(..drop);
            }
        }
    }
    /// Drain all buffered mic-TX samples (media-thread side).
    pub fn drain_tx(&self) -> Vec<f32> {
        match self.tx_ring.lock() {
            Ok(mut r) => r.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// A running microphone capture stream.  Holding it keeps capture alive; drop
/// to stop.  `device_name` is the device actually opened; `fell_back` is true
/// when the requested device was missing and we used the default.
pub struct MicCapture {
    _stream: cpal::Stream,
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
    // Prefer a 48 kHz config (avoids resampling); fall back to the device
    // default (resampled to 48 kHz for TX).
    let supported = pick_input_config(&device)?;
    let sample_format = supported.sample_format();
    let channels = supported.channels().max(1) as usize;
    let config: cpal::StreamConfig = supported.into();
    let in_rate = config.sample_rate.0 as f32;

    let err_fn = |e| log::error!("[mic] input stream error: {e}");
    let mut proc = MicProc::new(Arc::clone(&shared), channels, in_rate);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| proc.feed_f32(data),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| proc.feed_i16(data),
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| proc.feed_u16(data),
            err_fn,
            None,
        ),
        cpal::SampleFormat::U8 => device.build_input_stream(
            &config,
            move |data: &[u8], _| proc.feed_u8(data),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I8 => device.build_input_stream(
            &config,
            move |data: &[i8], _| proc.feed_i8(data),
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported input sample format {other:?}")),
    }
    .map_err(|e| format!("build input stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("play input stream: {e}"))?;

    log::info!(
        "[mic] microphone device selected: {device_name} @ {in_rate:.0} Hz (requested {requested:?})"
    );
    log::debug!("[mic] microphone stream started");

    Ok(MicCapture {
        _stream: stream,
        device_name,
        fell_back,
    })
}

/// Choose an input config, preferring one at 48 kHz so no resampling is needed.
fn pick_input_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, String> {
    if let Ok(ranges) = device.supported_input_configs() {
        for r in ranges {
            if r.min_sample_rate().0 <= TX_RATE_HZ && TX_RATE_HZ <= r.max_sample_rate().0 {
                return Ok(r.with_sample_rate(cpal::SampleRate(TX_RATE_HZ)));
            }
        }
    }
    device
        .default_input_config()
        .map_err(|e| format!("default input config: {e}"))
}

// ── Capture processing: downmix → gain → measure → (resample → buffer) ───────

/// Per-stream capture state (lives in the input callback).  Downmixes to mono,
/// applies the measurement gain, reports peak/clip, and — while streaming —
/// resamples to 48 kHz and buffers the gained audio for SSB TX.
struct MicProc {
    shared: Arc<MicShared>,
    channels: usize,
    in_rate: f32,
    mono: Vec<f32>,
    resampled: Vec<f32>,
    // Linear-resampler state (only used when in_rate != 48 kHz).
    pos: f32,
    prev: f32,
}

impl MicProc {
    fn new(shared: Arc<MicShared>, channels: usize, in_rate: f32) -> Self {
        Self {
            shared,
            channels,
            in_rate,
            mono: Vec::new(),
            resampled: Vec::new(),
            pos: 0.0,
            prev: 0.0,
        }
    }

    fn feed_f32(&mut self, data: &[f32]) {
        self.mono.clear();
        for frame in data.chunks(self.channels) {
            self.mono
                .push(frame.iter().sum::<f32>() / frame.len() as f32);
        }
        self.process();
    }

    fn feed_i16(&mut self, data: &[i16]) {
        self.mono.clear();
        for frame in data.chunks(self.channels) {
            let m = frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / frame.len() as f32;
            self.mono.push(m);
        }
        self.process();
    }

    fn feed_u16(&mut self, data: &[u16]) {
        self.mono.clear();
        for frame in data.chunks(self.channels) {
            let m = frame
                .iter()
                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                .sum::<f32>()
                / frame.len() as f32;
            self.mono.push(m);
        }
        self.process();
    }

    fn feed_u8(&mut self, data: &[u8]) {
        self.mono.clear();
        for frame in data.chunks(self.channels) {
            let m = frame
                .iter()
                .map(|&s| (s as f32 - 128.0) / 128.0)
                .sum::<f32>()
                / frame.len() as f32;
            self.mono.push(m);
        }
        self.process();
    }

    fn feed_i8(&mut self, data: &[i8]) {
        self.mono.clear();
        for frame in data.chunks(self.channels) {
            let m = frame.iter().map(|&s| s as f32 / 128.0).sum::<f32>() / frame.len() as f32;
            self.mono.push(m);
        }
        self.process();
    }

    /// Gain, measure peak/clip, and buffer 48 kHz mono audio while streaming.
    /// (`self.mono` is the device-rate mono block.)
    fn process(&mut self) {
        let gain = self.shared.gain();
        let mut peak = 0.0f32;
        let mut clipped = false;
        for s in &mut self.mono {
            *s *= gain;
            let mag = s.abs();
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
        self.shared.report(peak, clipped);

        // Push mic audio only when keyed AND no external source owns the ring
        // (the digital TX router feeds it directly while transmitting WSJT-X).
        if self.shared.tx_streaming() && !self.shared.external_tx_source() {
            if (self.in_rate - TX_RATE_HZ as f32).abs() < 1.0 {
                self.shared.push_tx(&self.mono);
            } else {
                self.resample_into_48k();
                let out = std::mem::take(&mut self.resampled);
                self.shared.push_tx(&out);
                self.resampled = out;
            }
        }
    }

    /// Linear-resample `self.mono` (at `in_rate`) to 48 kHz into `self.resampled`,
    /// carrying fractional phase + the last sample across blocks.  Adequate for
    /// voice SSB (the 48 kHz path above avoids this entirely on most devices).
    fn resample_into_48k(&mut self) {
        let step = self.in_rate / TX_RATE_HZ as f32; // input advance per output sample
        let maxk = self.mono.len();
        let prev = self.prev;
        let mut pos = self.pos;
        let mut out = std::mem::take(&mut self.resampled);
        out.clear();
        // Virtual input V: V[0] = prev, V[k] = mono[k-1].  Interpolate while a
        // full segment [k, k+1] is available (k+1 <= maxk).
        while (pos.floor() as usize) + 1 <= maxk {
            let f = pos.floor();
            let k = f as usize;
            let frac = pos - f;
            let vk = if k == 0 { prev } else { self.mono[k - 1] };
            let vk1 = self.mono[k];
            out.push(vk + (vk1 - vk) * frac);
            pos += step;
        }
        pos -= maxk as f32;
        self.pos = pos.max(0.0);
        if maxk > 0 {
            self.prev = self.mono[maxk - 1];
        }
        self.resampled = out;
    }
}
