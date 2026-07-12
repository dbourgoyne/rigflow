//! Client-side contact logging: the thin wiring around the `rigflow-log` crate.
//!
//! `capture` turns the live `UiState` into a `rigflow_log::CapturedRadioState`
//! (mirroring the server's effective-TX formula). `lifecycle` owns the
//! per-operator `LogStore` and the insert/query paths. The egui surfaces (entry
//! window, contact-view window, Station panel) live under `ui/`.

pub mod capture;
pub mod lifecycle;
pub mod wsjtx_listener;

/// Editable draft behind the manual log-entry window. The frozen-at-open
/// capture (date/time/freq/mode/split) sits alongside the operator-typed fields
/// so the logged time and frequency reflect when the entry opened, not when the
/// operator finally hits save. Not persisted.
#[derive(Debug, Clone, Default)]
pub struct LogEntryDraft {
    // Operator-entered fields.
    pub call: String,
    pub rst_sent: String,
    pub rst_rcvd: String,
    pub name: String,
    pub comment: String,
    pub gridsquare: String,
    /// ADIF mode — editable (a DgtU capture defaults to FT8 but the operator may
    /// be running JS8/etc.).
    pub mode: String,
    /// Derived `FREQ_RX` shown editable as a plain Hz string ("" = simplex).
    pub freq_rx_hz_str: String,

    // Frozen at open.
    pub qso_date: String,
    pub time_on: String,
    pub tx_freq_hz: u64,
    pub split_active: bool,
    pub derived_freq_rx_hz: Option<u64>,
}
