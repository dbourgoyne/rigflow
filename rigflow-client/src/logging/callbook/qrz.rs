//! QRZ XML callsign lookup. Two-step: log in (username+password → session key),
//! then look up (`s=<key>;callsign=<call>`). Requires an XML Logbook Data
//! subscription. Session handling (cache + re-login) is in [`super::CallbookClient`];
//! this module owns URL building + XML parsing (pure, unit-tested).
//!
//! NB the credential here is the qrz.com **login** (XML API), NOT the QRZ Logbook
//! API KEY used for QSO sync.

use super::{CallbookResult, Provider, Source, redact, xml_tag};

const BASE: &str = "https://xmldata.qrz.com/xml/current/";

/// Log in and return the session key, or an error string.
pub fn login(agent: &ureq::Agent, user: &str, password: &str) -> Result<String, String> {
    // The spec shows `;`-separated params; ureq joins with `&` (and url-encodes),
    // which QRZ's ColdFusion accepts. Switch to a manual `;` URL if lookups fail.
    let body = agent
        .get(BASE)
        .query("username", user)
        .query("password", password)
        .query("agent", "rigflow")
        .call()
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading QRZ response: {e}"))?;
    match xml_tag(&body, "Key") {
        Some(key) => Ok(key),
        None => Err(xml_tag(&body, "Error").unwrap_or_else(|| "QRZ login failed".into())),
    }
}

/// Raw lookup response for `call` using session `key`.
pub fn lookup_raw(agent: &ureq::Agent, key: &str, call: &str) -> Result<String, String> {
    agent
        .get(BASE)
        .query("s", key)
        .query("callsign", call)
        .call()
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading QRZ response: {e}"))
}

/// Whether the response says the session expired/invalid (→ caller re-logs in).
pub fn session_expired(body: &str) -> bool {
    let e = xml_tag(body, "Error")
        .unwrap_or_default()
        .to_ascii_lowercase();
    e.contains("session") && (e.contains("timeout") || e.contains("invalid"))
}

/// Parse a lookup response. `Ok(Some)` = found, `Ok(None)` = not found, `Err` =
/// a real error (a session error is left for the caller's expiry check first).
pub fn parse_callsign(body: &str) -> Result<Option<CallbookResult>, String> {
    if let Some(err) = xml_tag(body, "Error") {
        let low = err.to_ascii_lowercase();
        if low.starts_with("not found") {
            return Ok(None);
        }
        return Err(err);
    }
    if xml_tag(body, "call").is_none() {
        return Ok(None);
    }
    Ok(Some(CallbookResult {
        name: full_name(body),
        grid: xml_tag(body, "grid"),
        qth: xml_tag(body, "addr2"), // city/town
        state: xml_tag(body, "state"),
        county: xml_tag(body, "county"),
        country: xml_tag(body, "country"),
        dxcc: xml_tag(body, "dxcc").and_then(|s| s.trim().parse().ok()),
        cq_zone: xml_tag(body, "cqzone"),
        itu_zone: xml_tag(body, "ituzone"),
        license_class: xml_tag(body, "class"),
        source: Source::Provider(Provider::Qrz),
    }))
}

/// `<fname>` + `<name>` (first + last).
fn full_name(body: &str) -> Option<String> {
    match (xml_tag(body, "fname"), xml_tag(body, "name")) {
        (Some(f), Some(l)) => Some(format!("{f} {l}")),
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_callsign() {
        let xml = "<QRZDatabase><Callsign><call>W1AW</call><fname>ARRL HQ</fname>\
                   <name>OPERATORS CLUB</name><addr2>Newington</addr2><state>CT</state>\
                   <county>Hartford</county><country>United States</country><dxcc>291</dxcc>\
                   <grid>FN31pr</grid><cqzone>5</cqzone><ituzone>8</ituzone><class>C</class>\
                   </Callsign></QRZDatabase>";
        let r = parse_callsign(xml).unwrap().unwrap();
        assert_eq!(r.name.as_deref(), Some("ARRL HQ OPERATORS CLUB"));
        assert_eq!(r.grid.as_deref(), Some("FN31pr"));
        assert_eq!(r.state.as_deref(), Some("CT"));
        assert_eq!(r.dxcc, Some(291));
        assert_eq!(r.cq_zone.as_deref(), Some("5"));
    }

    #[test]
    fn not_found_is_none_expiry_is_flagged() {
        let nf = "<QRZDatabase><Session><Error>Not found: g1srdd</Error></Session></QRZDatabase>";
        assert_eq!(parse_callsign(nf).unwrap(), None);
        let to = "<QRZDatabase><Session><Error>Session Timeout</Error></Session></QRZDatabase>";
        assert!(session_expired(to));
        assert!(!session_expired(nf));
    }
}
