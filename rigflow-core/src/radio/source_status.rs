/// Generic read-only telemetry emitted by an IQ source.
///
/// All fields are `Option` so sources only populate what they support.
/// RTL-SDR and other sources that don't decode telemetry simply return
/// `SourceStatus::default()` (all `None`).
///
/// The client UI shows a "Source Status" pane only when at least one
/// field is `Some`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SourceStatus {
    /// Human-readable firmware version string.
    pub firmware_version: Option<String>,

    /// ADC overload flag: `true` = input is clipping.
    pub adc_overload: Option<bool>,

    /// Measured temperature in degrees Celsius.
    pub temperature_c: Option<f32>,

    /// DC supply current in amperes.
    pub current_a: Option<f32>,

    /// RF forward (transmitted) power in watts.
    pub forward_power_w: Option<f32>,

    /// RF reverse (reflected) power in watts.
    pub reverse_power_w: Option<f32>,

    /// Standing wave ratio computed from forward / reverse power.
    /// `None` when forward power is too low or not transmitting.
    pub swr: Option<f32>,

    /// Whether the transmitter is inhibited by hardware interlock.
    pub tx_inhibited: Option<bool>,

    /// ADC under/overflow recovery state.
    /// Human-readable string: "OK", "UNDERFLOW", "OVERFLOW", or "UNKNOWN".
    pub recovery_status: Option<String>,

    /// Whether the device is currently sending IQ.
    /// `Some(false)` = no data received recently ("not responding", e.g. a link
    /// blip or the device powered off); `Some(true)` = receiving normally;
    /// `None` = the source doesn't report this.
    pub device_responding: Option<bool>,
}

impl SourceStatus {
    /// Returns `true` if at least one field is populated.
    ///
    /// The UI uses this to decide whether to show the Source Status pane.
    pub fn has_any(&self) -> bool {
        self.firmware_version.is_some()
            || self.adc_overload.is_some()
            || self.temperature_c.is_some()
            || self.current_a.is_some()
            || self.forward_power_w.is_some()
            || self.reverse_power_w.is_some()
            || self.swr.is_some()
            || self.tx_inhibited.is_some()
            || self.recovery_status.is_some()
            || self.device_responding.is_some()
    }
}
