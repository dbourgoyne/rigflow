//! Core logging data model: a normalized [`Qso`] and the [`Station`] profile.
//!
//! A `Qso` is the single normalized-contact shape that every capture path
//! (manual entry, ADIF file import, WSJT-X ingest) converges on before it hits
//! the store. Columns mirror the SQLite schema; every ADIF field we don't model
//! as a column round-trips through [`Qso::extra`] so nothing is dropped.

use std::collections::BTreeMap;

use crate::normalize;

/// The logging station's identity. In rigflow the **callsign is per-operator**
/// (`station_call` = the operator id) and the **location is global** (grid,
/// state, county, zones, name apply to the whole physical station). These
/// values are snapshotted onto each QSO at log time so later edits never
/// rewrite history.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Station {
    pub station_call: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gridsquare: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_county: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cq_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub itu_zone: Option<String>,
}

impl Station {
    /// The `MY_*` ADIF fields this station contributes to a logged QSO's
    /// `extra` map. `MY_GRIDSQUARE` etc. are historical truth once copied.
    pub fn my_adif_fields(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        // STATION_CALLSIGN is the QSO-level "who transmitted"; also emit
        // OPERATOR so single-op logs are unambiguous.
        if !self.station_call.trim().is_empty() {
            m.insert(
                "STATION_CALLSIGN".into(),
                self.station_call.trim().to_string(),
            );
            m.insert("OPERATOR".into(), self.station_call.trim().to_string());
        }
        for (k, v) in [
            ("MY_GRIDSQUARE", &self.gridsquare),
            ("MY_NAME", &self.name),
            ("MY_STATE", &self.my_state),
            ("MY_CNTY", &self.my_county),
            ("MY_CQ_ZONE", &self.cq_zone),
            ("MY_ITU_ZONE", &self.itu_zone),
        ] {
            if let Some(v) = v.as_ref().filter(|s| !s.trim().is_empty()) {
                m.insert(k.to_string(), v.trim().to_string());
            }
        }
        m
    }
}

/// A normalized contact. Times are UTC, ADIF-native (`YYYYMMDD` / `HHMMSS`).
///
/// Split semantics (ADIF): `freq_hz`/`band` are the **transmit** frequency/band
/// (where *you* called); `freq_rx_hz`/`band_rx` are the **receive**
/// frequency/band (the DX station's transmit frequency) and are `None` on a
/// simplex QSO. `band` is derived from `freq_hz` and `band_rx` from
/// `freq_rx_hz`, independently.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Qso {
    pub call: String,
    pub qso_date: String,
    pub time_on: String,
    pub band: String,
    pub mode: String,
    pub submode: Option<String>,
    pub freq_hz: Option<u64>,
    pub freq_rx_hz: Option<u64>,
    pub band_rx: Option<String>,
    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,
    pub gridsquare: Option<String>,
    pub dxcc: Option<i64>,
    /// Every ADIF field not held in a column above, keyed by upper-case ADIF
    /// field name (includes `APP_*` extensions, `*_INTL` fields, and the
    /// snapshotted `MY_*` station fields). Ordered for deterministic output.
    pub extra: BTreeMap<String, String>,
}

impl Qso {
    /// Fill in derived/canonical fields:
    /// - normalize `mode`/`submode`,
    /// - derive `band` from `freq_hz` when band is blank,
    /// - derive `band_rx` from `freq_rx_hz` when a split RX freq is present.
    ///
    /// Idempotent — safe to call on already-normalized input.
    pub fn normalize(&mut self) {
        let (mode, submode) = normalize::normalize_mode(&self.mode, self.submode.as_deref());
        self.mode = mode;
        self.submode = submode;

        if self.band.trim().is_empty()
            && let Some(b) = self.freq_hz.and_then(normalize::band_for_freq_hz)
        {
            self.band = b.to_string();
        }
        // band_rx is derived from freq_rx independently; only meaningful when a
        // split RX frequency exists.
        if self.band_rx.is_none()
            && let Some(b) = self.freq_rx_hz.and_then(normalize::band_for_freq_hz)
        {
            self.band_rx = Some(b.to_string());
        }
    }
}
