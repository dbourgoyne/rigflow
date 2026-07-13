//! rigflow contact (QSO) logging — phase 1: local capture and storage.
//!
//! This crate is a **leaf**: it depends on no other rigflow crate and pulls in
//! neither egui nor ALSA, so it compiles and its tests run in environments that
//! cannot build `rigflow-client`. All pure/testable logging logic lives here —
//! the SQLite store, the ADIF parser/writer, mode/band normalization, dedupe,
//! the WSJT-X UDP decoder, and the split-frequency (`FREQ_RX`) derivation.
//!
//! The `rigflow-client` crate owns only the thin wiring: building
//! [`capture::CapturedRadioState`] from its live UI state, the egui windows and
//! panels, the hotkeys, and the UDP-2237 listener thread.

pub mod adif;
pub mod capture;
pub mod dedupe;
pub mod error;
pub mod export;
pub mod migrations;
pub mod model;
pub mod normalize;
pub mod schema;
pub mod store;
pub mod wsjtx;

pub use capture::{CapturedRadioState, Receiver};
pub use error::LogError;
pub use export::{ExportFilter, ExportOptions, ExportSummary, Exporter, FieldProfile};
pub use model::{Qso, Station};
pub use store::LogStore;

/// Current UTC as ADIF-native `(qso_date "YYYYMMDD", time_on "HHMMSS")`. Used to
/// freeze the log time at the instant a manual entry opens.
pub fn now_utc_adif() -> (String, String) {
    let now = chrono::Utc::now();
    (
        now.format("%Y%m%d").to_string(),
        now.format("%H%M%S").to_string(),
    )
}
