//! QRZ Logbook API sync — FETCH the logbook (download) and INSERT QSOs (upload).
//! Auth is a per-logbook **API key** (not your QRZ login).
//!
//! The API speaks `KEY=VALUE&KEY=VALUE`. The reply's `RESULT` is `OK` / `FAIL` /
//! `REPLACE` / `AUTH`, with `COUNT`, `ADIF`, `REASON`. Two gotchas learned on air:
//! the FETCH `ADIF` payload is **HTML-entity-encoded** (`&lt;`/`&gt;`) so it can't
//! be `&`-split with the metadata (see [`parse_reply`]); and INSERT is **one
//! record per call**, so [`insert`] loops. Confirmation is per-record via
//! `APP_QRZLOG_STATUS = C`, handled in the importer.

use std::collections::HashMap;
use std::time::Duration;

use super::services::UploadReport;

const API: &str = "https://logbook.qrz.com/api";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(90))
        .build()
}

/// FETCH the whole logbook as ADIF (empty string if the logbook is empty). The
/// caller decides which records are confirmations from their per-record
/// `APP_QRZLOG_STATUS` (a `STATUS:CONFIRMED` OPTION does not reliably filter, and
/// QRZ's default FETCH already returns ADIF).
pub fn fetch(api_key: &str) -> Result<String, String> {
    let body = agent()
        .post(API)
        .send_form(&[("KEY", api_key), ("ACTION", "FETCH")])
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the QRZ response: {e}"))?;
    let (f, adif) = parse_reply(&body);
    match f.get("RESULT").map(String::as_str) {
        Some("OK") | Some("PARTIAL") => Ok(adif.unwrap_or_default()),
        _ => Err(error_hint(&f)),
    }
}

/// Split a QRZ reply into its metadata fields and (if present) its ADIF payload.
///
/// The metadata is `KEY=VALUE&KEY=VALUE`, but the ADIF payload — always the last
/// field — is **HTML-entity-encoded** (`&lt;`/`&gt;`), so it's riddled with `&`
/// and cannot go through the `&`-splitting [`parse_kv`]. It's split off at `ADIF=`
/// and HTML-decoded on its own; everything before it parses normally.
fn parse_reply(body: &str) -> (HashMap<String, String>, Option<String>) {
    match body.find("ADIF=") {
        Some(i) => (
            parse_kv(&body[..i]),
            Some(html_decode(&body[i + "ADIF=".len()..])),
        ),
        None => (parse_kv(body), None),
    }
}

/// Decode the HTML entities QRZ uses in the ADIF payload. `&amp;` last so
/// `&amp;lt;` decodes to `&lt;`, not `<`.
fn html_decode(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// INSERT QSOs into the logbook. **QRZ's INSERT takes exactly one record per
/// call** (a single ADIF record, no header/EOH), so this loops. Returns the ids
/// that landed (inserted, replaced, or already-present duplicates), to stamp
/// `uploaded_at`, plus a tally.
///
/// A duplicate (`FAIL` with a duplicate reason) counts as landed — the QSO is on
/// QRZ already; we do not send `OPTION=REPLACE`, which would clobber a confirmed
/// QSO with our unconfirmed copy. An `AUTH` result (bad key / no privilege) is
/// fatal and stops the run rather than hammering the API once per record.
pub fn insert(
    api_key: &str,
    records: &[(i64, String)],
) -> Result<(Vec<i64>, UploadReport), String> {
    let a = agent();
    let mut landed: Vec<i64> = Vec::new();
    let mut rejected = 0usize;
    let mut last_msg = String::new();

    for (id, rec) in records {
        // A network blip on one record shouldn't abort the whole batch, nor mark
        // it uploaded: count it rejected and move on.
        let resp =
            match a
                .post(API)
                .send_form(&[("KEY", api_key), ("ACTION", "INSERT"), ("ADIF", rec)])
            {
                Ok(r) => r,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
        let body = match resp.into_string() {
            Ok(b) => b,
            Err(_) => {
                rejected += 1;
                continue;
            }
        };
        let f = parse_kv(&body);
        match f.get("RESULT").map(String::as_str) {
            Some("OK") | Some("REPLACE") => landed.push(*id),
            Some("FAIL") if is_duplicate(&f) => landed.push(*id),
            // Bad key / insufficient privilege applies to every record — stop.
            Some("AUTH") => return Err(error_hint(&f)),
            _ => {
                rejected += 1;
                last_msg = f
                    .get("REASON")
                    .cloned()
                    .unwrap_or_else(|| "rejected".into());
            }
        }
    }
    Ok((
        landed,
        UploadReport {
            accepted: 0, // per-id success is tracked by `landed`; the UI uses that
            rejected,
            message: last_msg,
        },
    ))
}

/// A QRZ FAIL that means "already in the logbook".
fn is_duplicate(f: &HashMap<String, String>) -> bool {
    ["REASON", "STATUS"].iter().any(|k| {
        f.get(*k)
            .map(|v| v.to_ascii_uppercase().contains("DUPLICATE"))
            .unwrap_or(false)
    })
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
    fn parses_a_fetch_reply_with_html_encoded_adif() {
        // QRZ's real shape: metadata first, then an HTML-entity-encoded ADIF whose
        // `&lt;`/`&gt;` would shred a naive `&`-split.
        let body =
            "COUNT=1&RESULT=OK&ADIF=&lt;call:4&gt;W1AW&lt;app_qrzlog_status:1&gt;C&lt;eor&gt;";
        let (f, adif) = parse_reply(body);
        assert_eq!(f.get("RESULT").unwrap(), "OK");
        assert_eq!(f.get("COUNT").unwrap(), "1");
        assert_eq!(
            adif.as_deref(),
            Some("<call:4>W1AW<app_qrzlog_status:1>C<eor>")
        );
    }

    #[test]
    fn a_reply_with_no_adif_has_none() {
        let (f, adif) = parse_reply("RESULT=FAIL&REASON=Bad+key");
        assert_eq!(f.get("RESULT").unwrap(), "FAIL");
        assert_eq!(adif, None);
    }

    #[test]
    fn recognizes_a_duplicate_fail() {
        // A duplicate is "already on QRZ", counted as landed, not an error.
        assert!(is_duplicate(&parse_kv(
            "RESULT=FAIL&REASON=Unable to add QSO STATUS=DUPLICATE"
        )));
        assert!(is_duplicate(&parse_kv("RESULT=FAIL&STATUS=DUPLICATE")));
        assert!(!is_duplicate(&parse_kv("RESULT=FAIL&REASON=Bad ADIF")));
    }
}
