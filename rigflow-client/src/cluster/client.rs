//! DX-cluster telnet client thread (Phase 2).
//!
//! A dedicated background thread (like `wsjtx_listener`) owns the TCP socket. It
//! is driven by [`DxClusterCommand`]s from the UI (connect / disconnect) and
//! writes parsed, deduped, bounded spots into a shared [`SpotBook`] that the
//! waterfall/spectrum renderer reads. Connection state is mirrored into
//! `UiState.dx_cluster_status` for the status line.
//!
//! Robustness:
//! - **blocking `TcpStream` with a read timeout** so the read loop also polls the
//!   command channel and runs the expiry sweep without a second thread;
//! - a **byte accumulator** for line framing — telnet lines (and the login
//!   prompt, which has no trailing newline) can arrive split across reads;
//! - **reconnect with capped backoff**, staying responsive to commands via
//!   `recv_timeout`. Cluster nodes drop connections routinely; that is normal.
//!
//! Login is just the operator callsign — public nodes have no password.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{DEFAULT_TTL, DxSpot, expire, insert_spot, parse_spot_line};
use crate::ui::state::UiState;

/// Shared, deduped, bounded spot list — written by the cluster thread, read by
/// the renderer. Kept OUT of `UiState` (deep-cloned every frame) so a few
/// thousand spots never clone per frame.
pub type SpotBook = Arc<Mutex<Vec<DxSpot>>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const EXPIRE_INTERVAL: Duration = Duration::from_secs(30);
const LOGIN_WINDOW: Duration = Duration::from_secs(6);
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Spots to backfill on connect so the display populates immediately.
const BACKFILL: u32 = 50;

/// Connection state, mirrored into `UiState` for the status line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DxClusterStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    /// Last error; the thread keeps retrying with backoff.
    Error(String),
}

/// A lifecycle command from the UI to the cluster thread.
#[derive(Debug, Clone)]
pub enum DxClusterCommand {
    Connect {
        host: String,
        port: u16,
        call: String,
    },
    Disconnect,
}

/// What the UI holds: a command sender plus the shared spot book.
pub struct DxClusterHandle {
    tx: Sender<DxClusterCommand>,
    pub spots: SpotBook,
}

impl DxClusterHandle {
    pub fn connect(&self, host: impl Into<String>, port: u16, call: impl Into<String>) {
        let _ = self.tx.send(DxClusterCommand::Connect {
            host: host.into(),
            port,
            call: call.into(),
        });
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(DxClusterCommand::Disconnect);
    }
}

/// Spawn the cluster thread. It idles until it receives a `Connect` command.
pub fn spawn_dx_cluster(state: Arc<Mutex<UiState>>) -> DxClusterHandle {
    let (tx, rx) = std::sync::mpsc::channel();
    let spots: SpotBook = Arc::new(Mutex::new(Vec::new()));
    let spots_thread = Arc::clone(&spots);
    let _ = std::thread::Builder::new()
        .name("dx-cluster".to_string())
        .spawn(move || run(state, rx, spots_thread));
    DxClusterHandle { tx, spots }
}

#[derive(Clone)]
struct Target {
    host: String,
    port: u16,
    call: String,
}

/// The effect a command has on the desired-connection target.
enum Effect {
    SetTarget(Target),
    Clear,
}

fn effect_of(cmd: DxClusterCommand) -> Effect {
    match cmd {
        DxClusterCommand::Connect { host, port, call } => {
            Effect::SetTarget(Target { host, port, call })
        }
        DxClusterCommand::Disconnect => Effect::Clear,
    }
}

/// Apply a command effect: a new target resets backoff, a disconnect clears it.
fn apply(effect: Effect, target: &mut Option<Target>, backoff: &mut Duration) {
    match effect {
        Effect::SetTarget(t) => {
            *target = Some(t);
            *backoff = INITIAL_BACKOFF;
        }
        Effect::Clear => *target = None,
    }
}

/// Why a live connection ended.
enum ServeEnd {
    /// UI asked to disconnect — go idle.
    Disconnect,
    /// UI asked to connect elsewhere — switch target immediately.
    NewTarget(Target),
    /// Socket dropped/errored — reconnect after backoff.
    Reconnect(String),
    /// Command channel closed (app shutting down) — exit the thread.
    Closed,
}

fn set_status(state: &Arc<Mutex<UiState>>, status: DxClusterStatus) {
    if let Ok(mut s) = state.lock() {
        s.dx_cluster_status = status;
    }
}

fn grow(backoff: Duration) -> Duration {
    (backoff * 2).min(MAX_BACKOFF)
}

fn run(state: Arc<Mutex<UiState>>, rx: Receiver<DxClusterCommand>, spots: SpotBook) {
    let mut target: Option<Target> = None;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        // No target: go idle and block until the UI asks to connect.
        let Some(t) = target.clone() else {
            set_status(&state, DxClusterStatus::Disconnected);
            match rx.recv() {
                Ok(cmd) => apply(effect_of(cmd), &mut target, &mut backoff),
                Err(_) => return, // sender dropped → app closing
            }
            continue;
        };

        set_status(&state, DxClusterStatus::Connecting);
        match tcp_connect(&t.host, t.port) {
            Ok(stream) => {
                backoff = INITIAL_BACKOFF;
                match serve(stream, &t, &rx, &state, &spots) {
                    ServeEnd::Disconnect => target = None,
                    ServeEnd::NewTarget(nt) => {
                        apply(Effect::SetTarget(nt), &mut target, &mut backoff)
                    }
                    ServeEnd::Closed => return,
                    ServeEnd::Reconnect(reason) => {
                        set_status(&state, DxClusterStatus::Error(reason));
                        if backoff_wait(&rx, &mut backoff, &mut target).is_closed() {
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                set_status(
                    &state,
                    DxClusterStatus::Error(format!("connect failed: {e}")),
                );
                if backoff_wait(&rx, &mut backoff, &mut target).is_closed() {
                    return;
                }
            }
        }
    }
}

enum WaitOutcome {
    Continue,
    Closed,
}

impl WaitOutcome {
    fn is_closed(&self) -> bool {
        matches!(self, WaitOutcome::Closed)
    }
}

/// Wait out the backoff, staying responsive: a command applies immediately (and
/// resets backoff), a real timeout grows it, a closed channel ends the thread.
fn backoff_wait(
    rx: &Receiver<DxClusterCommand>,
    backoff: &mut Duration,
    target: &mut Option<Target>,
) -> WaitOutcome {
    match rx.recv_timeout(*backoff) {
        Ok(cmd) => {
            apply(effect_of(cmd), target, backoff);
            WaitOutcome::Continue
        }
        Err(RecvTimeoutError::Timeout) => {
            *backoff = grow(*backoff);
            WaitOutcome::Continue
        }
        Err(RecvTimeoutError::Disconnected) => WaitOutcome::Closed,
    }
}

fn tcp_connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs()?;
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no address resolved")))
}

/// Drive a live connection: log in, request a backfill, then stream spots until
/// the socket drops or a command interrupts.
fn serve(
    mut stream: TcpStream,
    target: &Target,
    rx: &Receiver<DxClusterCommand>,
    state: &Arc<Mutex<UiState>>,
    spots: &SpotBook,
) -> ServeEnd {
    if let Err(e) = stream.set_read_timeout(Some(READ_TIMEOUT)) {
        return ServeEnd::Reconnect(format!("set_read_timeout: {e}"));
    }

    // Login + backfill.
    match login(&mut stream, &target.call, rx) {
        Ok(()) => {}
        Err(end) => return end,
    }
    let _ = send_line(&mut stream, &format!("sh/dx {BACKFILL}"));
    set_status(state, DxClusterStatus::Connected);

    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    let mut last_expire = Instant::now();

    loop {
        match poll_command(rx) {
            CmdPoll::Disconnect => return ServeEnd::Disconnect,
            CmdPoll::NewTarget(t) => return ServeEnd::NewTarget(t),
            CmdPoll::Closed => return ServeEnd::Closed,
            CmdPoll::None => {}
        }

        match stream.read(&mut buf) {
            Ok(0) => return ServeEnd::Reconnect("connection closed by node".to_string()),
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                drain_lines(&mut acc, spots);
            }
            Err(e) if is_timeout(&e) => {} // no data this tick — fall through to housekeeping
            Err(e) => return ServeEnd::Reconnect(format!("read error: {e}")),
        }

        if last_expire.elapsed() >= EXPIRE_INTERVAL {
            if let Ok(mut s) = spots.lock() {
                expire(&mut s, Instant::now(), DEFAULT_TTL);
            }
            last_expire = Instant::now();
        }
    }
}

/// Read until a recognisable login prompt (or a short timeout), then send the
/// callsign. Public nodes accept the call unprompted, so a timeout is harmless.
fn login(
    stream: &mut TcpStream,
    call: &str,
    rx: &Receiver<DxClusterCommand>,
) -> Result<(), ServeEnd> {
    let start = Instant::now();
    let mut acc = String::new();
    let mut buf = [0u8; 1024];

    while start.elapsed() < LOGIN_WINDOW {
        match poll_command(rx) {
            CmdPoll::Disconnect => return Err(ServeEnd::Disconnect),
            CmdPoll::NewTarget(t) => return Err(ServeEnd::NewTarget(t)),
            CmdPoll::Closed => return Err(ServeEnd::Closed),
            CmdPoll::None => {}
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err(ServeEnd::Reconnect("closed during login".to_string())),
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                if acc.contains("login")
                    || acc.contains("your call")
                    || acc.contains("call:")
                    || acc.contains("callsign")
                {
                    break;
                }
            }
            Err(e) if is_timeout(&e) => {}
            Err(e) => return Err(ServeEnd::Reconnect(format!("login read: {e}"))),
        }
    }

    send_line(stream, call).map_err(|e| ServeEnd::Reconnect(format!("login write: {e}")))
}

/// Pull complete `\n`-terminated lines out of the accumulator, parse each, and
/// insert any spots. A trailing partial line stays buffered for the next read.
fn drain_lines(acc: &mut String, spots: &SpotBook) {
    while let Some(pos) = acc.find('\n') {
        let line: String = acc.drain(..=pos).collect();
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(spot) = parse_spot_line(line, Instant::now())
            && let Ok(mut s) = spots.lock()
        {
            insert_spot(&mut s, spot);
        }
    }
}

enum CmdPoll {
    None,
    Disconnect,
    NewTarget(Target),
    Closed,
}

fn poll_command(rx: &Receiver<DxClusterCommand>) -> CmdPoll {
    match rx.try_recv() {
        Ok(DxClusterCommand::Disconnect) => CmdPoll::Disconnect,
        Ok(DxClusterCommand::Connect { host, port, call }) => {
            CmdPoll::NewTarget(Target { host, port, call })
        }
        Err(TryRecvError::Empty) => CmdPoll::None,
        Err(TryRecvError::Disconnected) => CmdPoll::Closed,
    }
}

fn send_line(stream: &mut TcpStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}
