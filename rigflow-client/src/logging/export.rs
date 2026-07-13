//! Client-side contact filtering and ADIF export: the drafts and the worker.
//!
//! **One filter, two consumers.** [`QsoFilterDraft`] builds the `ExportFilter`
//! that drives *both* the contact-view list and the export file, so what the
//! operator sees listed is what they get written. The filter lives with the
//! contact view (the `Filter…` window); the export window only chooses the
//! *output* shape.
//!
//! **Incremental is not a filter.** [`ExportDraft::incremental`] answers "what
//! haven't I exported yet", which is a fact about export progress, not about a
//! contact — so it lives in the export window and is ANDed onto the shared
//! filter at export time only. It would make no sense as a view filter (the list
//! would start hiding contacts for reasons unrelated to the contacts). The export
//! window shows the arithmetic, so the operator still knows what they're getting.
//!
//! All database work runs on the worker thread ([`spawn_export_worker`]) against
//! a read-only connection. The filters read the `extra` JSON blob, which is
//! unindexed — a full scan on every keystroke would stutter the UI thread that
//! owns the `LogStore`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rigflow_log::dedupe::DEFAULT_WINDOW_SECS;
use rigflow_log::display;
use rigflow_log::export::{
    ContactPage, ExportFilter, ExportOptions, ExportSummary, Exporter, FieldProfile, FilterError,
    GridPrecision, QslStatusFilter, Sort,
};
use rigflow_log::import::ImportPlan;
use rigflow_log::normalize::{self, ModeClass};

/// How long the UI waits after the last edit before re-querying.
pub const QUERY_DEBOUNCE: Duration = Duration::from_millis(250);

/// Rows the contact view holds. The view reports the *total* match count
/// separately, so this cap is visible ("showing 500 of 1,483") rather than a
/// silent truncation that would quietly break the see-what-you-export promise.
pub const VIEW_ROW_LIMIT: usize = 500;

/// Which set of fields an exported record carries — the draft's flattened form
/// of [`FieldProfile`] (egui radio buttons want a `Copy` discriminant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileChoice {
    #[default]
    Full,
    Core,
    Custom,
}

/// The shared contact filter: what the contact view lists, and what an export
/// writes. Edited in the `Filter…` window, owned by the contact view.
///
/// Deliberately does **not** carry `since_last_export` — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QsoFilterDraft {
    pub date_from: String,
    pub date_to: String,
    /// Selected ADIF bands (checkbox grid over `normalize::known_bands`).
    pub bands: BTreeSet<String>,
    /// Also match `band_rx` — the rare split-across-bands QSO.
    pub match_either_band: bool,
    pub mode_classes: BTreeSet<ModeClass>,
    /// Comma-separated exact canonical modes (`SSB, CW, FT8`).
    pub modes: String,
    /// Comma-separated submodes (`FT4, PSK31`).
    pub submodes: String,
    pub call_pattern: String,
    /// Comma-separated DXCC entity codes.
    pub dxcc: String,
    pub gridsquare: String,
    pub grid_precision: GridPrecision,
    pub my_gridsquare: String,
    pub contest_id: String,
    /// Service name for "not yet uploaded to …" (e.g. `lotw`). Empty = off.
    pub not_uploaded_to: String,
    /// Service name for "confirmed by …". Empty = off.
    pub confirmed_by: String,
    /// Only contacts whose QSL has come back (`QSL_RCVD` = Y).
    pub qsl_rcvd_yes: bool,
}

impl QsoFilterDraft {
    /// Whether any constraint is set. Drives the "filters are active" summary —
    /// a filtered list that doesn't *say* it's filtered is a footgun.
    pub fn is_active(&self) -> bool {
        *self != QsoFilterDraft::default()
    }

    /// Short human summary of the active constraints, for the chip under the
    /// contact-view toolbar (e.g. `20m, 40m · SSB · from 20260701`).
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.bands.is_empty() {
            let mut b = self.bands.iter().cloned().collect::<Vec<_>>();
            b.sort();
            parts.push(b.join(", "));
        }
        if !self.mode_classes.is_empty() {
            parts.push(
                self.mode_classes
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        for (label, v) in [
            ("", &self.modes),
            ("submode ", &self.submodes),
            ("call ", &self.call_pattern),
            ("dxcc ", &self.dxcc),
            ("grid ", &self.gridsquare),
            ("my grid ", &self.my_gridsquare),
            ("contest ", &self.contest_id),
        ] {
            let v = v.trim();
            if !v.is_empty() {
                parts.push(format!("{label}{v}"));
            }
        }
        if !self.date_from.trim().is_empty() {
            parts.push(format!(
                "from {}",
                display::date(&display::date_to_adif(&self.date_from))
            ));
        }
        if !self.date_to.trim().is_empty() {
            parts.push(format!(
                "to {}",
                display::date(&display::date_to_adif(&self.date_to))
            ));
        }
        if !self.not_uploaded_to.trim().is_empty() {
            parts.push(format!("not on {}", self.not_uploaded_to.trim()));
        }
        if !self.confirmed_by.trim().is_empty() {
            parts.push(format!("confirmed by {}", self.confirmed_by.trim()));
        }
        if self.qsl_rcvd_yes {
            parts.push("QSL received".into());
        }
        parts.join(" · ")
    }

    /// Build the shared [`ExportFilter`]. `incremental` (from the export window)
    /// is ANDed on here, and only there — the view always passes `None`.
    pub fn to_filter(&self, incremental: Option<String>) -> Result<ExportFilter, FilterError> {
        let f = ExportFilter {
            // The UI *shows* dates as `2026-07-12`, so an operator will type that
            // here. Accept it (and `2026/07/12`) and hand the store the
            // ADIF-native form; anything else falls through to `validate()` for a
            // proper error rather than being silently mangled.
            date_from: opt(&self.date_from).map(|s| display::date_to_adif(&s)),
            date_to: opt(&self.date_to).map(|s| display::date_to_adif(&s)),
            datetime_from: None,
            datetime_to: None,
            since_last_export: incremental,

            bands: (!self.bands.is_empty()).then(|| self.bands.iter().cloned().collect()),
            match_either_band: self.match_either_band,
            freq_from: None,
            freq_to: None,
            modes: csv(&self.modes),
            submodes: csv(&self.submodes),
            mode_classes: (!self.mode_classes.is_empty())
                .then(|| self.mode_classes.iter().copied().collect()),

            call_exact: None,
            call_pattern: opt(&self.call_pattern),
            call_prefix: None,
            dxcc: csv(&self.dxcc).map(|v| v.iter().filter_map(|s| s.parse().ok()).collect()),
            continent: None,
            cq_zone: None,
            itu_zone: None,
            gridsquare: opt(&self.gridsquare),
            grid_precision: self.grid_precision,
            state: None,
            country: None,

            station_ids: None,
            my_gridsquare: opt(&self.my_gridsquare),
            operator: None,
            station_callsign: None,

            uploaded_to: None,
            not_uploaded_to: opt(&self.not_uploaded_to).map(|s| vec![s]),
            confirmed_by: opt(&self.confirmed_by).map(|s| vec![s]),
            not_confirmed_by: None,
            qsl_sent: None,
            qsl_rcvd: self.qsl_rcvd_yes.then(|| QslStatusFilter {
                service: None,
                statuses: vec!["Y".into()],
            }),

            contest_ids: opt(&self.contest_id).map(|s| vec![s]),

            // Multi-select in the contact view is deferred; the filter API
            // supports an explicit id list already, there is just no UI to build
            // one yet.
            qso_ids: None,
        };
        f.validate()?;
        Ok(f)
    }

    /// Every band the checkbox grid offers.
    pub fn all_bands() -> Vec<&'static str> {
        normalize::known_bands().collect()
    }
}

/// A "have I worked this station?" lookup: matches the whole log by callsign,
/// **ignoring the view filter** — the question is about the log, not about what
/// the operator happens to be looking at. A 20m-filtered view must not hide the
/// 40m QSO with the station being checked.
pub fn call_lookup_filter(call: &str) -> Result<ExportFilter, FilterError> {
    let f = ExportFilter {
        call_exact: opt(call),
        ..Default::default()
    };
    f.validate()?;
    Ok(f)
}

/// The export window's state: **output shape only**, plus incremental.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportDraft {
    pub output_path: String,
    pub profile_choice: ProfileChoice,
    /// Comma-separated ADIF fields for [`ProfileChoice::Custom`].
    pub custom_fields: String,
    pub include_extra: bool,
    pub sort_reverse: bool,

    /// Export only what's new since this stream's bookmark. Not a view filter.
    pub incremental: bool,
    /// Bookmark stream name (blank → the default stream).
    pub profile: String,
}

impl ExportDraft {
    pub fn new(default_path: PathBuf) -> ExportDraft {
        ExportDraft {
            output_path: default_path.to_string_lossy().into_owned(),
            include_extra: true,
            ..Default::default()
        }
    }

    /// The bookmark stream this draft names (blank → the default).
    pub fn profile_name(&self) -> String {
        let p = self.profile.trim();
        if p.is_empty() {
            rigflow_log::export::DEFAULT_EXPORT_PROFILE.to_string()
        } else {
            p.to_string()
        }
    }

    /// `Some(stream)` when this export is incremental — what gets ANDed onto the
    /// shared filter at export time.
    pub fn incremental_profile(&self) -> Option<String> {
        self.incremental.then(|| self.profile_name())
    }

    pub fn to_options(&self) -> Result<ExportOptions, FilterError> {
        let field_profile = match self.profile_choice {
            ProfileChoice::Full => FieldProfile::Full,
            ProfileChoice::Core => FieldProfile::Core,
            ProfileChoice::Custom => {
                FieldProfile::Custom(csv(&self.custom_fields).unwrap_or_default())
            }
        };
        let opts = ExportOptions {
            output_path: PathBuf::from(self.output_path.trim()),
            field_profile,
            include_extra: self.include_extra,
            adif_version: rigflow_log::export::DEFAULT_ADIF_VERSION.to_string(),
            sort: if self.sort_reverse {
                Sort::Reverse
            } else {
                Sort::Chronological
            },
        };
        opts.validate()?;
        Ok(opts)
    }
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Split a comma-separated field into a list, or `None` if blank. An empty list
/// would be rejected by `validate()`, so blank must mean "no constraint".
fn csv(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    (!v.is_empty()).then_some(v)
}

// ─────────────────────────────────────────────────────────── worker ──────

/// Work handed to the export thread. Every variant carries the database path, so
/// the worker holds no per-operator state and an operator switch needs no
/// teardown.
pub enum ExportJob {
    /// The contact view's page: rows + total match count.
    Query {
        db_path: PathBuf,
        filter: Box<ExportFilter>,
        /// Echoed back so a reply for a filter the operator has already edited
        /// past is dropped instead of flashing stale rows on screen.
        seq: u64,
    },
    /// "Have I worked this station?" — the whole log, by callsign.
    CallLookup {
        db_path: PathBuf,
        call: String,
        filter: Box<ExportFilter>,
        seq: u64,
    },
    /// How many contacts an *incremental* export would write. Only needed when
    /// the export's set differs from the visible list (i.e. incremental is on);
    /// otherwise the view's own total already answers it.
    Count {
        db_path: PathBuf,
        filter: Box<ExportFilter>,
        seq: u64,
    },
    /// Write the file.
    Write {
        db_path: PathBuf,
        filter: Box<ExportFilter>,
        options: Box<ExportOptions>,
    },
    /// Read an ADIF file and work out what importing it *would* do — parse,
    /// normalize, validate, dedupe. Read-only, so it runs here rather than on the
    /// UI thread; a big file is slow to parse and the operator keeps logging.
    PlanImport { db_path: PathBuf, file: PathBuf },
}

/// What the worker reports back.
pub enum ExportEvent {
    Contacts {
        seq: u64,
        result: Result<Box<ContactPage>, String>,
    },
    CallMatches {
        seq: u64,
        call: String,
        result: Result<Box<ContactPage>, String>,
    },
    Count {
        seq: u64,
        result: Result<usize, String>,
    },
    Done(Result<Box<ExportSummary>, String>),
    /// The import preview. The plan carries the parsed contacts, so committing it
    /// needs no second parse — the operator confirms exactly what was planned.
    ImportPlanned {
        file: PathBuf,
        result: Result<Box<ImportPlan>, String>,
    },
}

/// Spawn the worker. One thread for the app's lifetime.
///
/// Queries are **coalesced**: while the operator types, several jobs can queue up
/// and only the newest of each kind is worth running — an older one's answer is
/// already stale, and running it only delays the one that matters. Writes are
/// never dropped.
pub fn spawn_export_worker() -> (Sender<ExportJob>, Receiver<ExportEvent>) {
    let (job_tx, job_rx) = channel::<ExportJob>();
    let (evt_tx, evt_rx) = channel::<ExportEvent>();

    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            while let Ok(job) = job_rx.recv() {
                let mut latest_query: Option<ExportJob> = None;
                let mut latest_lookup: Option<ExportJob> = None;
                let mut latest_count: Option<ExportJob> = None;
                let mut jobs = vec![job];
                jobs.extend(job_rx.try_iter());

                for job in jobs {
                    match job {
                        q @ ExportJob::Query { .. } => latest_query = Some(q),
                        l @ ExportJob::CallLookup { .. } => latest_lookup = Some(l),
                        c @ ExportJob::Count { .. } => latest_count = Some(c),
                        // Never coalesced: each is an explicit operator action.
                        w @ (ExportJob::Write { .. } | ExportJob::PlanImport { .. }) => {
                            if evt_tx.send(run(w)).is_err() {
                                return; // app gone
                            }
                        }
                    }
                }
                for job in [latest_query, latest_lookup, latest_count]
                    .into_iter()
                    .flatten()
                {
                    if evt_tx.send(run(job)).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawn export worker");

    (job_tx, evt_rx)
}

fn run(job: ExportJob) -> ExportEvent {
    match job {
        ExportJob::Query {
            db_path,
            filter,
            seq,
        } => ExportEvent::Contacts {
            seq,
            result: Exporter::open(&db_path)
                .and_then(|ex| ex.page(&filter, VIEW_ROW_LIMIT, Sort::Reverse))
                .map(Box::new)
                .map_err(|e| e.to_string()),
        },
        ExportJob::CallLookup {
            db_path,
            call,
            filter,
            seq,
        } => ExportEvent::CallMatches {
            seq,
            call,
            result: Exporter::open(&db_path)
                .and_then(|ex| ex.page(&filter, VIEW_ROW_LIMIT, Sort::Reverse))
                .map(Box::new)
                .map_err(|e| e.to_string()),
        },
        ExportJob::Count {
            db_path,
            filter,
            seq,
        } => ExportEvent::Count {
            seq,
            result: Exporter::open(&db_path)
                .and_then(|ex| ex.count(&filter))
                .map_err(|e| e.to_string()),
        },
        ExportJob::Write {
            db_path,
            filter,
            options,
        } => ExportEvent::Done(
            Exporter::open(&db_path)
                .and_then(|ex| ex.export(&filter, &options))
                .map(Box::new)
                .map_err(|e| e.to_string()),
        ),
        ExportJob::PlanImport { db_path, file } => {
            let result = std::fs::read_to_string(&file)
                .map_err(|e| format!("{}: {e}", file.display()))
                .and_then(|text| {
                    Exporter::open(&db_path)
                        .and_then(|ex| ex.plan_import(&text, DEFAULT_WINDOW_SECS))
                        .map_err(|e| e.to_string())
                })
                .map(Box::new);
            ExportEvent::ImportPlanned { file, result }
        }
    }
}
