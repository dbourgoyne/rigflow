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

    if let Some(href) = find_adif_link(&page) {
        return a
            .get(&resolve(&href))
            .call()
            .map_err(redact)?
            .into_string()
            .map_err(|e| format!("downloading the eQSL ADIF: {e}"));
    }
    // A successful build with no cards, or an empty inbox: not an error.
    let low = page.to_ascii_lowercase();
    if low.contains("has been built") || (low.contains("no ") && low.contains("adif")) {
        return Ok(String::new());
    }
    Err(error_hint(&page))
}

/// Upload a batch of QSOs to eQSL. Best-effort count parse; the reply is prose.
///
/// Credentials use eQSL's own field names `EQSL_USER` / `EQSL_PSWD` (not
/// `UserName`/`Password` — that's the download endpoint), with the batch in
/// `ADIFData`.
pub fn upload(user: &str, password: &str, adif: &str) -> Result<UploadReport, String> {
    let body = agent()
        .post(&format!("{BASE}/qslcard/importADIF.cfm"))
        .send_form(&[
            ("EQSL_USER", user),
            ("EQSL_PSWD", password),
            ("ADIFData", adif),
        ])
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading the eQSL response: {e}"))?;
    parse_upload(&body)
}

/// The `href` of the built ADIF from the DownloadInBox HTML — read from the page
/// rather than reconstructed, because eQSL moved `DownloadedFiles` out of the
/// `qslcard/` folder in 2019, so a hard-coded path 404s. Returns the raw href
/// (absolute URL, `/`-rooted, or relative); [`resolve`] turns it into a URL.
fn find_adif_link(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(".adi") {
        let end = from + rel + 4;
        // Walk back to the quote that opened this href value.
        if let Some(q) = html[..end].rfind(['"', '\'']) {
            let href = &html[q + 1..end];
            if !href.is_empty() && !href.contains('<') && !href.contains('>') {
                return Some(href.to_string());
            }
        }
        from = end;
    }
    None
}

/// Turn a page href into an absolute URL against eQSL's host.
fn resolve(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(rooted) = href.strip_prefix('/') {
        format!("{BASE}/{rooted}")
    } else {
        // Relative to /qslcard/ (where DownloadInBox.cfm lives), so "../x" climbs out.
        format!("{BASE}/qslcard/{href}")
    }
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
        // The real href reads straight from the page (case-insensitive tag).
        let html = r#"Your ADIF log file has been built.
            <A HREF="/downloadedfiles/AB1CD_inbox.adi">ADI</A>
            <a href="/downloadedfiles/AB1CD_inbox.txt">TXT</a>"#;
        assert_eq!(
            find_adif_link(html).as_deref(),
            Some("/downloadedfiles/AB1CD_inbox.adi")
        );
        assert_eq!(find_adif_link("<html>no link</html>"), None);
    }

    #[test]
    fn resolves_hrefs_against_the_host() {
        // The post-2019 relocated path lives above qslcard/, so a /-rooted href
        // must NOT get a qslcard prefix (that was the 404 bug).
        assert_eq!(
            resolve("/downloadedfiles/x.adi"),
            "https://www.eqsl.cc/downloadedfiles/x.adi"
        );
        assert_eq!(
            resolve("https://www.eqsl.cc/downloadedfiles/x.adi"),
            "https://www.eqsl.cc/downloadedfiles/x.adi"
        );
        assert_eq!(
            resolve("../downloadedfiles/x.adi"),
            "https://www.eqsl.cc/qslcard/../downloadedfiles/x.adi"
        );
    }

    #[test]
    fn parses_an_upload_count() {
        let r = parse_upload("Result: 3 out of 3 records added").unwrap();
        assert_eq!(r.accepted, 3);
        let err = parse_upload("Error: bad login").unwrap_err();
        assert!(err.to_lowercase().contains("bad login"));
    }
}
