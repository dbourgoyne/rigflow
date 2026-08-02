//! Callook.info callsign lookup — US only, free, no auth. `GET
//! https://callook.info/<call>/json`. A valid response is by definition a US
//! call, so DXCC is 291 / United States; city + state come from the address.

use serde::Deserialize;

use super::{CallbookResult, Provider, Source};

const BASE: &str = "https://callook.info";

#[derive(Deserialize)]
struct Resp {
    status: String,
    name: Option<String>,
    address: Option<Address>,
    location: Option<Location>,
    current: Option<Current>,
}

#[derive(Deserialize)]
struct Address {
    line2: Option<String>, // "CITY, ST ZIP"
}

#[derive(Deserialize)]
struct Location {
    gridsquare: Option<String>,
}

#[derive(Deserialize)]
struct Current {
    #[serde(rename = "operClass")]
    oper_class: Option<String>,
}

/// Look up a US callsign. `Ok(Some)` = valid US call, `Ok(None)` = not a valid
/// US call (or Callook has nothing), `Err` = transport/parse error.
pub fn lookup(agent: &ureq::Agent, call: &str) -> Result<Option<CallbookResult>, String> {
    let url = format!("{BASE}/{call}/json");
    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(super::redact(e)),
    };
    let body = resp
        .into_string()
        .map_err(|e| format!("reading Callook response: {e}"))?;
    let r: Resp = serde_json::from_str(&body).map_err(|e| format!("parsing Callook JSON: {e}"))?;
    if !r.status.eq_ignore_ascii_case("VALID") {
        return Ok(None);
    }
    let (qth, state) = r
        .address
        .and_then(|a| a.line2)
        .map(|l| split_city_state(&l))
        .unwrap_or((None, None));
    Ok(Some(CallbookResult {
        name: r.name.filter(|s| !s.trim().is_empty()),
        grid: r
            .location
            .and_then(|l| l.gridsquare)
            .filter(|s| !s.trim().is_empty()),
        qth,
        state,
        county: None,
        country: Some("United States".into()),
        dxcc: Some(291), // a valid Callook result is a US call
        cq_zone: None,   // varies by call area — leave to the prefix baseline/provider
        itu_zone: None,
        license_class: r.current.and_then(|c| c.oper_class),
        source: Source::Provider(Provider::Callook),
    }))
}

/// Split "NEWINGTON, CT 06111" → (Some("NEWINGTON"), Some("CT")).
fn split_city_state(line2: &str) -> (Option<String>, Option<String>) {
    let Some((city, rest)) = line2.split_once(',') else {
        return (None, None);
    };
    let city = city.trim();
    let state = rest.split_whitespace().next().unwrap_or("");
    (
        (!city.is_empty()).then(|| city.to_string()),
        (!state.is_empty()).then(|| state.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_city_and_state() {
        assert_eq!(
            split_city_state("NEWINGTON, CT 06111"),
            (Some("NEWINGTON".into()), Some("CT".into()))
        );
        assert_eq!(split_city_state("nowhere"), (None, None));
    }
}
