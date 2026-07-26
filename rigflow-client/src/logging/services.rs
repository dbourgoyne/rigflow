//! The online QSL services and the sync operations over them.
//!
//! One place that knows, per service: its storage key, display name, how it
//! authenticates (username+password vs an API key), and which directions it
//! supports. The per-service HTTP lives in [`super::lotw`], [`super::eqsl`],
//! [`super::qrz`]; this module dispatches to them so the worker and UI stay
//! service-agnostic.
//!
//! **Verification note:** LoTW download is the only path exercised against a live
//! service so far. The eQSL and QRZ request shapes follow their documented APIs
//! but need a run with real accounts — the exact form fields and response
//! wording are quirky and centralized here for easy adjustment.

use super::{eqsl, lotw, qrz};

/// An online QSL service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Lotw,
    Eqsl,
    Qrz,
}

/// A sync direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Pull confirmations (and QSOs) down, into the import pipeline.
    Download,
    /// Push not-yet-uploaded QSOs up.
    Upload,
}

/// How a service authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredKind {
    /// A username and password (LoTW, eQSL).
    UserPass,
    /// A single API key (QRZ Logbook). Stored in the password slot; login empty.
    ApiKey,
}

/// The result of an upload, parsed from the service's reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadReport {
    /// Records the service accepted (added, or already-present duplicates).
    pub accepted: usize,
    /// Records the service rejected outright.
    pub rejected: usize,
    /// A short human line from the service, for the status area.
    pub message: String,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::Lotw, Service::Eqsl, Service::Qrz];

    /// Storage key — the `qso_service.service` / `sync_state.service` value and
    /// the credential-store key. Lower-case and stable.
    pub fn key(self) -> &'static str {
        match self {
            Service::Lotw => "lotw",
            Service::Eqsl => "eqsl",
            Service::Qrz => "qrz",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Service::Lotw => "LoTW",
            Service::Eqsl => "eQSL",
            Service::Qrz => "QRZ Logbook",
        }
    }

    pub fn cred_kind(self) -> CredKind {
        match self {
            Service::Qrz => CredKind::ApiKey,
            _ => CredKind::UserPass,
        }
    }

    /// LoTW upload needs a TQSL-signed file + certificate, out of scope here.
    pub fn can_upload(self) -> bool {
        !matches!(self, Service::Lotw)
    }

    pub fn can_download(self) -> bool {
        true
    }
}

/// Download a service's ADIF (confirmations / QSOs) and the next incremental
/// marker, if the service provides one. `since` is the stored marker.
///
/// For QRZ, `password` carries the API key and `login` is unused.
pub fn download(
    service: Service,
    login: &str,
    password: &str,
    since: Option<&str>,
) -> Result<(String, Option<String>), String> {
    match service {
        Service::Lotw => {
            let adif = lotw::fetch_report(login, password, since)?;
            let marker = lotw::extract_lastqsl(&adif);
            Ok((adif, marker))
        }
        // eQSL/QRZ have no clean "since confirmation" cursor, so they pull the
        // full set and lean on the import being idempotent (already-confirmed
        // records skip). No marker.
        Service::Eqsl => Ok((eqsl::download(login, password)?, None)),
        Service::Qrz => Ok((qrz::fetch(password)?, None)),
    }
}

/// Upload QSOs to a service. `records` is `(qso_id, single-record ADIF)`; each
/// service assembles them as its API wants (eQSL one batch, QRZ one call per
/// record). Returns the ids that actually landed — so only those get stamped
/// `uploaded_at` — plus a tally. `password` carries the API key for QRZ.
pub fn upload(
    service: Service,
    login: &str,
    password: &str,
    records: &[(i64, String)],
) -> Result<(Vec<i64>, UploadReport), String> {
    match service {
        Service::Lotw => {
            Err("LoTW upload needs TQSL and a certificate — not supported here.".into())
        }
        Service::Eqsl => eqsl::upload(login, password, records),
        Service::Qrz => qrz::insert(password, records),
    }
}
