//! Human-readable rendering of ADIF-native values.
//!
//! ADIF stores a date as `YYYYMMDD` and a time as `HHMMSS` — compact, sortable,
//! and unpleasant to read at a glance (`20260712`, `2356`). These helpers are
//! **presentation only**: the database, the journal, and every exported file keep
//! the ADIF-native form, because that is what the spec (and every other logging
//! program) requires. Nothing here ever feeds back into storage.
//!
//! Dates render ISO-style (`2026-07-12`) rather than a locale order: rigflow's
//! log is UTC and international, and `07/12` means two different days depending
//! on which side of the Atlantic the operator is on.
//!
//! Every function is **total** — a malformed value from an imported log is
//! passed through unchanged rather than panicking or silently becoming wrong.
//! Third-party ADIF is not guaranteed to be well-formed, and a log viewer that
//! crashes on someone else's file is worse than one that shows an odd string.

/// `20260712` → `2026-07-12`. Anything that isn't 8 digits is returned as-is.
pub fn date(qso_date: &str) -> String {
    let d = qso_date.trim();
    if d.len() != 8 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return d.to_string();
    }
    format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
}

/// `235612` or `2356` → `23:56`. Seconds are dropped: they're noise in a log
/// list, and ADIF permits a 4-digit `HHMM` anyway. Anything that isn't 4 or 6
/// digits is returned as-is.
pub fn time_hhmm(time_on: &str) -> String {
    let t = time_on.trim();
    if !matches!(t.len(), 4 | 6) || !t.bytes().all(|b| b.is_ascii_digit()) {
        return t.to_string();
    }
    format!("{}:{}", &t[0..2], &t[2..4])
}

/// `235612` → `23:56:12`, for the one place seconds matter (the frozen capture
/// on the entry window, where the operator is checking the logged instant).
pub fn time_hhmmss(time_on: &str) -> String {
    let t = time_on.trim();
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return time_hhmm(t);
    }
    format!("{}:{}:{}", &t[0..2], &t[2..4], &t[4..6])
}

/// `20260712` + `2356` → `2026-07-12 23:56Z`.
pub fn datetime(qso_date: &str, time_on: &str) -> String {
    format!("{} {}Z", date(qso_date), time_hhmm(time_on))
}

/// Accept a human-typed date and return the ADIF-native form.
///
/// The UI *shows* `2026-07-12`, so an operator will reasonably *type* that into
/// a date filter — accepting only `20260712` there would be a small betrayal.
/// Strips `-` / `/` separators; anything else is passed through for
/// `ExportFilter::validate` to reject with a proper message.
pub fn date_to_adif(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|c| *c != '-' && *c != '/')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_render_iso() {
        assert_eq!(date("20260712"), "2026-07-12");
        assert_eq!(date(" 20260101 "), "2026-01-01");
    }

    #[test]
    fn times_render_with_a_colon() {
        assert_eq!(time_hhmm("235612"), "23:56");
        assert_eq!(time_hhmm("2356"), "23:56"); // ADIF permits 4-digit HHMM
        assert_eq!(time_hhmm("0007"), "00:07");
        assert_eq!(time_hhmmss("235612"), "23:56:12");
    }

    #[test]
    fn datetime_marks_utc() {
        assert_eq!(datetime("20260712", "235612"), "2026-07-12 23:56Z");
    }

    #[test]
    fn malformed_values_pass_through_rather_than_panic() {
        // An imported log is not guaranteed well-formed. A viewer that panics on
        // someone else's ADIF is worse than one that shows an odd string.
        for junk in ["", "2026-07-12", "notadate", "202607", "1", "202607123"] {
            assert_eq!(date(junk), junk.trim());
        }
        for junk in ["", "23", "23561", "abcd", "12345678"] {
            assert_eq!(time_hhmm(junk), junk.trim());
        }
        // Crucially: no slicing panic on a short value (the old UI did `[..4]`).
        assert_eq!(time_hhmm("2"), "2");
        assert_eq!(time_hhmmss("2"), "2");
    }

    #[test]
    fn typed_dates_come_back_adif_native() {
        assert_eq!(date_to_adif("2026-07-12"), "20260712");
        assert_eq!(date_to_adif("2026/07/12"), "20260712");
        assert_eq!(date_to_adif("20260712"), "20260712");
        assert_eq!(date_to_adif(" 2026-07-12 "), "20260712");
        // Junk is preserved so validate() can reject it with a real message.
        assert_eq!(date_to_adif("nope"), "nope");
    }

    #[test]
    fn display_round_trips_with_the_input_parser() {
        // What we render, we must accept back — otherwise the operator retypes
        // what they see and gets an error.
        let adif = "20260712";
        assert_eq!(date_to_adif(&date(adif)), adif);
    }
}
