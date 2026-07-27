//! HamQTH callsign lookup. Two-step: log in (username+password → `session_id`,
//! 1-hour TTL), then look up (`id=<sid>&callsign=<call>&prg=rigflow`). Free with
//! registration. Session handling is in [`super::CallbookClient`]; this module
//! owns URL building + XML parsing.

use super::{CallbookResult, Provider, Source, redact, xml_tag};

const BASE: &str = "https://www.hamqth.com/xml.php";

/// Log in and return the session id, or an error string.
pub fn login(agent: &ureq::Agent, user: &str, password: &str) -> Result<String, String> {
    let body = agent
        .get(BASE)
        .query("u", user)
        .query("p", password)
        .call()
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading HamQTH response: {e}"))?;
    match xml_tag(&body, "session_id") {
        Some(sid) => Ok(sid),
        None => Err(xml_tag(&body, "error").unwrap_or_else(|| "HamQTH login failed".into())),
    }
}

/// Raw lookup response for `call` using `session_id`.
pub fn lookup_raw(agent: &ureq::Agent, sid: &str, call: &str) -> Result<String, String> {
    agent
        .get(BASE)
        .query("id", sid)
        .query("callsign", call)
        .query("prg", "rigflow")
        .call()
        .map_err(redact)?
        .into_string()
        .map_err(|e| format!("reading HamQTH response: {e}"))
}

/// Whether the response says the session expired/invalid (→ caller re-logs in).
pub fn session_invalid(body: &str) -> bool {
    xml_tag(body, "error")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("session")
}

/// Parse a search response. `Ok(Some)` = found, `Ok(None)` = not found, `Err` =
/// a real error (a session error is left for the caller's check first).
pub fn parse_search(body: &str) -> Result<Option<CallbookResult>, String> {
    if let Some(err) = xml_tag(body, "error") {
        if err.to_ascii_lowercase().contains("not found") {
            return Ok(None);
        }
        return Err(err);
    }
    if xml_tag(body, "callsign").is_none() {
        return Ok(None);
    }
    Ok(Some(CallbookResult {
        // `adr_name` is the licensee's real name; `nick` is a handle.
        name: xml_tag(body, "adr_name").or_else(|| xml_tag(body, "nick")),
        grid: xml_tag(body, "grid"),
        qth: xml_tag(body, "qth"),
        state: xml_tag(body, "us_state"),
        county: xml_tag(body, "us_county"),
        country: xml_tag(body, "country"),
        dxcc: xml_tag(body, "adif").and_then(|s| s.trim().parse().ok()),
        cq_zone: xml_tag(body, "cq"),
        itu_zone: xml_tag(body, "itu"),
        license_class: None,
        source: Source::Provider(Provider::HamQTH),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_search() {
        let xml = "<HamQTH><search><callsign>ok7an</callsign><nick>Petr</nick>\
                   <adr_name>Petr Novak</adr_name><qth>Praha</qth><country>Czech Republic</country>\
                   <adif>503</adif><grid>JO70</grid><cq>15</cq><itu>28</itu></search></HamQTH>";
        let r = parse_search(xml).unwrap().unwrap();
        assert_eq!(r.name.as_deref(), Some("Petr Novak"));
        assert_eq!(r.grid.as_deref(), Some("JO70"));
        assert_eq!(r.dxcc, Some(503));
        assert_eq!(r.itu_zone.as_deref(), Some("28"));
    }

    #[test]
    fn not_found_and_session_error() {
        let nf = "<HamQTH><session><error>Callsign not found</error></session></HamQTH>";
        assert_eq!(parse_search(nf).unwrap(), None);
        assert!(!session_invalid(nf));
        let se =
            "<HamQTH><session><error>Session does not exist or expired</error></session></HamQTH>";
        assert!(session_invalid(se));
    }
}
