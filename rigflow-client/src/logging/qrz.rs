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

/// Records requested per FETCH page. QRZ caps a single fetch, so we page.
const PAGE: usize = 250;
/// Backstop on the page loop, in case the cursor ever fails to advance.
const MAX_PAGES: usize = 400;

/// FETCH the **confirmed** QSOs as ADIF (empty string if there are none).
///
/// Only `STATUS:CONFIRMED` — the confirmed set is a small fraction of a logbook,
/// so we never pull the whole thing. Confirmed records come back in ascending
/// `APP_QRZLOG_LOGID` order, so we page with `AFTERLOGID` (the largest logid of
/// the previous page) until a page comes back empty. **This is intra-fetch paging,
/// not a cross-sync cursor**: a confirmation lands on an arbitrarily old record,
/// so each sync re-pulls the full confirmed set and relies on the import being
/// idempotent. Which records are confirmations is still decided per-record by the
/// importer (`APP_QRZLOG_STATUS = C`).
pub fn fetch(api_key: &str) -> Result<String, String> {
    let a = agent();
    let mut out = String::new();
    let mut after: u64 = 0;

    for _ in 0..MAX_PAGES {
        let option = format!("STATUS:CONFIRMED,MAX:{PAGE},AFTERLOGID:{after}");
        // GET is QRZ's preferred method for FETCH; ureq url-encodes each query
        // value (like curl's --data-urlencode). redact() keeps the key — now in
        // the URL — out of any error string.
        let body = a
            .get(API)
            .query("KEY", api_key)
            .query("ACTION", "FETCH")
            .query("OPTION", &option)
            .call()
            .map_err(redact)?
            .into_string()
            .map_err(|e| format!("reading the QRZ response: {e}"))?;
        let (f, adif) = parse_reply(&body);
        match f.get("RESULT").map(String::as_str) {
            Some("OK") | Some("PARTIAL") => {}
            // A failure on the very first page is a real error (bad key, bad
            // option); later it just means we've paged past the last record.
            _ if out.is_empty() => return Err(error_hint(&f)),
            _ => break,
        }
        let Some(adif) = adif.filter(|s| !s.trim().is_empty()) else {
            break;
        };
        let (count, max_logid) = page_stats(&adif);
        if count == 0 {
            break;
        }
        out.push_str(&adif);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        // Stop when the cursor can't advance (last page, or a stuck cursor).
        if max_logid <= after {
            break;
        }
        after = max_logid;
    }
    Ok(out)
}

/// Records in one ADIF page (`<eor>` count) and the largest `APP_QRZLOG_LOGID` in
/// it — the `AFTERLOGID` cursor for the next page. `0` when a page carries no
/// logid, which stops paging (its `<=` cursor check trips).
fn page_stats(adif: &str) -> (usize, u64) {
    let lower = adif.to_ascii_lowercase();
    let count = lower.matches("<eor>").count();
    const KEY: &str = "<app_qrzlog_logid:";
    let mut max_logid = 0u64;
    let mut from = 0;
    while let Some(i) = lower[from..].find(KEY) {
        let start = from + i + KEY.len();
        let Some(gt) = lower[start..].find('>') else {
            break;
        };
        let len: usize = lower[start..start + gt]
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(0);
        let vstart = start + gt + 1;
        if let Some(v) = lower.get(vstart..vstart + len)
            && let Ok(id) = v.trim().parse::<u64>()
        {
            max_logid = max_logid.max(id);
        }
        from = vstart + len.max(1);
    }
    (count, max_logid)
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
    fn page_stats_counts_records_and_finds_the_max_logid() {
        // Decoded ADIF (what parse_reply hands us), two records, ascending logid.
        let adif = "<call:4>W1AW<app_qrzlog_logid:3>100<eor>\n\
                    <call:4>K5ZD<app_qrzlog_logid:10>1482539906<eor>\n";
        assert_eq!(page_stats(adif), (2, 1_482_539_906));
        // A record with no logid → count 1, cursor 0 (which stops paging).
        assert_eq!(page_stats("<call:4>W1AW<eor>"), (1, 0));
        assert_eq!(page_stats(""), (0, 0));
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
