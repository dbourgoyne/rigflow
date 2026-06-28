//! Amplifier integration (Hardrock-50).
//!
//! Phase 1: auto-detect over the USB serial link and poll status ~1 Hz.
//! Phase 2: also track the dial frequency (`FA`), apply control commands
//! (keying mode, ATU bypass/active, tune), and report TX power/SWR + ATU state.
//!
//! The poller thread **exclusively owns** the serial transport, so control comes
//! in through an [`AmpCommand`] channel and the current frequency through a
//! lock-free atomic — both supplied by the worker, keeping this module free of
//! the worker's internal state.  When no amplifier replies the status stays at
//! `model: None`.

pub mod hr50;
pub mod serial;
pub mod transport;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rigflow_core::radio::amplifier::{
    AmplifierAtuMode, AmplifierKeyingMode, AmplifierModel, AmplifierStatus,
};

use hr50::{
    cmd_fa, cmd_hrat, cmd_hrmd, cmd_hrtu, parse_hrat, parse_hrmx, parse_hrrx, parse_hrvt, CMD_HRAT,
    CMD_HRMX, CMD_HRRX, CMD_HRVT,
};
use transport::AmplifierTransport;

/// Response read timeout for a single command.
const READ_TIMEOUT: Duration = Duration::from_millis(700);
/// Per-attempt budget while probing a candidate port/baud during auto-detect.
/// Shorter than `READ_TIMEOUT` so a full no-amp scan stays a couple of seconds.
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
/// USB vendor ids of common USB-serial converter chips. A port whose owning
/// device matches one of these is a *candidate* worth probing for an HR50:
/// `0x04D8` is the Microchip MCP2200 inside the HR50's own USB-B port; the rest
/// cover generic USB-serial cables on the amp's ACC port. This list only decides
/// which ports get a read-only probe — a port is *adopted* solely when it
/// answers a real `HRRX;` query, so casting a wide net here is safe.
const CONVERTER_VENDOR_IDS: &[u16] = &[
    0x0403, // FTDI (FT232R/FT-X/FT232H/FT2232 …)
    0x04D8, // Microchip (MCP2200 — HR50 built-in USB port)
    0x10C4, // Silicon Labs (CP210x)
    0x067B, // Prolific (PL2303)
    0x1A86, // QinHeng (CH340/CH341)
];
/// Baud rates probed during auto-detect, after the caller's preferred value.
/// Limited to the rates the HR50 menus offer (and `SerialTransport` supports).
const PROBE_BAUDS: &[u32] = &[19200, 9600, 38400, 57600, 115200, 4800];
/// Best-effort discard window after a fire-and-forget SET command.
const SET_DRAIN: Duration = Duration::from_millis(120);
/// Poll cadence once an amplifier is detected (~1 Hz).
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// The HR50 firmware **hangs** (power-cycle required) if it receives serial while
/// keyed (PTT asserted) — so the poller stays completely silent during TX and for
/// this long after unkey, letting the amp settle into RX.  The HR50 peak-holds its
/// last-TX PEP/Avg/SWR, so the first `HRMX` read after the settle still reports the
/// over's TX stats.
const TX_SETTLE: Duration = Duration::from_millis(150);
/// How often to re-check the keyed state while holding serial off during TX.
const TX_HOLDOFF_CHECK: Duration = Duration::from_millis(100);
/// Retry cadence while no amplifier is detected.
const DETECT_RETRY: Duration = Duration::from_millis(2000);
/// Consecutive poll failures before declaring the amplifier gone.  Generous so a
/// transmission doesn't blank the panel: TX RF commonly garbles the serial line
/// for the length of an over (e.g. a 15 s FT8 over), and the amp is still there —
/// we keep showing the last-known status until it's been silent this long.
const MAX_POLL_FAILS: u32 = 30;

/// A control command for the amplifier, queued from the worker to the poller.
#[derive(Debug, Clone, Copy)]
pub enum AmpCommand {
    SetKeyingMode(AmplifierKeyingMode),
    SetAtuMode(AmplifierAtuMode),
    TuneAtu,
}

/// Auto-detect the HR50 serial port and its baud rate.
///
/// Two stages, so a wrong guess can never *continuously* talk to a non-HR50
/// device (the safety property the old hard-coded `/dev/ttyUSB0` default lacked):
/// 1. **Narrow** — enumerate USB-serial ports and keep only those whose owning
///    device is a known serial-converter chip ([`CONVERTER_VENDOR_IDS`]).
/// 2. **Confirm** — open each candidate and send a read-only `HRRX;`; adopt the
///    first port/baud that returns a valid HR50 status.
///
/// Bauds are tried `preferred_baud` first (the configured/`--hr50-baud` value),
/// then [`PROBE_BAUDS`], so the amp is found regardless of how its baud menu is
/// set. Returns the matched `(path, baud)`, or `None` if nothing answers.
/// Linux-only (reads sysfs); returns `None` on other hosts.
pub fn autodetect_serial(preferred_baud: u32) -> Option<(String, u32)> {
    let candidates: Vec<_> = serial::enumerate_ports()
        .into_iter()
        .filter(|p| CONVERTER_VENDOR_IDS.contains(&p.vid))
        .collect();
    if candidates.is_empty() {
        log::debug!("[hr50] auto-detect: no USB-serial converter ports present");
        return None;
    }

    // preferred_baud first, then the rest (skipping the duplicate).
    let bauds = std::iter::once(preferred_baud)
        .chain(PROBE_BAUDS.iter().copied().filter(|&b| b != preferred_baud));

    for baud in bauds {
        for p in &candidates {
            log::debug!(
                "[hr50] probing {} ({:04x}:{:04x} {}) @ {baud}",
                p.path,
                p.vid,
                p.pid,
                p.product.as_deref().unwrap_or("?"),
            );
            match serial::SerialTransport::open(&p.path, baud) {
                Ok(mut t) => {
                    if probe_is_hr50(&mut t) {
                        log::info!(
                            "[hr50] auto-detected amplifier on {} ({:04x}:{:04x} {}) @ {baud} 8N1",
                            p.path,
                            p.vid,
                            p.pid,
                            p.product.as_deref().unwrap_or("?"),
                        );
                        return Some((p.path.clone(), baud));
                    }
                }
                Err(e) => log::debug!("[hr50] probe open {} @ {baud} failed: {e}", p.path),
            }
        }
    }

    log::debug!(
        "[hr50] auto-detect: {} converter port(s) present, none answered an HR50 probe",
        candidates.len()
    );
    None
}

/// Send one read-only `HRRX;` and report whether a valid HR50 RX status came
/// back within [`PROBE_TIMEOUT`]. Used only during auto-detect.
fn probe_is_hr50(t: &mut dyn AmplifierTransport) -> bool {
    if t.write_cmd(CMD_HRRX).is_err() {
        return false;
    }
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match t.read_response(remaining) {
            // A real RX status has both mode and band; anything else is skipped
            // (the amp can interleave auto-reports) until the deadline.
            Ok(Some(seg)) => {
                if seg.contains("RX,") {
                    if let Some(rx) = parse_hrrx(&seg) {
                        if rx.mode.is_some() && rx.band.is_some() {
                            return true;
                        }
                    }
                }
            }
            Ok(None) | Err(_) => return false,
        }
    }
}

/// Run the detect + poll + control loop until `stop` is set.
///
/// - `commands` — control commands to apply (drained each cycle).
/// - `target_freq_hz` — current dial frequency; `FA` is sent whenever it changes.
/// - `publish` — called with the current status whenever it changes.
pub fn run_amplifier_poller<F>(
    mut transport: Box<dyn AmplifierTransport>,
    stop: Arc<AtomicBool>,
    commands: Receiver<AmpCommand>,
    target_freq_hz: Arc<AtomicU64>,
    amp_tx_prep_req: Arc<AtomicU64>,
    amp_tx_prep_done: Arc<AtomicBool>,
    tx_keyed: Vec<Arc<AtomicBool>>,
    mut publish: F,
) where
    F: FnMut(&AmplifierStatus),
{
    let mut status = AmplifierStatus::default();
    let mut detected = false;
    let mut fails = 0u32;
    let mut last_freq_sent = 0u64;
    let mut last_keyed: Option<Instant> = None;

    while !stop.load(Ordering::Relaxed) {
        // Never touch the serial port while keyed (it hangs the HR50), nor for a
        // brief settle after unkey.  This is not a poll failure — we're choosing
        // not to poll — so `fails` is left untouched and the amp won't be dropped.
        let keyed = tx_keyed.iter().any(|f| f.load(Ordering::Relaxed));
        if keyed {
            last_keyed = Some(Instant::now());
        }
        if keyed || last_keyed.is_some_and(|t| t.elapsed() < TX_SETTLE) {
            sleep_responsive(&stop, TX_HOLDOFF_CHECK);
            continue;
        }

        let t = transport.as_mut();

        // A cross-band split key-down waits for the band change before RF, so
        // honor a pending TX band-prep request as early as possible — and again
        // between each blocking status read below — to keep the pre-TX wait short.
        service_tx_prep(
            t,
            &tx_keyed,
            &amp_tx_prep_req,
            &amp_tx_prep_done,
            &mut last_freq_sent,
        );

        // 1. Apply queued control commands (one serial SET each).
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                AmpCommand::SetKeyingMode(m) => send_set(t, &cmd_hrmd(m), "HRMD"),
                AmpCommand::SetAtuMode(m) => send_set(t, &cmd_hrat(m.hr50_code()), "HRAT"),
                AmpCommand::TuneAtu => send_set(t, &cmd_hrtu(), "HRTU"),
            }
        }

        // 2. Frequency tracking: send FA when the dial frequency changes.
        let freq = target_freq_hz.load(Ordering::Relaxed);
        if freq != last_freq_sent && freq > 0 {
            send_set(t, &cmd_fa(freq), "FA");
            last_freq_sent = freq;
        }

        // 3. Poll status / telemetry (servicing band-prep between each read).
        let rx = query_hrrx(t);
        service_tx_prep(
            t,
            &tx_keyed,
            &amp_tx_prep_req,
            &amp_tx_prep_done,
            &mut last_freq_sent,
        );
        let vt = query_hrvt(t);
        service_tx_prep(
            t,
            &tx_keyed,
            &amp_tx_prep_req,
            &amp_tx_prep_done,
            &mut last_freq_sent,
        );
        let mx = query(t, CMD_HRMX, "HRMX", "HRMX").and_then(|s| parse_hrmx(&s));
        service_tx_prep(
            t,
            &tx_keyed,
            &amp_tx_prep_req,
            &amp_tx_prep_done,
            &mut last_freq_sent,
        );
        let at = query(t, CMD_HRAT, "HRAT", "HRAT").and_then(|s| parse_hrat(&s));

        if rx.is_some() || vt.is_some() {
            fails = 0;
            detected = true;
            let rx = rx.unwrap_or_default();
            // Preserve last-known TX/ATU readings when a field didn't read this
            // cycle (each field independently optional).
            let (tx_pep_w, tx_avg_w, tx_swr) = match mx {
                Some(m) => (m.pep_w, m.avg_w, m.swr),
                None => (status.tx_pep_w, status.tx_avg_w, status.tx_swr),
            };
            let (atu_present, atu_mode) = match at {
                Some(code) => (code != 0, AmplifierAtuMode::from_hr50_code(code)),
                None => (status.atu_present, status.atu_mode),
            };
            let next = AmplifierStatus {
                model: Some(AmplifierModel::Hr50),
                mode: rx.mode,
                band: rx.band,
                temperature_c: rx.temperature_c,
                voltage_v: vt.or(rx.voltage_v),
                tx_pep_w,
                tx_avg_w,
                tx_swr,
                atu_present,
                atu_mode,
                last_error: None,
            };
            publish_if_changed(&mut status, next, &mut publish);
        } else if detected {
            fails += 1;
            if fails >= MAX_POLL_FAILS {
                detected = false;
                let next = AmplifierStatus {
                    last_error: Some("amplifier not responding".to_string()),
                    ..Default::default()
                };
                publish_if_changed(&mut status, next, &mut publish);
            }
        } else {
            publish_if_changed(&mut status, AmplifierStatus::default(), &mut publish);
        }

        let wait = if detected {
            POLL_INTERVAL
        } else {
            DETECT_RETRY
        };
        wait_servicing_prep(
            &stop,
            wait,
            transport.as_mut(),
            &tx_keyed,
            &amp_tx_prep_req,
            &amp_tx_prep_done,
            &mut last_freq_sent,
        );
    }
}

/// Service a pending TX band-prep request: if the worker asked for an immediate
/// `FA` (a cross-band split key-down, set while still unkeyed) and we're not
/// keyed, send it now and signal completion via `prep_done`.  Called at several
/// points in the poll loop so the request is honored within one in-flight serial
/// read.  Never touches the serial while keyed (the HR50 hangs).
fn service_tx_prep(
    t: &mut dyn AmplifierTransport,
    tx_keyed: &[Arc<AtomicBool>],
    prep_req: &AtomicU64,
    prep_done: &AtomicBool,
    last_freq_sent: &mut u64,
) {
    let req = prep_req.load(Ordering::Relaxed);
    if req == 0 {
        return;
    }
    if tx_keyed.iter().any(|f| f.load(Ordering::Relaxed)) {
        // Should not happen (prep precedes key), but never serial while keyed.
        return;
    }
    send_set(t, &cmd_fa(req), "FA(tx-prep)");
    *last_freq_sent = req;
    prep_req.store(0, Ordering::Relaxed);
    prep_done.store(true, Ordering::Relaxed);
}

/// Idle wait that also services TX band-prep requests at ~10 ms granularity, so a
/// cross-band key-down isn't blocked by the 1 s poll cadence.  Wakes early on
/// `stop`.
#[allow(clippy::too_many_arguments)]
fn wait_servicing_prep(
    stop: &Arc<AtomicBool>,
    dur: Duration,
    t: &mut dyn AmplifierTransport,
    tx_keyed: &[Arc<AtomicBool>],
    prep_req: &AtomicU64,
    prep_done: &AtomicBool,
    last_freq_sent: &mut u64,
) {
    let step = Duration::from_millis(10);
    let mut left = dur;
    while left > Duration::ZERO && !stop.load(Ordering::Relaxed) {
        service_tx_prep(t, tx_keyed, prep_req, prep_done, last_freq_sent);
        let s = step.min(left);
        std::thread::sleep(s);
        left = left.saturating_sub(s);
    }
}

fn publish_if_changed<F: FnMut(&AmplifierStatus)>(
    current: &mut AmplifierStatus,
    next: AmplifierStatus,
    publish: &mut F,
) {
    if *current != next {
        *current = next;
        publish(current);
    }
}

/// Write `cmd`, then read `;`-terminated segments until one contains `prefix`
/// (skipping unrelated segments — the HR50 "responds to the last command", so
/// SET echoes / auto-reports can precede the reply we want), or the timeout
/// elapses.  Logs at debug for hardware bring-up.
fn query(t: &mut dyn AmplifierTransport, cmd: &[u8], prefix: &str, label: &str) -> Option<String> {
    if let Err(e) = t.write_cmd(cmd) {
        log::debug!("[hr50] {label} write failed: {e}");
        return None;
    }
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            log::debug!("[hr50] {label} <- (timeout, no reply)");
            return None;
        }
        match t.read_response(remaining) {
            Ok(Some(seg)) => {
                if seg.contains(prefix) {
                    log::debug!("[hr50] {label} <- {seg:?}");
                    return Some(seg);
                }
                log::trace!("[hr50] {label} skip {seg:?}");
            }
            Ok(None) => {
                log::debug!("[hr50] {label} <- (timeout, no reply)");
                return None;
            }
            Err(e) => {
                log::debug!("[hr50] {label} read failed: {e}");
                return None;
            }
        }
    }
}

/// Query `HRRX;` → `Some` only when the reply is a real RX status (mode + band
/// present), so garbage doesn't read as a detected amplifier.
fn query_hrrx(t: &mut dyn AmplifierTransport) -> Option<hr50::Hr50Rx> {
    let resp = query(t, CMD_HRRX, "RX,", "HRRX")?;
    let rx = parse_hrrx(&resp)?;
    (rx.mode.is_some() && rx.band.is_some()).then_some(rx)
}

/// Query `HRVT;` → volts.
fn query_hrvt(t: &mut dyn AmplifierTransport) -> Option<f32> {
    let resp = query(t, CMD_HRVT, "HRVT", "HRVT")?;
    parse_hrvt(&resp)
}

/// Write a fire-and-forget SET command, then briefly discard any reply so it
/// doesn't pollute the next GET (the amp may echo/respond to SETs).
fn send_set(t: &mut dyn AmplifierTransport, bytes: &[u8], label: &str) {
    if let Err(e) = t.write_cmd(bytes) {
        log::debug!("[hr50] {label} write failed: {e}");
        return;
    }
    log::debug!("[hr50] {label} -> {:?}", String::from_utf8_lossy(bytes));
    let _ = t.read_response(SET_DRAIN);
}

/// Sleep up to `dur`, waking early (~50 ms granularity) when `stop` is set.
fn sleep_responsive(stop: &Arc<AtomicBool>, dur: Duration) {
    let step = Duration::from_millis(50);
    let mut left = dur;
    while left > Duration::ZERO && !stop.load(Ordering::Relaxed) {
        let s = step.min(left);
        std::thread::sleep(s);
        left = left.saturating_sub(s);
    }
}
