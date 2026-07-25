//! QRZ Logbook API sync — FETCH the logbook (download) and INSERT QSOs (upload).
//! Auth is a per-logbook **API key** (not your QRZ login).
//!
//! The API speaks `KEY=VALUE&KEY=VALUE` (form request, url-encoded reply). The
//! reply's `RESULT` is `OK` / `FAIL` / `AUTH`, with `COUNT`, `ADIF`, `REASON`.
//! See the module-level verification note in [`super::services`]: exact behavior
//! (bulk INSERT, dupe handling) needs a run against a real logbook key.

use std::collections::HashMap;
use std::time::Duration;

use super::services::UploadReport;

const API: &str = "https://logbook.qrz.com/api";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(90))
        .build()
}

/// FETCH the whole logbook as ADIF (empty string if the logbook is empty).
pub fn fetch(api_key: &str) -> Result<String, String> {
    let body = agent()
        .post(API)
        .send_form(&[
            ("KEY", api_key),
            ("ACTION", "FETCH"),
            ("OPTION", "TYPE:ADIF"),
        ])
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the QRZ response: {e}"))?;
    let f = parse_kv(&body);
    match f.get("RESULT").map(String::as_str) {
        Some("OK") | Some("PARTIAL") => Ok(f.get("ADIF").cloned().unwrap_or_default()),
        _ => Err(error_hint(&f)),
    }
}

/// INSERT a batch of QSOs. QRZ accepts multiple records in one ADIF and answers
/// with a count; a fully-duplicate batch comes back FAIL/DUPLICATE, which we
/// treat as "already there" (accepted), not an error.
pub fn insert(api_key: &str, adif: &str) -> Result<UploadReport, String> {
    let body = agent()
        .post(API)
        .send_form(&[("KEY", api_key), ("ACTION", "INSERT"), ("ADIF", adif)])
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the QRZ response: {e}"))?;
    let f = parse_kv(&body);
    let count = f.get("COUNT").and_then(|c| c.parse::<usize>().ok());
    match f.get("RESULT").map(String::as_str) {
        Some("OK") => Ok(UploadReport {
            accepted: count.unwrap_or(0),
            rejected: 0,
            message: f.get("REASON").cloned().unwrap_or_default(),
        }),
        // A duplicate isn't a failure to us — the QSO is on QRZ already.
        Some("FAIL")
            if f.get("REASON")
                .map(|r| r.to_ascii_uppercase().contains("DUPLICATE"))
                .unwrap_or(false) =>
        {
            Ok(UploadReport {
                accepted: count.unwrap_or(0),
                rejected: 0,
                message: "already on QRZ (duplicate)".into(),
            })
        }
        _ => Err(error_hint(&f)),
    }
}

/// Parse `KEY=VALUE&KEY=VALUE`, url-decoding each value. Keys upper-cased.
fn parse_kv(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), percent_decode(v)))
        .collect()
}

/// Minimal `application/x-www-form-urlencoded` decode: `+`→space, `%XX`→byte.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn error_hint(f: &HashMap<String, String>) -> String {
    match f.get("REASON").or_else(|| f.get("RESULT")) {
        Some(r) if !r.is_empty() => format!("QRZ: {r}"),
        _ => "QRZ returned an unexpected response — check your API key.".into(),
    }
}

fn redact(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("QRZ returned HTTP {code}."),
        ureq::Error::Transport(t) => format!("could not reach QRZ: {}", t.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_form_values() {
        assert_eq!(percent_decode("W1AW%20test+de"), "W1AW test de");
        assert_eq!(percent_decode("no-escapes"), "no-escapes");
    }

    #[test]
    fn parses_a_fetch_reply() {
        let body = "RESULT=OK&COUNT=2&ADIF=%3Ccall%3A4%3EW1AW%3Ceor%3E";
        let f = parse_kv(body);
        assert_eq!(f.get("RESULT").unwrap(), "OK");
        assert_eq!(f.get("ADIF").unwrap(), "<call:4>W1AW<eor>");
    }

    #[test]
    fn insert_treats_a_duplicate_as_accepted() {
        // Parsed the same way the live reply is; a duplicate is not an error.
        let f = parse_kv("RESULT=FAIL&COUNT=1&REASON=Unable to add QSO STATUS=DUPLICATE");
        assert_eq!(f.get("RESULT").unwrap(), "FAIL");
        assert!(
            f.get("REASON")
                .unwrap()
                .to_ascii_uppercase()
                .contains("DUPLICATE")
        );
    }
}
