//! Digital Audio Interface — virtual audio device lifecycle.
//!
//! Creates the virtual audio endpoints that digital-mode software (WSJT-X /
//! Fldigi / JS8Call) routes through, named from the **external app's** point of
//! view.  Device model (Linux PipeWire / PulseAudio):
//!
//! ```text
//! RX  (Rigflow → app):  Rigflow writes fixed-level RX audio to the
//!     RigflowDigitalOutput        sink
//!   apps record from
//!     RigflowDigitalRX            source  (remapped from RigflowDigitalOutput.monitor)
//!
//! TX  (app → Rigflow):  the app plays TX audio into
//!     RigflowDigitalInput         sink
//!   Rigflow reads from
//!     RigflowDigitalInput.monitor source
//! ```
//!
//! So WSJT-X is configured as: **Input = RigflowDigitalRX**, **Output =
//! RigflowDigitalInput**.  Apps can't select monitor sources directly, which is
//! why `RigflowDigitalRX` is a remapped source rather than the raw monitor.
//!
//! Devices are created when the client starts and unloaded cleanly on exit; if
//! one already exists (e.g. a previous crash) it is reused — startup never fails
//! just because a device is present.  We drive the PipeWire / PulseAudio layer
//! through the `pactl` CLI (works under both via the Pulse compat layer), so no
//! extra crate dependency is needed.  Format is fixed mono / 48000 Hz / float32.

use std::process::Command;

/// Sink Rigflow plays RX audio into (the remap master).
pub const DIGITAL_OUTPUT_NAME: &str = "RigflowDigitalOutput";
/// Monitor of the output sink — master for the RX remap source.
const DIGITAL_OUTPUT_MONITOR: &str = "RigflowDigitalOutput.monitor";
/// Source digital apps record from (remapped from the output monitor).
pub const DIGITAL_RX_NAME: &str = "RigflowDigitalRX";
/// Sink digital apps play TX audio into (Rigflow reads its `.monitor`).
pub const DIGITAL_INPUT_NAME: &str = "RigflowDigitalInput";

/// Fixed audio format for the endpoints (matches the Rigflow pipeline).
const RATE_HZ: &str = "48000";
const FORMAT: &str = "float32le";
const CHANNELS: &str = "1";
const CHANNEL_MAP: &str = "mono";

/// Owns the lifecycle of the virtual audio devices.  Drop unloads any modules
/// this process created (devices that already existed are left untouched).
pub struct DigitalAudio {
    /// `module-null-sink` for `RigflowDigitalOutput`, if we created it.
    output_module: Option<u32>,
    /// `module-remap-source` for `RigflowDigitalRX`, if we created it.
    rx_module: Option<u32>,
    /// `module-null-sink` for `RigflowDigitalInput`, if we created it.
    input_module: Option<u32>,
    output_available: bool,
    rx_available: bool,
    input_available: bool,
}

impl DigitalAudio {
    /// Ensure all virtual devices exist, creating any that are missing.  Never
    /// panics — on any failure the affected device is reported unavailable and
    /// the client keeps running.  Order matters: the output sink (and its
    /// monitor) must exist before the RX remap source that masters off it.
    pub fn start() -> Self {
        let (output_available, output_module) = ensure_output_sink();
        let (rx_available, rx_module) = ensure_rx_remap_source();
        let (input_available, input_module) = ensure_input_sink();

        Self {
            output_module,
            rx_module,
            input_module,
            output_available,
            rx_available,
            input_available,
        }
    }

    /// `RigflowDigitalRX` source (what digital apps record from) is available.
    pub fn rx_available(&self) -> bool {
        self.rx_available
    }

    /// `RigflowDigitalInput` sink (what digital apps play TX into) is available.
    pub fn input_available(&self) -> bool {
        self.input_available
    }

    /// `RigflowDigitalOutput` sink (internal RX target) is available.
    pub fn output_available(&self) -> bool {
        self.output_available
    }
}

impl Drop for DigitalAudio {
    fn drop(&mut self) {
        // Unload in reverse dependency order; only modules we created.
        if let Some(id) = self.input_module.take() {
            log::info!(
                "[digital-audio] unloading {DIGITAL_INPUT_NAME} (module {id}): {}",
                unload_status(id)
            );
        }
        if let Some(id) = self.rx_module.take() {
            log::info!(
                "[digital-audio] unloading {DIGITAL_RX_NAME} (module {id}): {}",
                unload_status(id)
            );
        }
        if let Some(id) = self.output_module.take() {
            log::info!(
                "[digital-audio] unloading {DIGITAL_OUTPUT_NAME} (module {id}): {}",
                unload_status(id)
            );
        }
    }
}

/// Ensure the `RigflowDigitalOutput` **sink** exists, then force it to unity /
/// unmuted so the digital RX level is fixed regardless of the speaker volume.
fn ensure_output_sink() -> (bool, Option<u32>) {
    let module = if device_exists("sinks", DIGITAL_OUTPUT_NAME) {
        log::info!("[digital-audio] reusing existing sink {DIGITAL_OUTPUT_NAME}");
        None
    } else {
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
                log::info!("[digital-audio] created sink {DIGITAL_OUTPUT_NAME} (module {id})");
                Some(id)
            }
            None => {
                log::warn!("[digital-audio] failed to create sink {DIGITAL_OUTPUT_NAME}");
                return (false, None);
            }
        }
    };

    // Defensive: keep the digital RX path at a fixed unity level, independent of
    // any per-sink volume/mute a session manager might apply.
    let _ = Command::new("pactl")
        .args(["set-sink-volume", DIGITAL_OUTPUT_NAME, "100%"])
        .status();
    let _ = Command::new("pactl")
        .args(["set-sink-mute", DIGITAL_OUTPUT_NAME, "0"])
        .status();

    (true, module)
}

/// Ensure the `RigflowDigitalRX` **remap source** (master =
/// `RigflowDigitalOutput.monitor`) exists.  This is what apps record from,
/// because most apps won't list monitor sources directly.
fn ensure_rx_remap_source() -> (bool, Option<u32>) {
    if device_exists("sources", DIGITAL_RX_NAME) {
        log::info!("[digital-audio] reusing existing source {DIGITAL_RX_NAME}");
        return (true, None);
    }

    let args = [
        "load-module".to_string(),
        "module-remap-source".to_string(),
        format!("master={DIGITAL_OUTPUT_MONITOR}"),
        format!("source_name={DIGITAL_RX_NAME}"),
        format!("source_properties=device.description={DIGITAL_RX_NAME}"),
        format!("channels={CHANNELS}"),
        format!("channel_map={CHANNEL_MAP}"),
    ];
    match load_module(&args) {
        Some(id) => {
            log::info!("[digital-audio] created source {DIGITAL_RX_NAME} (module {id})");
            (true, Some(id))
        }
        None => {
            log::warn!("[digital-audio] failed to create source {DIGITAL_RX_NAME}");
            (false, None)
        }
    }
}

/// Ensure the `RigflowDigitalInput` **sink** exists.  Apps play TX audio into
/// it; Rigflow reads `RigflowDigitalInput.monitor` (TX routing is a later
/// phase).  A null sink auto-creates the matching monitor source.
fn ensure_input_sink() -> (bool, Option<u32>) {
    if device_exists("sinks", DIGITAL_INPUT_NAME) {
        log::info!("[digital-audio] reusing existing sink {DIGITAL_INPUT_NAME}");
        return (true, None);
    }

    let args = [
        "load-module".to_string(),
        "module-null-sink".to_string(),
        format!("sink_name={DIGITAL_INPUT_NAME}"),
        format!("rate={RATE_HZ}"),
        format!("channels={CHANNELS}"),
        format!("format={FORMAT}"),
        format!("channel_map={CHANNEL_MAP}"),
        format!("sink_properties=device.description={DIGITAL_INPUT_NAME}"),
    ];
    match load_module(&args) {
        Some(id) => {
            log::info!("[digital-audio] created sink {DIGITAL_INPUT_NAME} (module {id})");
            (true, Some(id))
        }
        None => {
            log::warn!("[digital-audio] failed to create sink {DIGITAL_INPUT_NAME}");
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

/// Best-effort `pactl unload-module <id>`; returns a short status for logging.
fn unload_status(id: u32) -> &'static str {
    match Command::new("pactl")
        .args(["unload-module", &id.to_string()])
        .status()
    {
        Ok(s) if s.success() => "ok",
        _ => "failed",
    }
}
