use std::path::PathBuf;

use clap::Parser;
use rigflow_protocol::radio_control::RadioInfo;

/// Headless diagnostic probe for rigflow-server.
///
/// Connects like the real client, receives the server's UDP audio stream,
/// optionally records it to a WAV, and reports transport/dropout statistics —
/// with no GUI and no audio device. Run it on the server box over loopback
/// (`--server 127.0.0.1:9000`) to take the network out of the picture and check
/// whether audio breaks are server-side.
///
/// The server is single-client: disconnect the GUI client before running this.
#[derive(Parser, Debug)]
#[command(name = "rigflow-probe", version, about)]
pub struct Args {
    /// Server control endpoint, `host:port` (WebSocket). Port defaults to 9000.
    #[arg(long, default_value = "127.0.0.1:9000")]
    pub server: String,

    /// Server UDP registration port (defaults to 9001).
    #[arg(long)]
    pub udp_port: Option<u16>,

    /// Radio to acquire: exact id, a substring of the id/display name, or a
    /// numeric index into the listed radios. If omitted, the list is printed
    /// and the first radio is used.
    #[arg(long)]
    pub radio: Option<String>,

    /// RF center frequency in Hz. Defaults to `--target-hz` if only that is given.
    #[arg(long)]
    pub center_hz: Option<u64>,

    /// Tuned/dial frequency in Hz. Defaults to `--center-hz` if only that is given.
    #[arg(long)]
    pub target_hz: Option<u64>,

    /// Demod mode (wfm|nfm|usb|lsb|am|cwu|cwl|dgt_u). Left as-is if omitted.
    #[arg(long)]
    pub mode: Option<String>,

    /// Source sample rate / HL2 bandwidth in Hz (e.g. 48000, 384000). Re-run with
    /// different values to compare bandwidths.
    #[arg(long)]
    pub sample_rate: Option<u32>,

    /// Capture window in seconds.
    #[arg(long, default_value_t = 15)]
    pub duration: u64,

    /// Jitter buffer target depth in ms (playout start / steady-state latency).
    /// Matches the real client's default.
    #[arg(long, default_value_t = 60)]
    pub jitter_target_ms: u32,

    /// Jitter buffer max depth in ms (overflow ceiling). Matches the real client's
    /// default. Raise it (e.g. 1000–2000) to absorb bursty WiFi delivery — fewer
    /// overflow drops/resyncs at the cost of added latency.
    #[arg(long, default_value_t = 500)]
    pub jitter_max_ms: u32,

    /// Write the received 48 kHz mono audio to this WAV path.
    #[arg(long)]
    pub wav: Option<PathBuf>,

    /// Waterfall frame rate in Hz; 0 disables the waterfall stream entirely. Use to
    /// A/B test whether waterfall traffic contends with audio (e.g. over WiFi). Omit
    /// to leave the server default (20 Hz). Server clamps to 0–30.
    #[arg(long)]
    pub waterfall_rate: Option<f32>,
}

impl Args {
    /// Jitter buffer `(target_samples, max_samples)` at 48 kHz, clamped to the
    /// `JitterBuffer` invariants (target ≥ one packet, max ≥ target).
    pub fn jitter_samples(&self, packet_samples: usize) -> (usize, usize) {
        let target = (self.jitter_target_ms as usize * 48).max(packet_samples);
        let max = (self.jitter_max_ms as usize * 48).max(target);
        (target, max)
    }
}

impl Args {
    /// Split `--server` into `(host, ws_port)`, defaulting the port to 9000.
    pub fn server_host_port(&self) -> (String, u16) {
        match self.server.rsplit_once(':') {
            Some((host, port)) => {
                let port = port.parse().unwrap_or(9000);
                (host.to_string(), port)
            }
            None => (self.server.clone(), 9000),
        }
    }

    /// UDP registration port (explicit, else default 9001).
    pub fn udp_reg_port(&self) -> u16 {
        self.udp_port.unwrap_or(9001)
    }

    /// Resolve the initial `(center_hz, target_hz)` to send in `AcquireRadio`,
    /// falling back across the two flags and finally to the snapshot default.
    pub fn initial_freqs(&self, snapshot_default: u64) -> (u64, u64) {
        match (self.center_hz, self.target_hz) {
            (Some(c), Some(t)) => (c, t),
            (Some(c), None) => (c, c),
            (None, Some(t)) => (t, t),
            (None, None) => (snapshot_default, snapshot_default),
        }
    }
}

/// Pretty-print the radio list (id, kind, state) so the operator can pick one.
pub fn print_radio_list(radios: &[RadioInfo]) {
    println!("Available radios:");
    for (i, r) in radios.iter().enumerate() {
        println!(
            "  [{i}] {:<20} {:<24} {:?}{}",
            r.id.0,
            r.display_name,
            r.state,
            if r.is_leased { "  (leased)" } else { "" },
        );
    }
}

/// Resolve `--radio` against the listed radios.
///
/// Matching order: numeric index, then exact id, then case-insensitive substring
/// of the id or display name. Returns the index into `radios`, or an error.
pub fn resolve_radio(radios: &[RadioInfo], selector: Option<&str>) -> Result<usize, String> {
    if radios.is_empty() {
        return Err("server reported no radios".to_string());
    }

    let Some(sel) = selector else {
        // No selector: caller prints the list; default to the first radio.
        return Ok(0);
    };

    if let Ok(idx) = sel.parse::<usize>() {
        if idx < radios.len() {
            return Ok(idx);
        }
        return Err(format!(
            "radio index {idx} out of range (0..{})",
            radios.len()
        ));
    }

    if let Some(idx) = radios.iter().position(|r| r.id.0 == sel) {
        return Ok(idx);
    }

    let needle = sel.to_lowercase();
    if let Some(idx) = radios.iter().position(|r| {
        r.id.0.to_lowercase().contains(&needle) || r.display_name.to_lowercase().contains(&needle)
    }) {
        return Ok(idx);
    }

    Err(format!("no radio matched \"{sel}\""))
}
