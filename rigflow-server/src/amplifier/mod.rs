//! Amplifier integration (Phase 1: Hardrock-50, read-only).
//!
//! Auto-detects an amplifier over a bidirectional transport (the Pi USB serial
//! link) and polls its status ~1 Hz, publishing a generic [`AmplifierStatus`].
//! Detection and polling are skipped for a one-way transport, so an unconfigured
//! or write-only link correctly reports "no amplifier".

pub mod hr50;
pub mod serial;
pub mod transport;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rigflow_core::radio::amplifier::{AmplifierModel, AmplifierStatus};

use hr50::{parse_hrrx, parse_hrvt, Hr50Rx, CMD_HRRX, CMD_HRVT};
use transport::AmplifierTransport;

/// Response read timeout for a single command.
const READ_TIMEOUT: Duration = Duration::from_millis(700);
/// Poll cadence once an amplifier is detected (~1 Hz).
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// Retry cadence while no amplifier is detected.
const DETECT_RETRY: Duration = Duration::from_millis(2000);
/// Consecutive poll failures before declaring the amplifier gone.
const MAX_POLL_FAILS: u32 = 3;

/// Run the detect + poll loop until `stop` is set, invoking `publish` with the
/// current status whenever it changes.  Returns immediately for a non-bidirectional
/// transport (status needs replies).
pub fn run_amplifier_poller<F>(
    mut transport: Box<dyn AmplifierTransport>,
    stop: Arc<AtomicBool>,
    mut publish: F,
) where
    F: FnMut(&AmplifierStatus),
{
    if !transport.is_bidirectional() {
        return;
    }

    let mut status = AmplifierStatus::default();
    let mut detected = false;
    let mut fails = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let rx = query_hrrx(transport.as_mut());
        let vt = query_hrvt(transport.as_mut());

        if rx.is_some() || vt.is_some() {
            // A valid reply on either command means the amp is present.
            fails = 0;
            detected = true;
            let rx = rx.unwrap_or_default();
            let next = AmplifierStatus {
                model: Some(AmplifierModel::Hr50),
                mode: rx.mode,
                band: rx.band,
                temperature_c: rx.temperature_c,
                // HRVT is the dedicated voltage command; fall back to HRRX's.
                voltage_v: vt.or(rx.voltage_v),
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
            // Still undetected — keep the published state at "None".
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

/// Query `HRRX;`.  Returns `Some` only when the reply is a real RX status
/// (mode + band present), so garbage doesn't read as a detected amplifier.
/// Logs each attempt at debug level for hardware bring-up: a garbled reply
/// usually means a baud mismatch; a timeout means no data (wrong port/wiring or
/// ACC serial disabled); a clean reply that still returns `None` is a parse issue.
fn query_hrrx(t: &mut dyn AmplifierTransport) -> Option<Hr50Rx> {
    if let Err(e) = t.write_cmd(CMD_HRRX) {
        log::debug!("[hr50] HRRX write failed: {e}");
        return None;
    }
    match t.read_response(READ_TIMEOUT) {
        Ok(Some(resp)) => {
            log::debug!("[hr50] HRRX <- {resp:?}");
            let rx = parse_hrrx(&resp)?;
            (rx.mode.is_some() && rx.band.is_some()).then_some(rx)
        }
        Ok(None) => {
            log::debug!("[hr50] HRRX <- (timeout, no reply)");
            None
        }
        Err(e) => {
            log::debug!("[hr50] HRRX read failed: {e}");
            None
        }
    }
}

/// Query `HRVT;` → volts.
fn query_hrvt(t: &mut dyn AmplifierTransport) -> Option<f32> {
    if let Err(e) = t.write_cmd(CMD_HRVT) {
        log::debug!("[hr50] HRVT write failed: {e}");
        return None;
    }
    match t.read_response(READ_TIMEOUT) {
        Ok(Some(resp)) => {
            log::debug!("[hr50] HRVT <- {resp:?}");
            parse_hrvt(&resp)
        }
        Ok(None) => {
            log::debug!("[hr50] HRVT <- (timeout, no reply)");
            None
        }
        Err(e) => {
            log::debug!("[hr50] HRVT read failed: {e}");
            None
        }
    }
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
