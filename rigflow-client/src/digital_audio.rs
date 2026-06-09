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

use std::fs::File;
use std::process::{Child, Command, Stdio};

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
    /// Silent playback that holds `RigflowDigitalInput` active so its monitor
    /// never suspends (keeps TX capture warm at key-down).  Killed on Drop.
    input_keepalive: Option<Child>,
    output_available: bool,
    rx_available: bool,
    input_available: bool,
    /// Failure reason captured when an endpoint could not be created (pactl
    /// missing, PipeWire down, stderr from `load-module`).  `None` when created
    /// or reused successfully.  Surfaced in the client's Problems area.
    output_reason: Option<String>,
    rx_reason: Option<String>,
    input_reason: Option<String>,
}

impl DigitalAudio {
    /// Ensure all virtual devices exist, creating any that are missing.  Never
    /// panics — on any failure the affected device is reported unavailable and
    /// the client keeps running.  Order matters: the output sink (and its
    /// monitor) must exist before the RX remap source that masters off it.
    pub fn start() -> Self {
        let (output_available, output_module, output_reason) = ensure_output_sink();
        let (rx_available, rx_module, rx_reason) = ensure_rx_remap_source();
        let (input_available, input_module, input_reason) = ensure_input_sink();

        // Hold the input sink permanently active with a silent playback stream so
        // PipeWire/Pulse never suspends it on idle.  A monitor capture alone does
        // not keep a sink busy, so without this the monitor sleeps between overs
        // and the TX capture (parec) starves for ~1 s at the next key-down.  The
        // silence mixes harmlessly with the app's TX audio (silence + FT8 = FT8).
        let input_keepalive = if input_available {
            match spawn_input_keepalive() {
                Some(c) => {
                    log::info!("[digital-audio] {DIGITAL_INPUT_NAME} keep-alive started");
                    Some(c)
                }
                None => {
                    log::warn!(
                        "[digital-audio] {DIGITAL_INPUT_NAME} keep-alive unavailable (no pacat/pw-cat)"
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            output_module,
            rx_module,
            input_module,
            input_keepalive,
            output_available,
            rx_available,
            input_available,
            output_reason,
            rx_reason,
            input_reason,
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

    /// Failure reason for each endpoint (`None` when available).  Mirrors the
    /// `*_available()` accessors so the UI can show *why* a device is missing.
    pub fn output_reason(&self) -> Option<String> {
        self.output_reason.clone()
    }

    pub fn rx_reason(&self) -> Option<String> {
        self.rx_reason.clone()
    }

    pub fn input_reason(&self) -> Option<String> {
        self.input_reason.clone()
    }
}

impl Drop for DigitalAudio {
    fn drop(&mut self) {
        // Stop the silence keep-alive before unloading its sink.
        if let Some(mut c) = self.input_keepalive.take() {
            let _ = c.kill();
            let _ = c.wait();
            log::info!("[digital-audio] {DIGITAL_INPUT_NAME} keep-alive stopped");
        }
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
fn ensure_output_sink() -> (bool, Option<u32>, Option<String>) {
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
            Ok(id) => {
                log::info!("[digital-audio] created sink {DIGITAL_OUTPUT_NAME} (module {id})");
                Some(id)
            }
            Err(reason) => {
                log::warn!("[digital-audio] failed to create sink {DIGITAL_OUTPUT_NAME}: {reason}");
                return (false, None, Some(reason));
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

    (true, module, None)
}

/// Ensure the `RigflowDigitalRX` **remap source** (master =
/// `RigflowDigitalOutput.monitor`) exists.  This is what apps record from,
/// because most apps won't list monitor sources directly.
fn ensure_rx_remap_source() -> (bool, Option<u32>, Option<String>) {
    if device_exists("sources", DIGITAL_RX_NAME) {
        log::info!("[digital-audio] reusing existing source {DIGITAL_RX_NAME}");
        return (true, None, None);
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
        Ok(id) => {
            log::info!("[digital-audio] created source {DIGITAL_RX_NAME} (module {id})");
            (true, Some(id), None)
        }
        Err(reason) => {
            log::warn!("[digital-audio] failed to create source {DIGITAL_RX_NAME}: {reason}");
            (false, None, Some(reason))
        }
    }
}

/// Ensure the `RigflowDigitalInput` **sink** exists.  Apps play TX audio into
/// it; Rigflow reads `RigflowDigitalInput.monitor` (TX routing is a later
/// phase).  A null sink auto-creates the matching monitor source.
fn ensure_input_sink() -> (bool, Option<u32>, Option<String>) {
    if device_exists("sinks", DIGITAL_INPUT_NAME) {
        log::info!("[digital-audio] reusing existing sink {DIGITAL_INPUT_NAME}");
        return (true, None, None);
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
        Ok(id) => {
            log::info!("[digital-audio] created sink {DIGITAL_INPUT_NAME} (module {id})");
            (true, Some(id), None)
        }
        Err(reason) => {
            log::warn!("[digital-audio] failed to create sink {DIGITAL_INPUT_NAME}: {reason}");
            (false, None, Some(reason))
        }
    }
}

/// Spawn a silent playback into `RigflowDigitalInput` to keep the sink active
/// (so its monitor never suspends).  Reads zeros from `/dev/zero` — pacat paces
/// reads to the sink rate, so this is real-time silence, not a busy loop.  Tries
/// `pacat` (Pulse/PipeWire-pulse) then `pw-cat` (PipeWire-native).
fn spawn_input_keepalive() -> Option<Child> {
    let zero = File::open("/dev/zero").ok()?;
    let pacat = Command::new("pacat")
        .args([
            "--playback",
            "--raw",
            &format!("--device={DIGITAL_INPUT_NAME}"),
            &format!("--rate={RATE_HZ}"),
            &format!("--channels={CHANNELS}"),
            &format!("--format={FORMAT}"),
        ])
        .stdin(Stdio::from(zero))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(c) = pacat {
        return Some(c);
    }

    let zero = File::open("/dev/zero").ok()?;
    Command::new("pw-cat")
        .args([
            "--playback",
            "--raw",
            "--target",
            DIGITAL_INPUT_NAME,
            "--rate",
            RATE_HZ,
            "--channels",
            CHANNELS,
            "--format",
            "f32",
            "-",
        ])
        .stdin(Stdio::from(zero))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
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
/// `pactl` prints on stdout, otherwise the failure reason (so callers can
/// surface *why* the endpoint is unavailable rather than just a red dot).
fn load_module(args: &[String]) -> Result<u32, String> {
    let out = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| format!("pactl not available: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        log::debug!("[digital-audio] pactl load-module failed: {stderr}");
        return Err(if stderr.is_empty() {
            "pactl load-module failed".to_string()
        } else {
            stderr
        });
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "pactl returned no module id".to_string())
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
