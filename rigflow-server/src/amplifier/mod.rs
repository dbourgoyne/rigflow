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
/// Best-effort discard window after a fire-and-forget SET command.
const SET_DRAIN: Duration = Duration::from_millis(120);
/// Poll cadence once an amplifier is detected (~1 Hz).
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// Retry cadence while no amplifier is detected.
const DETECT_RETRY: Duration = Duration::from_millis(2000);
/// Consecutive poll failures before declaring the amplifier gone.
const MAX_POLL_FAILS: u32 = 3;

/// A control command for the amplifier, queued from the worker to the poller.
#[derive(Debug, Clone, Copy)]
pub enum AmpCommand {
    SetKeyingMode(AmplifierKeyingMode),
    SetAtuMode(AmplifierAtuMode),
    TuneAtu,
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
    mut publish: F,
) where
    F: FnMut(&AmplifierStatus),
{
    let mut status = AmplifierStatus::default();
    let mut detected = false;
    let mut fails = 0u32;
    let mut last_freq_sent = 0u64;

    while !stop.load(Ordering::Relaxed) {
        let t = transport.as_mut();

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

        // 3. Poll status / telemetry.
        let rx = query_hrrx(t);
        let vt = query_hrvt(t);
        let mx = query(t, CMD_HRMX, "HRMX", "HRMX").and_then(|s| parse_hrmx(&s));
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
        sleep_responsive(&stop, wait);
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
