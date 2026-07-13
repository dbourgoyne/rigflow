//! Client-side ADIF export: the dialog's draft state and the worker thread.
//!
//! Export runs **off the UI thread**. `rigflow_log::Exporter` opens its own
//! read-only SQLite connection, so a 200k-QSO export streams to disk on a worker
//! while the app's read-write `LogStore` keeps logging contacts on the UI thread
//! — WAL gives concurrent readers for free. The worker never writes, so it also
//! cannot advance an incremental bookmark: it reports `max_qso_id` back and the
//! UI thread advances the bookmark on the store it owns (and only for an
//! incremental, non-dry-run export). See `rigflow_log::export`.
//!
//! The live match count in the dialog is a *dry run* through the same filter, so
//! the number the operator sees is the number they get. Those counts are
//! debounced (see [`COUNT_DEBOUNCE`]) because the `extra`-JSON-backed filters
//! (continent, zones, contest id, the `MY_*` snapshot) are unindexed full scans.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rigflow_log::export::{
    ExportFilter, ExportOptions, ExportSummary, Exporter, FieldProfile, FilterError, GridPrecision,
    QslStatusFilter, Sort,
};
use rigflow_log::normalize::{self, ModeClass};

/// How long the dialog waits after the last edit before running a count.
pub const COUNT_DEBOUNCE: Duration = Duration::from_millis(250);

/// Which set of fields a record carries — the draft's flattened form of
/// [`FieldProfile`] (egui radio buttons want a `Copy` discriminant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileChoice {
    #[default]
    Full,
    Core,
    Custom,
}

/// The export dialog's editable state.
///
/// This lives on `RigflowApp`, **not** in `UiState`: only the export window
/// touches it, and `UiState` is cloned every frame by `snapshot_state()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportDraft {
    // ── filters ──────────────────────────────────────────────────────────
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
    /// Only QSOs I haven't yet received a QSL for (`QSL_RCVD` != Y).
    pub qsl_rcvd_yes: bool,

    // ── incremental ──────────────────────────────────────────────────────
    /// Export only what's new since this profile's bookmark.
    pub incremental: bool,
    /// Bookmark profile name (blank → the default profile).
    pub profile: String,

    // ── output ───────────────────────────────────────────────────────────
    pub output_path: String,
    pub profile_choice: ProfileChoice,
    /// Comma-separated ADIF fields for [`ProfileChoice::Custom`].
    pub custom_fields: String,
    pub include_extra: bool,
    pub sort_reverse: bool,
}

impl ExportDraft {
    /// A fresh draft, pre-filled with a sensible destination.
    pub fn new(default_path: PathBuf) -> ExportDraft {
        ExportDraft {
            output_path: default_path.to_string_lossy().into_owned(),
            include_extra: true,
            ..Default::default()
        }
    }

    /// The bookmark profile this draft names (blank → the default).
    pub fn profile_name(&self) -> String {
        let p = self.profile.trim();
        if p.is_empty() {
            rigflow_log::export::DEFAULT_EXPORT_PROFILE.to_string()
        } else {
            p.to_string()
        }
    }

    /// Translate the draft into an [`ExportFilter`].
    ///
    /// Returns the *validated* filter, so a malformed one is reported in the
    /// dialog (as a red line under the count) instead of running a query that
    /// silently matches nothing.
    pub fn to_filter(&self) -> Result<ExportFilter, FilterError> {
        let f = ExportFilter {
            date_from: opt(&self.date_from),
            date_to: opt(&self.date_to),
            datetime_from: None,
            datetime_to: None,
            since_last_export: self.incremental.then(|| self.profile_name()),

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

    /// Translate the draft into [`ExportOptions`].
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

    /// Every band the checkbox grid offers.
    pub fn all_bands() -> Vec<&'static str> {
        normalize::known_bands().collect()
    }
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Split a comma-separated field into a list, or `None` if it's blank. An empty
/// list would be rejected by `validate()`, so blank must mean "no constraint".
fn csv(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    (!v.is_empty()).then_some(v)
}

// ─────────────────────────────────────────────────────────── worker ──────

/// Work handed to the export thread. Both variants carry the database path, so
/// the worker holds no per-operator state and an operator switch needs no
/// teardown.
pub enum ExportJob {
    /// Dry run: count the matches, write nothing.
    Count {
        db_path: PathBuf,
        filter: Box<ExportFilter>,
    },
    /// Write the file.
    Write {
        db_path: PathBuf,
        filter: Box<ExportFilter>,
        options: Box<ExportOptions>,
    },
}

/// What the worker reports back.
pub enum ExportEvent {
    Count(Result<usize, String>),
    Done(Result<Box<ExportSummary>, String>),
}

/// Spawn the export worker. One thread for the app's lifetime.
///
/// Counts are **coalesced**: while the operator types, several count jobs can
/// queue up, and only the newest is worth running. Writes are never dropped.
pub fn spawn_export_worker() -> (Sender<ExportJob>, Receiver<ExportEvent>) {
    let (job_tx, job_rx) = channel::<ExportJob>();
    let (evt_tx, evt_rx) = channel::<ExportEvent>();

    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            while let Ok(job) = job_rx.recv() {
                // Drain whatever else is queued: run every Write, but only the
                // last Count — an older count's answer is already stale.
                let mut pending_count: Option<ExportJob> = None;
                let mut jobs = vec![job];
                jobs.extend(job_rx.try_iter());

                for job in jobs {
                    match job {
                        c @ ExportJob::Count { .. } => pending_count = Some(c),
                        w @ ExportJob::Write { .. } => {
                            if evt_tx.send(run(w)).is_err() {
                                return; // app gone
                            }
                        }
                    }
                }
                if let Some(c) = pending_count
                    && evt_tx.send(run(c)).is_err()
                {
                    return;
                }
            }
        })
        .expect("spawn export worker");

    (job_tx, evt_rx)
}

fn run(job: ExportJob) -> ExportEvent {
    match job {
        ExportJob::Count { db_path, filter } => ExportEvent::Count(
            Exporter::open(&db_path)
                .and_then(|ex| ex.count(&filter))
                .map_err(|e| e.to_string()),
        ),
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
    }
}
