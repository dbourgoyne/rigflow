//! Duplicate detection: natural key `call + mode + band` within a time window.
//!
//! This is a **soft** signal, never a hard reject. Contests legitimately
//! re-work the same station on the same band/mode, so the caller decides what
//! to do with a match (typically warn the operator). It exists mainly to guard
//! against double-logging the same QSO — e.g. a QSO logged both by rigflow and
//! by GridTracker, or a WSJT-X "logged ADIF" packet arriving twice.

use chrono::NaiveDateTime;

use crate::model::Qso;

/// Default match window: ±30 minutes.
pub const DEFAULT_WINDOW_SECS: i64 = 30 * 60;

/// Parse ADIF `qso_date` (`YYYYMMDD`) + `time_on` (`HHMM` or `HHMMSS`) into a
/// naive UTC datetime. Returns `None` on malformed input.
pub fn parse_dt(date: &str, time: &str) -> Option<NaiveDateTime> {
    let t = match time.trim().len() {
        4 => format!("{}00", time.trim()), // HHMM → HHMM00
        6 => time.trim().to_string(),
        _ => return None,
    };
    NaiveDateTime::parse_from_str(&format!("{}{}", date.trim(), t), "%Y%m%d%H%M%S").ok()
}

/// Same natural key: callsign (case-insensitive), mode, and band all match.
pub fn same_natural_key(a: &Qso, b: &Qso) -> bool {
    a.call.eq_ignore_ascii_case(b.call.trim()) && a.mode == b.mode && a.band == b.band
}

/// The two QSOs' start times are within `window_secs` of each other. If either
/// timestamp is unparseable, they are treated as **not** within the window
/// (conservative — don't suppress on bad data).
pub fn within_window(a: &Qso, b: &Qso, window_secs: i64) -> bool {
    match (
        parse_dt(&a.qso_date, &a.time_on),
        parse_dt(&b.qso_date, &b.time_on),
    ) {
        (Some(x), Some(y)) => (x - y).num_seconds().abs() <= window_secs,
        _ => false,
    }
}

/// Same natural key AND within the time window.
pub fn is_duplicate(a: &Qso, b: &Qso, window_secs: i64) -> bool {
    same_natural_key(a, b) && within_window(a, b, window_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(call: &str, band: &str, mode: &str, date: &str, time: &str) -> Qso {
        Qso {
            call: call.into(),
            band: band.into(),
            mode: mode.into(),
            qso_date: date.into(),
            time_on: time.into(),
            ..Default::default()
        }
    }

    #[test]
    fn same_contact_five_minutes_apart_is_dup() {
        let a = q("W1AW", "20m", "SSB", "20260711", "142300");
        let b = q("w1aw", "20m", "SSB", "20260711", "142800");
        assert!(is_duplicate(&a, &b, DEFAULT_WINDOW_SECS));
    }

    #[test]
    fn window_boundary_is_inclusive() {
        let a = q("W1AW", "20m", "SSB", "20260711", "140000");
        let exactly = q("W1AW", "20m", "SSB", "20260711", "143000"); // +30:00
        let just_over = q("W1AW", "20m", "SSB", "20260711", "143001"); // +30:01
        assert!(is_duplicate(&a, &exactly, DEFAULT_WINDOW_SECS));
        assert!(!is_duplicate(&a, &just_over, DEFAULT_WINDOW_SECS));
    }

    #[test]
    fn contest_rework_hours_later_is_not_dup() {
        let a = q("W1AW", "20m", "SSB", "20260711", "142300");
        let b = q("W1AW", "20m", "SSB", "20260711", "182300"); // 4h later
        assert!(!is_duplicate(&a, &b, DEFAULT_WINDOW_SECS));
    }

    #[test]
    fn different_band_is_not_dup() {
        let a = q("W1AW", "20m", "SSB", "20260711", "142300");
        let b = q("W1AW", "40m", "SSB", "20260711", "142400");
        assert!(!is_duplicate(&a, &b, DEFAULT_WINDOW_SECS));
    }

    #[test]
    fn different_mode_is_not_dup() {
        let a = q("W1AW", "20m", "SSB", "20260711", "142300");
        let b = q("W1AW", "20m", "CW", "20260711", "142400");
        assert!(!is_duplicate(&a, &b, DEFAULT_WINDOW_SECS));
    }

    #[test]
    fn dup_across_midnight() {
        let a = q("W1AW", "20m", "SSB", "20260711", "235800");
        let b = q("W1AW", "20m", "SSB", "20260712", "000200"); // +4 min over date line
        assert!(is_duplicate(&a, &b, DEFAULT_WINDOW_SECS));
    }
}
