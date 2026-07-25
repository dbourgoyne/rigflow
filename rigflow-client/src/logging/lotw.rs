//! LoTW (ARRL Logbook of the World) confirmation **download**.
//!
//! Download-only: fetch the QSL confirmation report over HTTPS and hand the ADIF
//! to the shared import pipeline ([`rigflow_log::import`]). This needs only a
//! username + password. *Upload* is deliberately out of scope — LoTW upload
//! requires a TQSL-signed file and a certificate, a different and much larger
//! problem than a signed-in GET.

use std::time::Duration;

/// The confirmation-report endpoint.
const REPORT_URL: &str = "https://lotw.arrl.org/lotwuser/lotwreport.adi";

/// Fetch the LoTW confirmation report as ADIF text.
///
/// `since` is the incremental marker (`qso_qslsince`) — the `APP_LoTW_LASTQSL`
/// timestamp from the previous run, so only newer confirmations come back.
/// `None` pulls the whole confirmed set (a first sync). Only confirmed QSOs are
/// requested (`qso_qsl=yes`), with detail so each record carries `QSL_RCVD` /
/// `QSLRDATE` for the importer to read.
///
/// The password travels as a query parameter (over TLS); it is never placed in
/// the returned error string — see [`redact_error`].
pub fn fetch_report(login: &str, password: &str, since: Option<&str>) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build();
    let mut req = agent
        .get(REPORT_URL)
        .query("login", login)
        .query("password", password)
        .query("qso_query", "1")
        .query("qso_qsl", "yes")
        .query("qso_qsldetail", "yes");
    if let Some(s) = since {
        req = req.query("qso_qslsince", s);
    }
    let resp = req.call().map_err(redact_error)?;
    let body = resp
        .into_string()
        .map_err(|e| format!("reading the LoTW response: {e}"))?;
    validate(body)
}

/// LoTW answers a bad login with an HTML/plain error page (often HTTP 200), not
/// ADIF. A real report always contains an `<eoh>` header terminator, so use that
/// as the success test and surface a trimmed first line otherwise.
fn validate(body: String) -> Result<String, String> {
    if body.to_ascii_lowercase().contains("<eoh>") {
        return Ok(body);
    }
    let snippet: String = body
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    if snippet.is_empty() {
        Err("LoTW returned an empty response — check your username and password.".into())
    } else {
        Err(format!(
            "LoTW did not return a log (check your username and password). It said: {snippet}"
        ))
    }
}

/// Turn a `ureq` transport error into a message with **no URL in it** — the URL
/// carries the password as a query parameter, so its `Display` must never reach
/// the user or a log.
fn redact_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("LoTW returned HTTP {code}."),
        ureq::Error::Transport(t) => format!("could not reach LoTW: {}", t.kind()),
    }
}

/// Pull the `APP_LoTW_LASTQSL` timestamp from a report header — the marker to
/// pass as `qso_qslsince` next run. `None` if absent (an empty report has none;
/// the caller then keeps its previous marker). Length-prefixed parse, so a value
/// containing spaces (it does: `YYYY-MM-DD HH:MM:SS`) is read whole.
pub fn extract_lastqsl(adif: &str) -> Option<String> {
    const KEY: &str = "<APP_LoTW_LASTQSL:";
    let at = adif.find(KEY)? + KEY.len();
    let rest = &adif[at..];
    let gt = rest.find('>')?;
    let len: usize = rest[..gt].split(':').next()?.trim().parse().ok()?;
    let vstart = at + gt + 1;
    adif.get(vstart..vstart + len).map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_lastqsl_marker() {
        let hdr = "ARRL Logbook of the World Status Report\n\
                   <PROGRAMID:4>LoTW\n<APP_LoTW_LASTQSL:19>2026-07-23 20:45:06\n<eoh>\n";
        assert_eq!(extract_lastqsl(hdr).as_deref(), Some("2026-07-23 20:45:06"));
    }

    #[test]
    fn no_marker_when_absent() {
        assert_eq!(extract_lastqsl("<PROGRAMID:4>LoTW\n<eoh>\n"), None);
    }

    #[test]
    fn validate_accepts_a_real_report_and_rejects_an_error_page() {
        assert!(validate("<PROGRAMID:4>LoTW\n<eoh>\n<CALL:4>W1AW<eor>".into()).is_ok());
        let err = validate("Username/password incorrect".into()).unwrap_err();
        assert!(err.contains("username and password"));
        assert!(validate("   ".into()).is_err());
    }
}
