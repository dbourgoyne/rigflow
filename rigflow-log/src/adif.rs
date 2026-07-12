//! Hand-rolled ADIF parser and writer.
//!
//! ADIF fields are length-prefixed: `<NAME:LEN>value` or `<NAME:LEN:TYPE>value`,
//! where `LEN` is the **byte** count of `value` (so `*_INTL` UTF-8 fields work
//! unchanged). Control tags `<EOH>` (end of header) and `<EOR>` (end of record)
//! carry no length. Text outside tags is a comment and ignored.
//!
//! We parse to [`AdifRecord`] (upper-cased field name → value) and map to/from
//! [`Qso`]: modeled fields become columns, everything else (including `APP_*`
//! extensions and `*_INTL` fields) round-trips through [`Qso::extra`].

use std::collections::BTreeMap;

use crate::model::Qso;

/// A raw ADIF record: upper-cased field name → value.
pub type AdifRecord = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdifError {
    /// A `<` tag was opened but never closed with `>`.
    UnterminatedTag,
    /// A field tag was missing its `:LEN` (only `EOH`/`EOR` may be lengthless).
    MissingLength(String),
    /// `LEN` was not a valid number.
    BadLength(String),
    /// The declared length ran past the end of the input.
    TruncatedValue {
        field: String,
        want: usize,
        have: usize,
    },
    /// A field value was not valid UTF-8.
    NonUtf8(String),
}

impl std::fmt::Display for AdifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdifError::UnterminatedTag => write!(f, "unterminated ADIF tag (missing '>')"),
            AdifError::MissingLength(t) => write!(f, "ADIF field <{t}> missing length"),
            AdifError::BadLength(t) => write!(f, "ADIF field <{t}> has an invalid length"),
            AdifError::TruncatedValue { field, want, have } => {
                write!(
                    f,
                    "ADIF field {field} declares {want} bytes, only {have} left"
                )
            }
            AdifError::NonUtf8(field) => write!(f, "ADIF field {field} value is not valid UTF-8"),
        }
    }
}

impl std::error::Error for AdifError {}

/// The ADIF fields modeled as `Qso` columns; every other field goes to `extra`.
const COLUMN_FIELDS: &[&str] = &[
    "CALL",
    "QSO_DATE",
    "TIME_ON",
    "BAND",
    "MODE",
    "SUBMODE",
    "FREQ",
    "FREQ_RX",
    "BAND_RX",
    "RST_SENT",
    "RST_RCVD",
    "GRIDSQUARE",
    "DXCC",
];

/// Parse an ADIF document into its records. A leading header terminated by
/// `<EOH>` is discarded; if there is no `<EOH>`, the whole document is records.
pub fn parse_adif(text: &str) -> Result<Vec<AdifRecord>, AdifError> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut records: Vec<AdifRecord> = Vec::new();
    let mut current: AdifRecord = BTreeMap::new();

    while i < bytes.len() {
        // Advance to the next tag; everything before '<' is comment text.
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Find the closing '>'.
        let close = bytes[i + 1..]
            .iter()
            .position(|&b| b == b'>')
            .map(|p| i + 1 + p)
            .ok_or(AdifError::UnterminatedTag)?;
        let inner = &text[i + 1..close];
        i = close + 1;

        let mut parts = inner.splitn(3, ':');
        let name = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        let len_part = parts.next();
        // parts.next() would be the optional TYPE indicator; we don't need it.

        match (name.as_str(), len_part) {
            ("EOH", _) => {
                // End of header — discard anything accumulated as header fields.
                current.clear();
            }
            ("EOR", _) => {
                if !current.is_empty() {
                    records.push(std::mem::take(&mut current));
                }
            }
            (_, None) => return Err(AdifError::MissingLength(name)),
            (_, Some(len_str)) => {
                let len: usize = len_str
                    .trim()
                    .parse()
                    .map_err(|_| AdifError::BadLength(name.clone()))?;
                let end = i + len;
                if end > bytes.len() {
                    return Err(AdifError::TruncatedValue {
                        field: name,
                        want: len,
                        have: bytes.len() - i,
                    });
                }
                let value = std::str::from_utf8(&bytes[i..end])
                    .map_err(|_| AdifError::NonUtf8(name.clone()))?
                    .to_string();
                i = end;
                current.insert(name, value);
            }
        }
    }
    // Tolerate a final record with no trailing <EOR> only if we already saw
    // records (otherwise a bare header without <EOH> would masquerade as one).
    if !current.is_empty() && !records.is_empty() {
        records.push(current);
    }
    Ok(records)
}

/// The single ingest pipeline: ADIF text → normalized [`Qso`]s. File import and
/// WSJT-X `LoggedADIF` both go through here, so mode/band normalization and
/// band-from-freq derivation are applied uniformly.
pub fn parse_adif_to_qsos(text: &str) -> Result<Vec<Qso>, AdifError> {
    Ok(parse_adif(text)?
        .iter()
        .map(|r| {
            let mut q = record_to_qso(r);
            q.normalize();
            q
        })
        .collect())
}

/// The ADIF header written once at journal creation.
pub fn adif_header() -> String {
    "<ADIF_VER:5>3.1.6 <PROGRAMID:7>rigflow <EOH>\n".to_string()
}

/// Serialize one record as `<NAME:LEN>value…<EOR>`. Fields are emitted in
/// sorted (deterministic) order so the writer is stable across round-trips.
pub fn write_record(record: &AdifRecord) -> String {
    let mut out = String::new();
    for (name, value) in record {
        out.push_str(&format!("<{}:{}>{}", name, value.len(), value));
        out.push(' ');
    }
    out.push_str("<EOR>\n");
    out
}

/// Convert a [`Qso`] to an [`AdifRecord`]. Modeled columns become their ADIF
/// fields; `extra` entries are copied verbatim. `FREQ`/`FREQ_RX` are emitted in
/// MHz per ADIF; `BAND_RX`/`FREQ_RX` are omitted on a simplex QSO.
pub fn qso_to_record(q: &Qso) -> AdifRecord {
    let mut r: AdifRecord = BTreeMap::new();
    r.insert("CALL".into(), q.call.clone());
    r.insert("QSO_DATE".into(), q.qso_date.clone());
    r.insert("TIME_ON".into(), q.time_on.clone());
    if !q.band.is_empty() {
        r.insert("BAND".into(), q.band.clone());
    }
    r.insert("MODE".into(), q.mode.clone());
    if let Some(sm) = &q.submode {
        r.insert("SUBMODE".into(), sm.clone());
    }
    if let Some(hz) = q.freq_hz {
        r.insert("FREQ".into(), hz_to_mhz_string(hz));
    }
    // Split-only fields: present iff a real split RX frequency exists.
    if let Some(hz) = q.freq_rx_hz {
        r.insert("FREQ_RX".into(), hz_to_mhz_string(hz));
        if let Some(b) = &q.band_rx {
            r.insert("BAND_RX".into(), b.clone());
        }
    }
    for (k, v) in [
        ("RST_SENT", &q.rst_sent),
        ("RST_RCVD", &q.rst_rcvd),
        ("GRIDSQUARE", &q.gridsquare),
    ] {
        if let Some(v) = v {
            r.insert(k.into(), v.clone());
        }
    }
    if let Some(dxcc) = q.dxcc {
        r.insert("DXCC".into(), dxcc.to_string());
    }
    for (k, v) in &q.extra {
        r.entry(k.clone()).or_insert_with(|| v.clone());
    }
    r
}

/// Convert an [`AdifRecord`] to a [`Qso`]. Modeled fields fill columns; all
/// other fields (incl. `APP_*`, `*_INTL`, `MY_*`) go to `extra`. Does **not**
/// normalize — callers that want canonical mode/band call [`Qso::normalize`].
pub fn record_to_qso(r: &AdifRecord) -> Qso {
    let get = |k: &str| r.get(k).cloned();
    let mut extra = BTreeMap::new();
    for (k, v) in r {
        if !COLUMN_FIELDS.contains(&k.as_str()) {
            extra.insert(k.clone(), v.clone());
        }
    }
    Qso {
        call: get("CALL").unwrap_or_default(),
        qso_date: get("QSO_DATE").unwrap_or_default(),
        time_on: get("TIME_ON").unwrap_or_default(),
        band: get("BAND").unwrap_or_default(),
        mode: get("MODE").unwrap_or_default(),
        submode: get("SUBMODE"),
        freq_hz: get("FREQ").as_deref().and_then(mhz_string_to_hz),
        freq_rx_hz: get("FREQ_RX").as_deref().and_then(mhz_string_to_hz),
        band_rx: get("BAND_RX"),
        rst_sent: get("RST_SENT"),
        rst_rcvd: get("RST_RCVD"),
        gridsquare: get("GRIDSQUARE"),
        dxcc: get("DXCC").and_then(|s| s.trim().parse().ok()),
        extra,
    }
}

/// Hz → ADIF `FREQ` (MHz, 6 decimal places = 1 Hz resolution).
pub fn hz_to_mhz_string(hz: u64) -> String {
    format!("{:.6}", hz as f64 / 1_000_000.0)
}

/// ADIF `FREQ` (MHz) → Hz, rounded to the nearest Hz. Returns `None` if the
/// value isn't a number.
pub fn mhz_string_to_hz(mhz: &str) -> Option<u64> {
    let v: f64 = mhz.trim().parse().ok()?;
    if v < 0.0 {
        return None;
    }
    Some((v * 1_000_000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_fields_and_lengths() {
        let recs = parse_adif("<CALL:4>W1AW <QSO_DATE:8>20260711<EOR>").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["CALL"], "W1AW");
        assert_eq!(recs[0]["QSO_DATE"], "20260711");
    }

    #[test]
    fn parse_skips_header_and_comments() {
        let text = "Generated by X\n<ADIF_VER:5>3.1.6<PROGRAMID:1>X<EOH>\n\
                    <CALL:4>K5ZD<EOR>\n";
        let recs = parse_adif(text).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["CALL"], "K5ZD");
        // Header fields must not leak into the record.
        assert!(!recs[0].contains_key("ADIF_VER"));
        assert!(!recs[0].contains_key("PROGRAMID"));
    }

    #[test]
    fn parse_type_indicator_is_ignored() {
        let recs = parse_adif("<FREQ:9:N>14.074000<EOR>").unwrap();
        assert_eq!(recs[0]["FREQ"], "14.074000");
    }

    #[test]
    fn parse_value_with_embedded_angle_bracket() {
        // Length-prefix means '<' inside a value is data, not a tag.
        let recs = parse_adif("<COMMENT:5>a<b>c<EOR>").unwrap();
        assert_eq!(recs[0]["COMMENT"], "a<b>c");
    }

    #[test]
    fn parse_intl_utf8_byte_length() {
        // "José" is 5 bytes in UTF-8 (é = 2 bytes).
        let recs = parse_adif("<NAME_INTL:5>José<EOR>").unwrap();
        assert_eq!(recs[0]["NAME_INTL"], "José");
    }

    #[test]
    fn parse_truncated_value_errors() {
        let err = parse_adif("<CALL:10>W1AW<EOR>").unwrap_err();
        assert!(matches!(err, AdifError::TruncatedValue { .. }));
    }

    #[test]
    fn parse_field_missing_length_errors() {
        assert!(matches!(
            parse_adif("<CALL>W1AW<EOR>").unwrap_err(),
            AdifError::MissingLength(_)
        ));
    }

    #[test]
    fn parse_two_records() {
        let recs = parse_adif("<CALL:4>W1AW<EOR><CALL:4>K5ZD<EOR>").unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1]["CALL"], "K5ZD");
    }

    #[test]
    fn freq_roundtrip() {
        for hz in [
            1_840_000u64,
            14_074_000,
            21_074_000,
            50_313_000,
            144_174_000,
        ] {
            assert_eq!(mhz_string_to_hz(&hz_to_mhz_string(hz)), Some(hz));
        }
    }

    fn sample_qso() -> Qso {
        let mut extra = BTreeMap::new();
        extra.insert("APP_RIGFLOW_FOO".into(), "bar".into());
        extra.insert("NAME_INTL".into(), "José".into());
        extra.insert("MY_GRIDSQUARE".into(), "EM12".into());
        Qso {
            call: "W1AW".into(),
            qso_date: "20260711".into(),
            time_on: "142300".into(),
            band: "20m".into(),
            mode: "SSB".into(),
            submode: None,
            freq_hz: Some(14_207_000),
            freq_rx_hz: Some(14_200_000),
            band_rx: Some("20m".into()),
            rst_sent: Some("59".into()),
            rst_rcvd: Some("57".into()),
            gridsquare: Some("FN31".into()),
            dxcc: Some(291),
            extra,
        }
    }

    #[test]
    fn write_parse_write_byte_identical() {
        let q = sample_qso();
        let first = write_record(&qso_to_record(&q));
        let reparsed = record_to_qso(&parse_adif(&first).unwrap()[0]);
        let second = write_record(&qso_to_record(&reparsed));
        assert_eq!(first, second, "round-trip must be byte-identical");
    }

    #[test]
    fn roundtrip_preserves_app_intl_my_fields() {
        let q = sample_qso();
        let reparsed = record_to_qso(&parse_adif(&write_record(&qso_to_record(&q))).unwrap()[0]);
        assert_eq!(
            reparsed.extra.get("APP_RIGFLOW_FOO"),
            Some(&"bar".to_string())
        );
        assert_eq!(reparsed.extra.get("NAME_INTL"), Some(&"José".to_string()));
        assert_eq!(
            reparsed.extra.get("MY_GRIDSQUARE"),
            Some(&"EM12".to_string())
        );
        // Whole Qso round-trips (no column lost, none invented).
        assert_eq!(reparsed, q);
    }

    #[test]
    fn simplex_omits_rx_fields() {
        let mut q = sample_qso();
        q.freq_rx_hz = None;
        q.band_rx = None;
        let rec = qso_to_record(&q);
        assert!(!rec.contains_key("FREQ_RX"));
        assert!(!rec.contains_key("BAND_RX"));
    }
}
