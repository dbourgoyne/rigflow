//! [`ExportFilter`] — the public API of export — plus output options and
//! up-front validation.
//!
//! Every filter field is `Option`: absent means "no constraint on this
//! dimension", never "match nothing". The egui dialog and any programmatic
//! caller both build one of these; [`crate::export::query`] is the only thing
//! that knows how they become SQL.
//!
//! **Combining semantics** (fixed, relied on by the tests):
//! - different filter *categories* combine with **AND**
//! - multiple values *within* one filter combine with **OR** (`bands = [20m,
//!   40m]` means 20m **or** 40m)
//! - `not_*` variants negate.

use std::path::PathBuf;

use crate::normalize::{self, ModeClass};

/// The ADIF `CONT` enumeration. Closed set, so a typo is a validation error
/// rather than an export that silently matches nothing.
pub const CONTINENTS: &[&str] = &["NA", "SA", "EU", "AF", "AS", "OC", "AN"];

/// How much of a gridsquare to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridPrecision {
    /// First 2 chars — the Maidenhead *field* (e.g. `EM`).
    Field,
    /// First 4 chars — the *square* (e.g. `EM12`).
    Square,
    /// The whole stored gridsquare, exact.
    #[default]
    Full,
}

impl GridPrecision {
    /// Number of leading characters to compare, or `None` for the whole value.
    pub fn chars(self) -> Option<usize> {
        match self {
            GridPrecision::Field => Some(2),
            GridPrecision::Square => Some(4),
            GridPrecision::Full => None,
        }
    }
}

/// A QSL status filter: the status letters to match, optionally scoped to one
/// service.
///
/// `service = None` matches the QSO-level `QSL_SENT` / `QSL_RCVD`;
/// `service = Some("lotw")` matches `LOTW_QSL_SENT` / `LOTW_QSL_RCVD`.
///
/// **These come from the `extra` JSON, not from `qso_service`** — that table has
/// no status column (it holds upload/confirm *timestamps*), and in ADIF the QSL
/// status fields are QSO-level anyway. This is a deliberate deviation, confirmed
/// with the user; see the crate-level export docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QslStatusFilter {
    /// `None` = the QSO-level field; `Some(svc)` = that service's variant.
    pub service: Option<String>,
    /// Status letters to match (ADIF: `Y`, `N`, `R`, `I`, `V`). ORed.
    pub statuses: Vec<String>,
}

/// A full-precision UTC instant, ADIF-native.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp {
    /// `YYYYMMDD`
    pub date: String,
    /// `HHMMSS` (a 4-digit `HHMM` is accepted and padded).
    pub time: String,
}

impl Timestamp {
    /// The 14-char `YYYYMMDDHHMMSS` key this compares as. Times are padded to 6
    /// digits so a 4-digit ADIF `TIME_ON` still orders correctly.
    pub fn key(&self) -> String {
        let mut t = self.time.trim().to_string();
        while t.len() < 6 {
            t.push('0');
        }
        format!("{}{}", self.date.trim(), &t[..6])
    }
}

/// Which ADIF fields an exported record carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FieldProfile {
    /// All modeled columns, plus the `extra` passthrough when
    /// [`ExportOptions::include_extra`] is set.
    #[default]
    Full,
    /// A minimal, self-sufficient set — see [`CORE_FIELDS`].
    Core,
    /// Exactly these ADIF fields (upper-cased), and nothing else.
    Custom(Vec<String>),
}

/// The `Core` profile's field set.
///
/// Note `SUBMODE` is included even though the brief's Core list omits it:
/// without it an FT4 QSO exports as bare `MODE=MFSK` and loses its identity on
/// re-import, and an FT8-vs-FT4 distinction is not something a "minimal but
/// correct" profile can afford to drop. `FREQ_RX`/`BAND_RX` are listed here but,
/// like every field, only emitted when actually present on the record — so a
/// simplex QSO still omits them.
pub const CORE_FIELDS: &[&str] = &[
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
];

/// Record ordering in the output file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Oldest first (`qso_date`, `time_on` ascending) — the ADIF convention.
    #[default]
    Chronological,
    /// Newest first.
    Reverse,
}

/// The ADIF spec version stamped into the header.
pub const DEFAULT_ADIF_VERSION: &str = "3.1.6";

/// The incremental bookmark profile used when the caller doesn't name one.
pub const DEFAULT_EXPORT_PROFILE: &str = "default";

/// Output shape — *not* filters. These never affect which QSOs match, only how
/// the matched ones are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// Destination file.
    pub output_path: PathBuf,
    pub field_profile: FieldProfile,
    /// Emit the `extra` passthrough (`APP_*`, `MY_*`, unmodeled ADIF fields).
    /// Only meaningful for [`FieldProfile::Full`] — `Core` and `Custom` are
    /// explicit whitelists and ignore it.
    pub include_extra: bool,
    pub adif_version: String,
    pub sort: Sort,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            output_path: PathBuf::new(),
            field_profile: FieldProfile::Full,
            include_extra: true,
            adif_version: DEFAULT_ADIF_VERSION.to_string(),
            sort: Sort::Chronological,
        }
    }
}

/// Which QSOs to export. All-optional; see the module docs for how the fields
/// combine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportFilter {
    // ---- A. date / time (all UTC) ----
    /// Inclusive `YYYYMMDD` lower bound on `qso_date`.
    pub date_from: Option<String>,
    /// Inclusive `YYYYMMDD` upper bound on `qso_date`.
    pub date_to: Option<String>,
    /// Inclusive full-precision lower bound (contest sessions).
    pub datetime_from: Option<Timestamp>,
    /// Inclusive full-precision upper bound.
    pub datetime_to: Option<Timestamp>,
    /// Export only QSOs newer than this named bookmark's position. Filters on
    /// **insertion order** (`qso.id`), not `qso_date` — it's about export
    /// progress, not contact time. `Some(profile_name)`; see
    /// [`DEFAULT_EXPORT_PROFILE`].
    pub since_last_export: Option<String>,

    // ---- B. frequency / band / mode ----
    /// ADIF `BAND` values. Matches the TX `band` column.
    pub bands: Option<Vec<String>>,
    /// Also match `band_rx` (the rare split-across-bands QSO). Only meaningful
    /// alongside `bands`.
    pub match_either_band: bool,
    /// Inclusive Hz lower bound on `freq_hz` (TX).
    pub freq_from: Option<u64>,
    /// Inclusive Hz upper bound on `freq_hz` (TX).
    pub freq_to: Option<u64>,
    /// Exact canonical `MODE` values (`SSB`, `CW`, `FT8`, `MFSK`, …).
    pub modes: Option<Vec<String>>,
    /// Exact `SUBMODE` values (`FT4`, `PSK31`, …).
    pub submodes: Option<Vec<String>>,
    /// Coarse mode families; expanded to concrete modes via
    /// [`normalize::modes_in_class`].
    pub mode_classes: Option<Vec<ModeClass>>,

    // ---- C. worked station / entity ----
    pub call_exact: Option<String>,
    /// User wildcard: `*` = any run, `?` = any one char. Passed as a bound
    /// parameter — never interpolated.
    pub call_pattern: Option<String>,
    /// Starts-with (a rough DXCC-prefix filter, e.g. `JA`).
    pub call_prefix: Option<String>,
    pub dxcc: Option<Vec<i64>>,
    /// ADIF `CONT` — see [`CONTINENTS`]. From `extra`.
    pub continent: Option<Vec<String>>,
    /// ADIF `CQZ`. From `extra`. Compared numerically, so a stored `"05"` and a
    /// filter of `5` match.
    pub cq_zone: Option<Vec<i64>>,
    /// ADIF `ITUZ`. From `extra`. Compared numerically.
    pub itu_zone: Option<Vec<i64>>,
    /// The worked station's grid, compared at `grid_precision`.
    pub gridsquare: Option<String>,
    pub grid_precision: GridPrecision,
    /// ADIF `STATE`. From `extra`.
    pub state: Option<Vec<String>>,
    /// ADIF `COUNTRY`. From `extra`.
    pub country: Option<Vec<String>>,

    // ---- D. my station ----
    /// `station.id` values. NB this identifies **which callsign** was used, not
    /// which location: the station row is keyed on callsign and updated in
    /// place, so it holds the *current* location, not the one in force at QSO
    /// time. Location filters below read the per-QSO snapshot instead.
    pub station_ids: Option<Vec<i64>>,
    /// `MY_GRIDSQUARE` **from the per-QSO snapshot in `extra`**, so a filter
    /// still finds the QSOs made from a grid you've since moved away from.
    pub my_gridsquare: Option<String>,
    /// `OPERATOR` from the snapshot.
    pub operator: Option<String>,
    /// `STATION_CALLSIGN` from the snapshot (may differ from `OPERATOR`).
    pub station_callsign: Option<String>,

    // ---- E. confirmation / service ----
    // These join `qso_service`, which is empty until a later phase populates it.
    // That is correct, not a bug: `not_uploaded_to` then matches everything and
    // `uploaded_to`/`confirmed_by` match nothing. They light up on their own.
    pub uploaded_to: Option<Vec<String>>,
    pub not_uploaded_to: Option<Vec<String>>,
    pub confirmed_by: Option<Vec<String>>,
    pub not_confirmed_by: Option<Vec<String>>,
    pub qsl_sent: Option<QslStatusFilter>,
    pub qsl_rcvd: Option<QslStatusFilter>,

    // ---- F. contest ----
    /// ADIF `CONTEST_ID`. From `extra`.
    pub contest_ids: Option<Vec<String>>,

    // ---- G. explicit selection ----
    /// Explicit `qso.id` list (the rows a user multi-selected).
    ///
    /// **This ANDs with the other filters** — it's a selection you may further
    /// narrow, not an override. The UI clears the other filters when exporting a
    /// selection, so there is exactly one authoritative behavior here.
    pub qso_ids: Option<Vec<i64>>,
}

/// Why a filter or option was rejected. Export validates up front and refuses,
/// rather than running a query that silently matches nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    BadDate(String),
    BadTime(String),
    /// A `from` bound is later than its `to` bound.
    InvertedRange {
        from: String,
        to: String,
    },
    UnknownBand(String),
    /// A mode that isn't canonical (e.g. `USB`, `FT4`) — with the canonical form
    /// to use instead, so the message is actionable.
    NonCanonicalMode {
        given: String,
        canonical: String,
    },
    UnknownContinent(String),
    /// A list-valued filter was present but empty — almost certainly a UI bug,
    /// and it would match nothing.
    EmptyList(&'static str),
    /// `FieldProfile::Custom` with no fields.
    EmptyFieldList,
    /// A service name with characters outside `[A-Za-z0-9_]`.
    ///
    /// Service names in [`QslStatusFilter::service`] are the one user value that
    /// reaches SQL as *text* rather than as a bound parameter — they name an
    /// ADIF field inside a `json_extract` path (`'$.LOTW_QSL_RCVD'`), and SQLite
    /// takes no parameter there. So they are whitelisted at the door instead.
    BadServiceName(String),
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::BadDate(d) => write!(f, "invalid date {d:?} (want YYYYMMDD)"),
            FilterError::BadTime(t) => write!(f, "invalid time {t:?} (want HHMM or HHMMSS)"),
            FilterError::InvertedRange { from, to } => {
                write!(f, "range start {from:?} is after its end {to:?}")
            }
            FilterError::UnknownBand(b) => write!(f, "unknown band {b:?}"),
            FilterError::NonCanonicalMode { given, canonical } => write!(
                f,
                "{given:?} is not a canonical ADIF MODE — use {canonical:?} \
                 (or filter it as a submode)"
            ),
            FilterError::UnknownContinent(c) => {
                write!(f, "unknown continent {c:?} (want one of {CONTINENTS:?})")
            }
            FilterError::EmptyList(which) => write!(f, "{which} filter is present but empty"),
            FilterError::EmptyFieldList => write!(f, "custom field profile has no fields"),
            FilterError::BadServiceName(s) => {
                write!(f, "invalid service name {s:?} (letters, digits, _ only)")
            }
        }
    }
}

/// Service names must be plain identifiers — see [`FilterError::BadServiceName`].
pub(crate) fn valid_service_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl std::error::Error for FilterError {}

fn valid_date(d: &str) -> bool {
    let d = d.trim();
    d.len() == 8
        && d.bytes().all(|b| b.is_ascii_digit())
        && chrono::NaiveDate::parse_from_str(d, "%Y%m%d").is_ok()
}

fn valid_time(t: &str) -> bool {
    let t = t.trim();
    (t.len() == 4 || t.len() == 6) && t.bytes().all(|b| b.is_ascii_digit())
}

fn check_nonempty<T>(name: &'static str, v: &Option<Vec<T>>) -> Result<(), FilterError> {
    match v {
        Some(list) if list.is_empty() => Err(FilterError::EmptyList(name)),
        _ => Ok(()),
    }
}

impl ExportFilter {
    /// Reject bad input up front. Called by every export/count entry point, so
    /// a malformed filter can never reach SQL.
    pub fn validate(&self) -> Result<(), FilterError> {
        for (name, d) in [("date_from", &self.date_from), ("date_to", &self.date_to)] {
            if let Some(d) = d
                && !valid_date(d)
            {
                let _ = name;
                return Err(FilterError::BadDate(d.clone()));
            }
        }
        if let (Some(from), Some(to)) = (&self.date_from, &self.date_to)
            && from.trim() > to.trim()
        {
            return Err(FilterError::InvertedRange {
                from: from.clone(),
                to: to.clone(),
            });
        }

        for ts in [&self.datetime_from, &self.datetime_to]
            .into_iter()
            .flatten()
        {
            if !valid_date(&ts.date) {
                return Err(FilterError::BadDate(ts.date.clone()));
            }
            if !valid_time(&ts.time) {
                return Err(FilterError::BadTime(ts.time.clone()));
            }
        }
        if let (Some(from), Some(to)) = (&self.datetime_from, &self.datetime_to)
            && from.key() > to.key()
        {
            return Err(FilterError::InvertedRange {
                from: from.key(),
                to: to.key(),
            });
        }

        if let (Some(from), Some(to)) = (self.freq_from, self.freq_to)
            && from > to
        {
            return Err(FilterError::InvertedRange {
                from: from.to_string(),
                to: to.to_string(),
            });
        }

        check_nonempty("bands", &self.bands)?;
        check_nonempty("modes", &self.modes)?;
        check_nonempty("submodes", &self.submodes)?;
        check_nonempty("mode_classes", &self.mode_classes)?;
        check_nonempty("dxcc", &self.dxcc)?;
        check_nonempty("continent", &self.continent)?;
        check_nonempty("cq_zone", &self.cq_zone)?;
        check_nonempty("itu_zone", &self.itu_zone)?;
        check_nonempty("state", &self.state)?;
        check_nonempty("country", &self.country)?;
        check_nonempty("station_ids", &self.station_ids)?;
        check_nonempty("contest_ids", &self.contest_ids)?;
        check_nonempty("qso_ids", &self.qso_ids)?;
        check_nonempty("uploaded_to", &self.uploaded_to)?;
        check_nonempty("not_uploaded_to", &self.not_uploaded_to)?;
        check_nonempty("confirmed_by", &self.confirmed_by)?;
        check_nonempty("not_confirmed_by", &self.not_confirmed_by)?;

        for b in self.bands.iter().flatten() {
            if !normalize::is_known_band(b) {
                return Err(FilterError::UnknownBand(b.clone()));
            }
        }

        // A mode filter compares against the stored canonical `mode` column, so
        // a non-canonical value (USB, FT4) would match zero rows however many
        // such QSOs are logged. Reject it and say what to use instead.
        for m in self.modes.iter().flatten() {
            let (canonical, submode) = normalize::normalize_mode(m, None);
            if canonical != m.trim().to_ascii_uppercase() || submode.is_some() {
                return Err(FilterError::NonCanonicalMode {
                    given: m.clone(),
                    canonical,
                });
            }
        }

        for c in self.continent.iter().flatten() {
            let c_uc = c.trim().to_ascii_uppercase();
            if !CONTINENTS.contains(&c_uc.as_str()) {
                return Err(FilterError::UnknownContinent(c.clone()));
            }
        }

        // Every service name, whether it ends up bound (uploaded_to &c.) or
        // interpolated into a JSON path (qsl_sent/qsl_rcvd). Checking all of
        // them keeps the rule one rule instead of two.
        let services = [
            &self.uploaded_to,
            &self.not_uploaded_to,
            &self.confirmed_by,
            &self.not_confirmed_by,
        ];
        for svc in services.into_iter().flatten().flatten() {
            if !valid_service_name(svc) {
                return Err(FilterError::BadServiceName(svc.clone()));
            }
        }
        for q in [&self.qsl_sent, &self.qsl_rcvd].into_iter().flatten() {
            if let Some(svc) = &q.service
                && !valid_service_name(svc)
            {
                return Err(FilterError::BadServiceName(svc.clone()));
            }
            if q.statuses.is_empty() {
                return Err(FilterError::EmptyList("qsl status"));
            }
        }

        Ok(())
    }

    /// The bookmark profile this filter reads, if it is an incremental export.
    pub fn incremental_profile(&self) -> Option<&str> {
        self.since_last_export.as_deref()
    }
}

impl ExportOptions {
    pub fn validate(&self) -> Result<(), FilterError> {
        if let FieldProfile::Custom(fields) = &self.field_profile
            && fields.is_empty()
        {
            return Err(FilterError::EmptyFieldList);
        }
        Ok(())
    }

    /// Project a record down to this profile's fields.
    ///
    /// This is a **projection over the shared writer's output**, not a second
    /// serializer: the record has already been built by
    /// [`crate::adif::qso_to_record`], so the split-frequency rule
    /// (`FREQ_RX`/`BAND_RX` only when present) and the modeled-column-wins rule
    /// are applied identically here and in the journal.
    pub fn project(&self, record: &mut crate::adif::AdifRecord) {
        match &self.field_profile {
            FieldProfile::Full => {
                if !self.include_extra {
                    record.retain(|k, _| crate::adif::COLUMN_FIELDS.contains(&k.as_str()));
                }
            }
            FieldProfile::Core => {
                record.retain(|k, _| CORE_FIELDS.contains(&k.as_str()));
            }
            FieldProfile::Custom(fields) => {
                let want: Vec<String> = fields
                    .iter()
                    .map(|f| f.trim().to_ascii_uppercase())
                    .collect();
                record.retain(|k, _| want.contains(k));
            }
        }
    }
}
