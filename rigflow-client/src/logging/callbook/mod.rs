//! Callbook lookup — enrich a QSO with the operator's details (name, grid, QTH,
//! DXCC/zones) from an online callbook, with an always-on offline prefix baseline.
//!
//! Design: `docs/release-review/callbook-lookup-design.md`. This module is the
//! **pure core** — providers ([`qrz`], [`hamqth`], [`callook`]), the offline
//! [`prefix`] baseline, and the [`CallbookClient`] orchestration (priority order,
//! first-match-wins, session caching). The worker wiring and the `L`-window merge
//! live elsewhere (see the design's §7/§8).
//!
//! Providers differ in auth (QRZ + HamQTH need a session from username+password;
//! Callook is no-auth, US-only) but every one returns a normalized
//! [`CallbookResult`]. The **merge** (Typed > Online > Prefix) happens in the UI
//! against the draft; here we just produce results.

pub mod callook;
pub mod hamqth;
pub mod prefix;
pub mod qrz;

/// An online callbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Qrz,
    HamQTH,
    Callook,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Qrz, Provider::HamQTH, Provider::Callook];

    /// Stable config key.
    pub fn key(self) -> &'static str {
        match self {
            Provider::Qrz => "qrz",
            Provider::HamQTH => "hamqth",
            Provider::Callook => "callook",
        }
    }

    pub fn from_key(k: &str) -> Option<Provider> {
        Provider::ALL.into_iter().find(|p| p.key() == k)
    }

    /// Credential-store key (distinct from the sync service keys). Callook has none.
    pub fn cred_key(self) -> Option<&'static str> {
        match self {
            Provider::Qrz => Some("qrz-xml"),
            Provider::HamQTH => Some("hamqth"),
            Provider::Callook => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Qrz => "QRZ",
            Provider::HamQTH => "HamQTH",
            Provider::Callook => "Callook",
        }
    }
}

/// Per-operator callbook configuration: which providers are on, and in what
/// priority order. Persisted as `callbook.json` in the operator directory
/// (credentials themselves live in the keyring/file store, not here).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallbookConfig {
    #[serde(default = "default_order")]
    pub order: Vec<String>,
    #[serde(default)]
    pub enabled: Vec<String>,
}

fn default_order() -> Vec<String> {
    vec!["qrz".into(), "hamqth".into(), "callook".into()]
}

impl Default for CallbookConfig {
    fn default() -> Self {
        CallbookConfig {
            order: default_order(),
            enabled: Vec::new(),
        }
    }
}

impl CallbookConfig {
    pub fn is_enabled(&self, p: Provider) -> bool {
        self.enabled.iter().any(|k| k == p.key())
    }

    pub fn set_enabled(&mut self, p: Provider, on: bool) {
        self.enabled.retain(|k| k != p.key());
        if on {
            self.enabled.push(p.key().to_string());
        }
    }

    /// Providers in priority order (unknown keys skipped, missing ones appended).
    pub fn ordered(&self) -> Vec<Provider> {
        let mut out: Vec<Provider> = self
            .order
            .iter()
            .filter_map(|k| Provider::from_key(k))
            .collect();
        for p in Provider::ALL {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// Enabled providers in priority order — what the worker actually queries.
    pub fn active(&self) -> Vec<Provider> {
        self.ordered()
            .into_iter()
            .filter(|p| self.is_enabled(*p))
            .collect()
    }

    pub fn move_up(&mut self, p: Provider) {
        self.normalize_order();
        if let Some(i) = self.order.iter().position(|k| k == p.key())
            && i > 0
        {
            self.order.swap(i, i - 1);
        }
    }

    pub fn move_down(&mut self, p: Provider) {
        self.normalize_order();
        if let Some(i) = self.order.iter().position(|k| k == p.key())
            && i + 1 < self.order.len()
        {
            self.order.swap(i, i + 1);
        }
    }

    /// Ensure `order` lists every provider exactly once (self-heal a hand-edited
    /// or older file) before a swap.
    fn normalize_order(&mut self) {
        self.order = self.ordered().iter().map(|p| p.key().to_string()).collect();
    }
}

const CONFIG_FILE: &str = "callbook.json";

pub fn load_config(operator_dir: &std::path::Path) -> CallbookConfig {
    std::fs::read_to_string(operator_dir.join(CONFIG_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(operator_dir: &std::path::Path, cfg: &CallbookConfig) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default();
    std::fs::write(operator_dir.join(CONFIG_FILE), json)
}

/// Loaded credentials for the auth'd providers. A provider with `None` here is
/// skipped even if it appears in the priority order.
#[derive(Debug, Clone, Default)]
pub struct CallbookCreds {
    pub qrz: Option<super::credentials::Credential>,
    pub hamqth: Option<super::credentials::Credential>,
}

/// A normalized callbook result. Every field is optional — different providers
/// and the offline prefix baseline fill different subsets. `None` means "this
/// source had nothing for this field", which is what lets the UI merge sources
/// per-field (online overwrites prefix only where online actually has a value).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbookResult {
    pub name: Option<String>,
    pub grid: Option<String>,
    pub qth: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub country: Option<String>,
    pub dxcc: Option<i64>,
    pub cq_zone: Option<String>,
    pub itu_zone: Option<String>,
    pub license_class: Option<String>,
    /// Which source produced this (for the UI's "via QRZ / via prefix" note).
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Prefix,
    Provider(Provider),
}

impl Source {
    /// Short label for the "via …" note — the provider's name, or "prefix".
    pub fn label(self) -> &'static str {
        match self {
            Source::Prefix => "prefix",
            Source::Provider(p) => p.name(),
        }
    }
}

impl CallbookResult {
    /// Nothing useful was found.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.grid.is_none()
            && self.qth.is_none()
            && self.state.is_none()
            && self.county.is_none()
            && self.country.is_none()
            && self.dxcc.is_none()
            && self.cq_zone.is_none()
            && self.itu_zone.is_none()
    }

    /// The ADIF `extra` fields this result contributes (everything that is not a
    /// modeled `Qso` column — `gridsquare`/`dxcc` are columns and handled by the
    /// caller). Only present values are emitted.
    pub fn extra_fields(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let mut push = |k: &'static str, v: &Option<String>| {
            if let Some(v) = v.as_ref().filter(|s| !s.trim().is_empty()) {
                out.push((k, v.trim().to_string()));
            }
        };
        push("NAME", &self.name);
        push("QTH", &self.qth);
        push("STATE", &self.state);
        push("CNTY", &self.county);
        push("COUNTRY", &self.country);
        push("CQZ", &self.cq_zone);
        push("ITUZ", &self.itu_zone);
        out
    }
}

/// Overlay `online` onto `prefix`, per field — online wins where it has a value,
/// prefix fills the rest. Either may be absent. This is how the instant offline
/// floor and the async provider result combine into one effective result.
pub fn merge(
    prefix: Option<CallbookResult>,
    online: Option<CallbookResult>,
) -> Option<CallbookResult> {
    match (prefix, online) {
        (None, None) => None,
        (Some(r), None) | (None, Some(r)) => Some(r),
        (Some(p), Some(o)) => Some(CallbookResult {
            name: o.name.or(p.name),
            grid: o.grid.or(p.grid),
            qth: o.qth.or(p.qth),
            state: o.state.or(p.state),
            county: o.county.or(p.county),
            country: o.country.or(p.country),
            dxcc: o.dxcc.or(p.dxcc),
            cq_zone: o.cq_zone.or(p.cq_zone),
            itu_zone: o.itu_zone.or(p.itu_zone),
            license_class: o.license_class.or(p.license_class),
            source: o.source, // the online provider that supplied the overlay
        }),
    }
}

/// Holds the ureq agent and cached provider sessions. The worker owns one; the
/// sessions persist across lookups and are re-acquired lazily when a provider
/// reports an expired/invalid session.
pub struct CallbookClient {
    agent: ureq::Agent,
    qrz_session: Option<String>,
    hamqth_session: Option<String>,
}

impl Default for CallbookClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CallbookClient {
    pub fn new() -> Self {
        CallbookClient {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(15))
                .build(),
            qrz_session: None,
            hamqth_session: None,
        }
    }

    /// Look a callsign up, trying each enabled provider in `order` until one
    /// returns a non-empty result. A provider that errors or has no match is
    /// skipped (errors are logged, redacted). `None` = nobody had it.
    pub fn lookup(
        &mut self,
        order: &[Provider],
        creds: &CallbookCreds,
        call: &str,
    ) -> Option<CallbookResult> {
        let call = call.trim().to_ascii_uppercase();
        if call.is_empty() {
            return None;
        }
        for &p in order {
            let got = match p {
                Provider::Qrz => match &creds.qrz {
                    Some(c) => self.qrz_lookup(&c.login, &c.password, &call),
                    None => continue,
                },
                Provider::HamQTH => match &creds.hamqth {
                    Some(c) => self.hamqth_lookup(&c.login, &c.password, &call),
                    None => continue,
                },
                Provider::Callook => callook::lookup(&self.agent, &call),
            };
            match got {
                Ok(Some(r)) if !r.is_empty() => return Some(r),
                Ok(_) => {} // not found → next provider
                Err(e) => eprintln!("callbook: {} lookup failed: {e}", p.name()),
            }
        }
        None
    }

    /// QRZ XML: cached session key, re-login once on an expired/invalid session.
    fn qrz_lookup(
        &mut self,
        user: &str,
        password: &str,
        call: &str,
    ) -> Result<Option<CallbookResult>, String> {
        for attempt in 0..2 {
            if self.qrz_session.is_none() {
                self.qrz_session = Some(qrz::login(&self.agent, user, password)?);
            }
            let key = self.qrz_session.clone().unwrap();
            let body = qrz::lookup_raw(&self.agent, &key, call)?;
            if qrz::session_expired(&body) && attempt == 0 {
                self.qrz_session = None; // force re-login and retry once
                continue;
            }
            return qrz::parse_callsign(&body);
        }
        Ok(None)
    }

    /// HamQTH: cached session id, re-login once on an invalid session.
    fn hamqth_lookup(
        &mut self,
        user: &str,
        password: &str,
        call: &str,
    ) -> Result<Option<CallbookResult>, String> {
        for attempt in 0..2 {
            if self.hamqth_session.is_none() {
                self.hamqth_session = Some(hamqth::login(&self.agent, user, password)?);
            }
            let sid = self.hamqth_session.clone().unwrap();
            let body = hamqth::lookup_raw(&self.agent, &sid, call)?;
            if hamqth::session_invalid(&body) && attempt == 0 {
                self.hamqth_session = None;
                continue;
            }
            return hamqth::parse_search(&body);
        }
        Ok(None)
    }
}

// ---- shared parse/redact helpers used by the XML providers ----

/// Extract the text of the first `<tag>…</tag>` (case-insensitive tag match),
/// trimmed, `None` if absent or empty. Good enough for QRZ/HamQTH's flat XML —
/// no XML dependency, no nesting to worry about for the fields we read.
pub(crate) fn xml_tag(body: &str, tag: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = format!("<{}>", tag.to_ascii_lowercase());
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find(&close)? + start;
    let v = body.get(start..end)?.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Redact a ureq transport error — the QRZ/HamQTH login URLs carry the password
/// as a query param, so their `Display` must never reach a log or the UI.
pub(crate) fn redact(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => format!("network error ({})", t.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_tag_extracts_case_insensitively() {
        let xml = "<Callsign><call>W1AW</call><grid>FN31pr</grid></Callsign>";
        assert_eq!(xml_tag(xml, "call").as_deref(), Some("W1AW"));
        assert_eq!(xml_tag(xml, "GRID").as_deref(), Some("FN31pr"));
        assert_eq!(xml_tag(xml, "name"), None);
    }

    #[test]
    fn extra_fields_emits_only_present_values() {
        let r = CallbookResult {
            name: Some("Dave".into()),
            qth: Some("  ".into()), // blank → skipped
            state: Some("CT".into()),
            ..Default::default()
        };
        let f = r.extra_fields();
        assert!(f.contains(&("NAME", "Dave".to_string())));
        assert!(f.contains(&("STATE", "CT".to_string())));
        assert!(!f.iter().any(|(k, _)| *k == "QTH"));
    }
}
