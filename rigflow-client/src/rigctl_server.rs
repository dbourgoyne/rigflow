//! Minimal Hamlib `rigctld`-compatible CAT server (for WSJT-X et al.).
//!
//! Runs in the **client** (not the server): the client owns the radio lease and
//! already tracks frequency / mode / TX, and WSJT-X runs on the same machine and
//! connects to `127.0.0.1:4532`.  We implement just enough of the rigctl wire
//! protocol for WSJT-X to read/set frequency & mode and key PTT.
//!
//! ```text
//!   WSJT-X  ──TCP 127.0.0.1:4532──▶  RigctlServer (here)
//!                                       │ reads UiState (freq/mode)
//!                                       │ sends ControlCommand (set freq/mode/PTT)
//!                                       ▼
//!                                    rest of the client → server → radio
//! ```
//!
//! WSJT-X config: Rig = "Hamlib NET rigctl", Host 127.0.0.1, Port 4532, PTT = CAT.
//!
//! All CAT logic is isolated here; it talks to the rest of the client only
//! through the existing `UiState` snapshot and the `ControlCommand` channel — no
//! duplicate frequency/mode state is introduced (PTT keeps a small commanded
//! flag, since the client has no single readback of "transmitting").

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedSender;

use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_protocol::radio_control::ClientRadioMessage;

use crate::net::control::ControlCommand;
use crate::ui::freq_limits::{active_freq_limits, clamp_center};
use crate::ui::state::UiState;

/// Default Hamlib NET rigctl port (WSJT-X default).
pub const DEFAULT_RIGCTL_PORT: u16 = 4532;

/// Shared handles the connection handlers use to read state and issue commands.
struct RigctlShared {
    ui_state: Arc<Mutex<UiState>>,
    cmd_tx: UnboundedSender<ControlCommand>,
    /// The exact rigctl mode string last set over CAT (e.g. `PKTUSB`).  Echoed
    /// back by `m` so a client's read-after-write matches — Rigflow has no
    /// distinct data/packet mode (`PKTUSB`/`USB` both map to `DemodMode::Usb`),
    /// and reporting `USB` after the client set `PKTUSB` makes WSJT-X loop
    /// forever trying to "fix" the mode.  Cleared/ignored once the underlying
    /// demod is changed elsewhere (UI).
    last_cat_mode: Mutex<Option<String>>,
}

/// The CAT (rigctl) TCP server.
pub struct RigctlServer {
    port: u16,
    shared: Arc<RigctlShared>,
}

impl RigctlServer {
    pub fn new(ui_state: Arc<Mutex<UiState>>, cmd_tx: UnboundedSender<ControlCommand>) -> Self {
        Self {
            port: DEFAULT_RIGCTL_PORT,
            shared: Arc::new(RigctlShared {
                ui_state,
                cmd_tx,
                last_cat_mode: Mutex::new(None),
            }),
        }
    }

    /// Listen on `127.0.0.1:<port>` and serve CAT clients until the process
    /// exits.  Never panics; logs and keeps going on accept/connection errors.
    pub async fn run(self) {
        let addr = ("127.0.0.1", self.port);
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                log::error!("[rigctl] failed to bind 127.0.0.1:{}: {e}", self.port);
                return;
            }
        };
        log::info!("[rigctl] CAT server listening on 127.0.0.1:{}", self.port);

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let shared = Arc::clone(&self.shared);
                    tokio::spawn(async move {
                        log::info!("[rigctl] CAT client connected: {peer}");
                        if let Err(e) = handle_connection(stream, &shared).await {
                            log::debug!("[rigctl] connection error ({peer}): {e}");
                        }
                        log::info!("[rigctl] CAT client disconnected: {peer}");
                    });
                }
                Err(e) => log::warn!("[rigctl] accept failed: {e}"),
            }
        }
    }
}

/// Per-connection read loop: parse one command per line, write the response.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    shared: &RigctlShared,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        log::debug!("[rigctl] cmd: {line}");

        match handle_command(line, shared) {
            Reply::Send(text) => write.write_all(text.as_bytes()).await?,
            Reply::Close => break,
        }
    }
    Ok(())
}

/// What to do after handling a command.
enum Reply {
    Send(String),
    Close,
}

/// `RPRT <code>` response (0 = success; negative = error).
fn rprt(code: i32) -> Reply {
    Reply::Send(format!("RPRT {code}\n"))
}

/// Parse and handle one rigctl command line.  Supports both short (`f`, `F …`)
/// and long (`\get_freq`, `\set_freq …`) forms for the commands WSJT-X uses.
fn handle_command(line: &str, shared: &RigctlShared) -> Reply {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");

    match cmd {
        // ── Get frequency ────────────────────────────────────────────────
        "f" | "\\get_freq" => Reply::Send(format!("{}\n", current_freq_hz(shared))),

        // ── Set frequency ────────────────────────────────────────────────
        "F" | "\\set_freq" => match parts.next().and_then(parse_freq_hz) {
            Some(hz) => {
                set_frequency(shared, hz);
                rprt(0)
            }
            None => rprt(-1), // RIG_EINVAL
        },

        // ── Get mode (mode name + passband) ──────────────────────────────
        "m" | "\\get_mode" => {
            let (mode, passband) = current_mode(shared);
            Reply::Send(format!("{mode}\n{passband}\n"))
        }

        // ── Set mode ─────────────────────────────────────────────────────
        "M" | "\\set_mode" => {
            let mode = parts.next().unwrap_or("");
            // Passband is optional / may be 0 ("use default").
            let passband = parts.next().and_then(|s| s.parse::<f32>().ok());
            match rigctl_mode_to_demod(mode) {
                Some(demod) => {
                    set_mode(shared, demod, passband);
                    // Remember the exact mode string so `m` reads it back.
                    if let Ok(mut last) = shared.last_cat_mode.lock() {
                        *last = Some(mode.to_string());
                    }
                    rprt(0)
                }
                None => {
                    log::warn!("[rigctl] unsupported mode: {mode}");
                    rprt(-1)
                }
            }
        }

        // ── Get PTT ──────────────────────────────────────────────────────
        "t" | "\\get_ptt" => {
            let ptt = shared.ui_state.lock().map(|s| s.cat_ptt).unwrap_or(false);
            Reply::Send(format!("{}\n", if ptt { 1 } else { 0 }))
        }

        // ── Set PTT ──────────────────────────────────────────────────────
        "T" | "\\set_ptt" => match parts.next() {
            Some("1") => {
                set_ptt(shared, true);
                rprt(0)
            }
            Some("0") => {
                set_ptt(shared, false);
                rprt(0)
            }
            _ => rprt(-1),
        },

        // ── Capability / handshake commands WSJT-X issues ────────────────
        "\\dump_state" => Reply::Send(DUMP_STATE.to_string()),
        "\\chk_vfo" => Reply::Send("CHKVFO 0\n".to_string()),
        "v" | "\\get_vfo" => Reply::Send("VFOA\n".to_string()),
        "\\get_powerstat" => Reply::Send("1\n".to_string()),

        // ── Lock mode (WSJT-X polls this) ────────────────────────────────
        "\\get_lock_mode" => Reply::Send("0\n".to_string()),
        "\\set_lock_mode" => rprt(0),

        // ── Split (we don't do rig split; report off / accept set as no-op
        //    so WSJT-X "Fake It"/"None" split modes work).  `s` get_split_vfo
        //    replies "<split>\n<tx_vfo>\n".
        "s" | "\\get_split_vfo" => Reply::Send("0\nVFOA\n".to_string()),
        "S" | "\\set_split_vfo" => rprt(0),
        "\\get_split_freq" => Reply::Send(format!("{}\n", current_freq_hz(shared))),
        "\\set_split_freq" => rprt(0),

        // ── Quit ─────────────────────────────────────────────────────────
        "q" | "Q" | "\\quit" => Reply::Close,

        // ── Anything else: not implemented ───────────────────────────────
        other => {
            log::warn!("[rigctl] unsupported command: {other}");
            rprt(-11) // RIG_ENIMPL
        }
    }
}

// ── State access (reads UiState, writes via ControlCommand) ──────────────────

fn current_freq_hz(shared: &RigctlShared) -> u64 {
    shared
        .ui_state
        .lock()
        .map(|s| s.target_freq_hz.max(0.0) as u64)
        .unwrap_or(0)
}

/// Returns `(rigctl mode name, passband Hz)`.
///
/// If the last CAT-set mode string still maps to the current demod (i.e. the
/// mode hasn't been changed via the UI since), echo that exact string back —
/// so a client that set `PKTUSB` reads `PKTUSB`, not `USB`.  Otherwise derive
/// the name from the current demod.
fn current_mode(shared: &RigctlShared) -> (String, u32) {
    let (demod, bw) = shared
        .ui_state
        .lock()
        .map(|s| (s.demod_mode, s.filter_bandwidth_hz))
        .unwrap_or((DemodMode::Usb, 2700.0));

    let name = match shared.last_cat_mode.lock().ok().and_then(|g| g.clone()) {
        Some(s) if rigctl_mode_to_demod(&s) == Some(demod) => s,
        _ => demod_to_rigctl_mode(demod).to_string(),
    };
    (name, bw.max(0.0) as u32)
}

/// Tune to a CAT frequency.  A CAT frequency is the **operating** frequency, so
/// it must always be reachable: if it falls within the current visible band
/// (`center ± sample_rate/2`) we just move the target (preserving the LO /
/// waterfall view); otherwise it's a band change, so we move the LO center too
/// (target = center, offset 0), like tuning to a new band in the UI.  This is
/// critical — `clamp_target` would otherwise pin a far frequency to the band
/// edge, so a 14→28 MHz jump would never actually move the radio (and WSJT-X,
/// reading the frequency back, would see a mismatch and retry/stall).
fn set_frequency(shared: &RigctlShared, hz: u64) {
    let (center, sample_rate, limits) = match shared.ui_state.lock() {
        Ok(s) => (
            s.center_freq_hz,
            s.input_sample_rate_hz,
            active_freq_limits(&s),
        ),
        Err(_) => return,
    };

    // Clamp to the radio's RF tuning range (valid for both center and target).
    let freq = clamp_center(hz as f32, &limits);
    let half_bw = (sample_rate / 2.0).max(0.0);
    let in_band = half_bw > 0.0 && (freq - center).abs() <= half_bw;

    if in_band {
        if let Ok(mut s) = shared.ui_state.lock() {
            s.target_freq_hz = freq;
        }
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetTargetFrequency {
                target_freq_hz: freq as u64,
            },
        ));
    } else {
        // Band change: recenter the LO on the requested frequency.
        if let Ok(mut s) = shared.ui_state.lock() {
            s.center_freq_hz = freq;
            s.target_freq_hz = freq;
        }
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetCenterFrequency {
                center_freq_hz: freq as u64,
            },
        ));
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetTargetFrequency {
                target_freq_hz: freq as u64,
            },
        ));
    }
}

/// Change demod mode like the UI: send `SetDemodMode` (+ `SetSideband` for SSB),
/// and the filter bandwidth when a non-zero passband is given.
fn set_mode(shared: &RigctlShared, mode: DemodMode, passband: Option<f32>) {
    if let Ok(mut state) = shared.ui_state.lock() {
        state.demod_mode = mode;
        if let Some(pb) = passband {
            if pb > 0.0 {
                state.filter_bandwidth_hz = pb;
            }
        }
    }

    let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
        ClientRadioMessage::SetDemodMode { mode },
    ));

    if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
        let sideband = if mode == DemodMode::Usb {
            Sideband::Usb
        } else {
            Sideband::Lsb
        };
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetSideband { sideband },
        ));
    }

    if let Some(pb) = passband {
        if pb > 0.0 {
            let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
                ClientRadioMessage::SetFilterBandwidth { bandwidth_hz: pb },
            ));
        }
    }
}

/// Key/unkey the transmitter.  Reuses the SSB mic-TX path (`Start/StopMicTx`),
/// which keys PTT on the server; TX audio routing is intentionally not done yet.
fn set_ptt(shared: &RigctlShared, on: bool) {
    // Record the commanded PTT so the status bar shows TX and `t` reads it back.
    if let Ok(mut s) = shared.ui_state.lock() {
        s.cat_ptt = on;
    }
    let msg = if on {
        ClientRadioMessage::StartMicTx
    } else {
        ClientRadioMessage::StopMicTx
    };
    let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(msg));
}

// ── Mode name mapping ────────────────────────────────────────────────────────

/// Map a Rigflow demod to the rigctl mode name reported by `m` when we have no
/// CAT-set mode to echo.  SSB reports the **data** variant (`PKTUSB`/`PKTLSB`)
/// rather than `USB`/`LSB`, because this CAT server exists for digital software
/// (WSJT-X et al.) which runs in Data/Pkt mode and sets `PKTUSB`.  Reporting the
/// data variant makes WSJT-X's first mode read match what it intends to set, so
/// it skips its slow ~20 s mode-sync loop (which otherwise blocks even frequency
/// changes).  Rigflow has only one upper/lower-sideband mode, so this is purely
/// a CAT naming choice.  (If a client sets plain `USB`/`LSB`, the `m` echo
/// reports that back — see `current_mode`.)
fn demod_to_rigctl_mode(mode: DemodMode) -> &'static str {
    match mode {
        DemodMode::Usb => "PKTUSB",
        DemodMode::Lsb => "PKTLSB",
        DemodMode::Cwu => "CW",
        DemodMode::Cwl => "CWR",
        DemodMode::Am => "AM",
        DemodMode::Nfm => "FM",
        DemodMode::Wfm => "WFM",
    }
}

fn rigctl_mode_to_demod(mode: &str) -> Option<DemodMode> {
    match mode {
        "USB" | "PKTUSB" | "DATA-U" => Some(DemodMode::Usb),
        "LSB" | "PKTLSB" | "DATA-L" | "RTTY" => Some(DemodMode::Lsb),
        "CW" => Some(DemodMode::Cwu),
        "CWR" => Some(DemodMode::Cwl),
        "AM" => Some(DemodMode::Am),
        "FM" | "PKTFM" => Some(DemodMode::Nfm),
        "WFM" => Some(DemodMode::Wfm),
        _ => None,
    }
}

/// Parse a rigctl frequency token (may be an integer or `14074000.000000`).
fn parse_freq_hz(tok: &str) -> Option<u64> {
    tok.parse::<u64>()
        .ok()
        .or_else(|| tok.parse::<f64>().ok().map(|f| f.max(0.0) as u64))
}

/// `\dump_state` reply so Hamlib (WSJT-X) accepts the connection.  Protocol
/// version 0, classic field order (verified against Hamlib 4.7.0's parser via
/// `rigctl -m 2 -vvvvv`).  Broad RX/TX ranges so any HF/VHF frequency and the
/// common modes are accepted.  **Field order (each on its own line):** version,
/// model, ITU region; RX ranges then an all-zero terminator; TX ranges then
/// terminator; tuning steps `0 0`; filters `0 0`; max_rit; max_xit; max_ifshift;
/// announces; **preamp list (`0`)**; **attenuator list (`0`)**; then the six
/// bitmasks get_func / set_func / get_level / set_level / get_parm / set_parm.
/// (The preamp + attenuator lines were initially missing, which left Hamlib two
/// fields short → `-5 Communication timed out`.)
const DUMP_STATE: &str = "0
2
1
30000.000000 470000000.000000 0xffffff -1 -1 0x3 0x0
0 0 0 0 0 0 0
30000.000000 470000000.000000 0xffffff 1 100 0x3 0x0
0 0 0 0 0 0 0
0 0
0 0
0
0
0
0
0
0
0x0
0x0
0x0
0x0
0x0
0x0
";
