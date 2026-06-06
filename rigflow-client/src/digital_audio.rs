//! Digital Audio Interface — Phase 1: virtual audio device lifecycle.
//!
//! Creates the two virtual audio endpoints that future digital-mode work
//! (WSJT-X / Fldigi / JS8Call) will route through, the same way Quisk exposes
//! dedicated endpoints:
//!
//! - **`RigflowDigitalOutput`** — a null **sink**.  Rigflow's RX audio will be
//!   played into it (Phase 2); its auto-created `RigflowDigitalOutput.monitor`
//!   **source** is what the digital app records from.
//! - **`RigflowDigitalInput`** — a virtual **source**.  The digital app's TX
//!   audio reaches it; Rigflow will read it into the TX path (Phase 3).
//!
//! This phase ONLY creates/destroys the devices — no audio routing, CAT, PTT,
//! or mode logic.  Devices are created when the client starts and unloaded
//! cleanly on exit; if a device of the same name already exists it is reused
//! (we don't create or later unload it).
//!
//! Implementation: we drive the **PipeWire / PulseAudio** layer through the
//! `pactl` CLI (present under both, via the Pulse compatibility layer), so no
//! extra crate dependency is needed.  Format is fixed at mono / 48000 Hz /
//! float32 to match Rigflow's audio pipeline — no rate negotiation.

use std::process::Command;

/// Sink Rigflow plays RX audio into (digital app records its `.monitor`).
pub const DIGITAL_OUTPUT_NAME: &str = "RigflowDigitalOutput";
/// Virtual source the digital app feeds TX audio into (Rigflow reads it).
pub const DIGITAL_INPUT_NAME: &str = "RigflowDigitalInput";

/// Fixed audio format for both endpoints (matches the Rigflow pipeline).
const RATE_HZ: &str = "48000";
const FORMAT: &str = "float32le";
const CHANNELS: &str = "1";
const CHANNEL_MAP: &str = "mono";

/// Owns the lifecycle of the virtual audio devices.  Drop unloads any modules
/// this process created (devices that already existed are left untouched).
pub struct DigitalAudio {
    /// `module-null-sink` index for the output sink, if we created it.
    output_module: Option<u32>,
    /// virtual-source module index for the input, if we created it.
    input_module: Option<u32>,
    output_available: bool,
    input_available: bool,
}

impl DigitalAudio {
    /// Ensure both virtual devices exist, creating any that are missing.
    /// Never panics — on any failure the corresponding device is reported
    /// `Unavailable` and the client keeps running.
    pub fn start() -> Self {
        let (output_available, output_module) = ensure_output_sink();
        let (input_available, input_module) = ensure_input_source();

        Self {
            output_module,
            input_module,
            output_available,
            input_available,
        }
    }

    pub fn output_available(&self) -> bool {
        self.output_available
    }

    pub fn input_available(&self) -> bool {
        self.input_available
    }
}

impl Drop for DigitalAudio {
    fn drop(&mut self) {
        // Only unload modules we created (leave pre-existing devices alone).
        if let Some(id) = self.output_module.take() {
            unload_module(id);
            log::info!("[digital-audio] digital output device destroyed ({DIGITAL_OUTPUT_NAME})");
        }
        if let Some(id) = self.input_module.take() {
            unload_module(id);
            log::info!("[digital-audio] digital input device destroyed ({DIGITAL_INPUT_NAME})");
        }
    }
}

/// Ensure the output **sink** exists.  Returns `(available, module_to_unload)`.
fn ensure_output_sink() -> (bool, Option<u32>) {
    if device_exists("sinks", DIGITAL_OUTPUT_NAME) {
        log::info!("[digital-audio] reusing existing sink {DIGITAL_OUTPUT_NAME}");
        return (true, None);
    }

    let args = [
        "load-module".to_string(),
        "module-null-sink".to_string(),
        format!("sink_name={DIGITAL_OUTPUT_NAME}"),
        format!("rate={RATE_HZ}"),
        format!("channels={CHANNELS}"),
        format!("format={FORMAT}"),
        format!("channel_map={CHANNEL_MAP}"),
        format!("sink_properties=device.description={DIGITAL_OUTPUT_NAME}"),
    ];
    match load_module(&args) {
        Some(id) => {
            log::info!(
                "[digital-audio] digital output device created ({DIGITAL_OUTPUT_NAME}, module {id})"
            );
            (true, Some(id))
        }
        None => {
            log::warn!("[digital-audio] failed to create {DIGITAL_OUTPUT_NAME}");
            (false, None)
        }
    }
}

/// Ensure the input **source** exists.  Returns `(available, module_to_unload)`.
///
/// Created as a PipeWire virtual source (`media.class=Audio/Source/Virtual`) so
/// it shows up under `pactl list short sources` with the expected name.
fn ensure_input_source() -> (bool, Option<u32>) {
    if device_exists("sources", DIGITAL_INPUT_NAME) {
        log::info!("[digital-audio] reusing existing source {DIGITAL_INPUT_NAME}");
        return (true, None);
    }

    let args = [
        "load-module".to_string(),
        "module-null-sink".to_string(),
        "media.class=Audio/Source/Virtual".to_string(),
        format!("sink_name={DIGITAL_INPUT_NAME}"),
        format!("rate={RATE_HZ}"),
        format!("channels={CHANNELS}"),
        format!("format={FORMAT}"),
        format!("channel_map={CHANNEL_MAP}"),
        format!("source_properties=device.description={DIGITAL_INPUT_NAME}"),
    ];
    match load_module(&args) {
        Some(id) => {
            log::info!(
                "[digital-audio] digital input device created ({DIGITAL_INPUT_NAME}, module {id})"
            );
            (true, Some(id))
        }
        None => {
            log::warn!("[digital-audio] failed to create {DIGITAL_INPUT_NAME}");
            (false, None)
        }
    }
}

/// Return true if a `pactl list short <kind>` entry with `name` exists.
/// `kind` is `"sinks"` or `"sources"`.
fn device_exists(kind: &str, name: &str) -> bool {
    let out = match Command::new("pactl").args(["list", "short", kind]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return false,
    };
    let text = String::from_utf8_lossy(&out);
    // Each line: "<id>\t<name>\t<driver>\t...".  Match the name column exactly.
    text.lines()
        .filter_map(|line| line.split('\t').nth(1))
        .any(|n| n == name)
}

/// Run `pactl load-module …`; on success return the new module index that
/// `pactl` prints on stdout.
fn load_module(args: &[String]) -> Option<u32> {
    let out = Command::new("pactl").args(args).output().ok()?;
    if !out.status.success() {
        log::debug!(
            "[digital-audio] pactl load-module failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

/// Best-effort `pactl unload-module <id>`.
fn unload_module(id: u32) {
    let _ = Command::new("pactl")
        .args(["unload-module", &id.to_string()])
        .status();
}
