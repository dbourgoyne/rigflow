/// Client-local arming/availability state for the TX tune test feature.
///
/// This enum is a UI-only concept. It is never serialised into protocol
/// messages or persisted to disk. The server communicates TX support through
/// `SourceCapabilities::supports_tx_tune_test`; the client derives its local
/// state from that flag plus the user's arm checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxTuneState {
    /// Hardware does not advertise TX tune test support.
    Unavailable,

    /// Supported but not yet armed by the operator.
    Disarmed,

    /// Armed: operator has enabled the arm checkbox.
    Armed,
}

impl Default for TxTuneState {
    fn default() -> Self {
        Self::Disarmed
    }
}

/// Placeholder result from a TX tune test measurement.
///
/// All fields are `Option` because the result is only populated after a
/// successful tune-test exchange with the server. They remain `None` until
/// a real TX tune test is implemented in a future task.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TxTuneResult {
    /// Forward power measured during the pulse (Watts).
    pub forward_power_w: Option<f32>,

    /// Reverse power measured during the pulse (Watts).
    pub reverse_power_w: Option<f32>,

    /// Standing-wave ratio derived from forward/reverse power.
    pub swr: Option<f32>,

    /// Human-readable status message (e.g. "OK", "not implemented").
    pub message: Option<String>,
}
