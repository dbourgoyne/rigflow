//! eQSL.cc sync — download received QSLs (confirmations) and upload QSOs.
//! Username + password auth.
//!
//! Download is two-step: `DownloadInBox.cfm` builds an ADIF and returns an HTML
//! page linking to it; we follow the link. Upload posts the ADIF batch to
//! `ImportADIF.cfm`. The exact form fields and reply wording are eQSL-specific
//! and centralized in the small helpers here for easy adjustment after a live
//! run — see the module-level verification note in [`super::services`].

use std::time::Duration;

use super::services::UploadReport;

const BASE: &str = "https://www.eqsl.cc";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(90))
        .build()
}

/// Download the received-QSL inbox as ADIF (empty string if there's nothing new).
pub fn download(user: &str, password: &str) -> Result<String, String> {
    let a = agent();
    let page = a
        .get(&format!("{BASE}/qslcard/DownloadInBox.cfm"))
        .query("UserName", user)
        .query("Password", password)
        .query("QSLDetail", "1")
        .call()
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the eQSL response: {e}"))?;

    if let Some(rel) = find_adif_link(&page) {
        let url = format!("{BASE}{rel}");
        return a
            .get(&url)
            .call()
            .map_err(redact)?
            .into_string()
            .map_err(|e| format!("downloading the eQSL ADIF: {e}"));
    }
    // eQSL says so in prose when the inbox is empty.
    let low = page.to_ascii_lowercase();
    if low.contains("no ") && low.contains("adif") {
        return Ok(String::new());
    }
    Err(error_hint(&page))
}

/// Upload a batch of QSOs to eQSL. Best-effort count parse; the reply is prose.
pub fn upload(user: &str, password: &str, adif: &str) -> Result<UploadReport, String> {
    let body = agent()
        .post(&format!("{BASE}/qslcard/importADIF.cfm"))
        .send_form(&[
            ("UserName", user),
            ("Password", password),
            ("ADIFData", adif),
        ])
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the eQSL response: {e}"))?;
    parse_upload(&body)
}

/// Pull the generated ADIF path out of the DownloadInBox HTML. eQSL links it as
/// `…downloadedfiles/NAME.adi`; return it rooted at `/qslcard/`.
fn find_adif_link(html: &str) -> Option<String> {
    const KEY: &str = "downloadedfiles/";
    let start = html.find(KEY)?;
    let tail = &html[start..];
    let end = tail.find(".adi")? + 4;
    Some(format!("/qslcard/{}", &tail[..end]))
}

/// Best-effort read of the eQSL import reply. eQSL reports like "Result: N out
/// of M records added" with rejects called out; if we can't find numbers we keep
/// the whole (trimmed) message so the operator sees what eQSL said.
fn parse_upload(body: &str) -> Result<UploadReport, String> {
    let low = body.to_ascii_lowercase();
    if low.contains("error") && !low.contains("added") {
        return Err(error_hint(body));
    }
    let accepted = number_before(&low, "record").unwrap_or(0);
    let rejected = number_before(&low, "rejected").unwrap_or(0);
    Ok(UploadReport {
        accepted,
        rejected,
        message: first_meaningful_line(body),
    })
}

/// The integer immediately preceding the first occurrence of `word`.
fn number_before(haystack: &str, word: &str) -> Option<usize> {
    let at = haystack.find(word)?;
    haystack[..at]
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}

fn first_meaningful_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('<'))
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

fn error_hint(body: &str) -> String {
    let line = first_meaningful_line(body);
    if line.is_empty() {
        "eQSL returned an unexpected response — check your username and password.".into()
    } else {
        format!("eQSL: {line}")
    }
}

/// No URL (it carries the password) in a transport error.
fn redact(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("eQSL returned HTTP {code}."),
        ureq::Error::Transport(t) => format!("could not reach eQSL: {}", t.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_adif_link() {
        let html =
            r#"<html><a href="/qslcard/downloadedfiles/AB1CD_inbox.adi">Download</a></html>"#;
        assert_eq!(
            find_adif_link(html).as_deref(),
            Some("/qslcard/downloadedfiles/AB1CD_inbox.adi")
        );
        assert_eq!(find_adif_link("<html>no link</html>"), None);
    }

    #[test]
    fn parses_an_upload_count() {
        let r = parse_upload("Result: 3 out of 3 records added").unwrap();
        assert_eq!(r.accepted, 3);
        let err = parse_upload("Error: bad login").unwrap_err();
        assert!(err.to_lowercase().contains("bad login"));
    }
}
