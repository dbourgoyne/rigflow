#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GainMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectSamplingMode {
    Off,
    I,
    Q,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceControlState {
    pub sample_rate_hz: u32,
    pub gain_mode: GainMode,
    pub gain_db: f32,
    pub ppm_correction: i32,
    pub direct_sampling: DirectSamplingMode,

    /// Transmit drive level in percent (0–100).  Operator transmit-power
    /// control; applies to all transmit operations (Spot/SWR now, voice/data
    /// later).  Persisted and synced like the other source-control fields.
    /// `#[serde(default)]` so older persisted state without it loads cleanly.
    #[serde(default = "default_tx_drive_percent")]
    pub tx_drive_percent: f32,

    /// N2ADR HF filter board enabled (HL2).  When set, the server programs the
    /// correct band filter from the tuned frequency, suppressing TX harmonics.
    /// **Defaults on** (safe by default; a no-op without the board).
    /// Persisted/synced like the other source-control fields.
    #[serde(default = "default_n2adr_enabled")]
    pub n2adr_enabled: bool,

    /// FDX / TX Monitor Spectrum (HL2).  When set, RX IQ captured during a
    /// Spot/SWR (or SWR sweep) transmit is forwarded into the receive DSP
    /// pipeline so the spectrum and waterfall stay live (and the transmit
    /// carrier becomes visible) instead of freezing.  Visual monitoring only —
    /// it does not change audio.  Persisted/synced like the other fields.
    #[serde(default)]
    pub fdx_enabled: bool,

    /// Spot Level in percent (0–100): the digital carrier IQ amplitude used for
    /// Spot / SWR / SWR-sweep transmits (`amplitude_fs = spot_level_percent /
    /// 100`).  Matches Quisk's Spot-slider behaviour.  Affects ONLY Spot/SWR —
    /// not voice/CW/digital TX.  RF power ≈ TX Drive × Spot Level.
    /// Persisted/synced like the other source-control fields.
    #[serde(default = "default_spot_level_percent")]
    pub spot_level_percent: f32,

    /// TX PTT sequencing lead delay in ms (0–100): PTT is asserted, then this
    /// delay elapses (relays settle) BEFORE any RF is emitted.  Shared by all
    /// HL2 transmit paths (Spot/SWR/sweep/test-tone, future CW).  Persisted.
    #[serde(default = "default_tx_ptt_lead_ms")]
    pub tx_ptt_lead_ms: u32,

    /// TX PTT sequencing tail delay in ms (0–100): after RF stops, PTT is held
    /// for this delay BEFORE release (prevents hot-switching relays).  Persisted.
    #[serde(default = "default_tx_ptt_tail_ms")]
    pub tx_ptt_tail_ms: u32,
}

pub fn default_tx_drive_percent() -> f32 {
    10.0
}

/// Quisk's default Spot level is 500 / 1000 = 50%.
pub fn default_spot_level_percent() -> f32 {
    50.0
}

/// Default PTT lead delay (ms) — enough for typical relay actuation.
pub fn default_tx_ptt_lead_ms() -> u32 {
    20
}

/// The N2ADR HF filter board defaults **on** as a safe-by-default choice: with the
/// board installed it auto-engages the band-pass filter (suppressing TX harmonics),
/// and without the board the J16 control bits drive nothing (a harmless no-op).
pub fn default_n2adr_enabled() -> bool {
    true
}

/// Default PTT tail delay (ms).
pub fn default_tx_ptt_tail_ms() -> u32 {
    20
}

impl Default for SourceControlState {
    fn default() -> Self {
        Self {
            sample_rate_hz: 2_048_000,
            gain_mode: GainMode::Auto,
            gain_db: 0.0,
            ppm_correction: 0,
            direct_sampling: DirectSamplingMode::Off,
            tx_drive_percent: default_tx_drive_percent(),
            n2adr_enabled: default_n2adr_enabled(),
            fdx_enabled: false,
            spot_level_percent: default_spot_level_percent(),
            tx_ptt_lead_ms: default_tx_ptt_lead_ms(),
            tx_ptt_tail_ms: default_tx_ptt_tail_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceCapabilities {
    pub supports_sample_rate: bool,
    pub sample_rates_hz: Vec<u32>,

    pub supports_gain_mode: bool,
    pub supports_gain: bool,
    pub gain_values_db: Vec<f32>,

    pub supports_ppm_correction: bool,
    pub ppm_min: i32,
    pub ppm_max: i32,

    pub supports_direct_sampling: bool,
    pub direct_sampling_modes: Vec<DirectSamplingMode>,
    pub direct_sampling_freq_hz_max: u32,

    pub tuner_freq_hz_min: u32,
    pub tuner_freq_hz_max: u32,

    /// Whether the source can transmit at all.  This is the master gate for ALL
    /// transmit UI (Radio Control → Transmit, the Dual-VFO split/TX controls, TX
    /// latency, two-tone / TX-audio diagnostics, mic/CW keying).  Receive-only
    /// sources (RTL-SDR, WAV playback, fake tone) advertise `false`.
    #[serde(default)]
    pub supports_transmit: bool,

    /// Whether the source has a second hardware receiver, so dual-watch (VFO B
    /// received in stereo with its own spectrum) is available.  Static per source
    /// (HL2 = true); gates the dual-watch UI.
    #[serde(default)]
    pub supports_dual_watch: bool,

    /// Whether the source supports a TX tune test (short low-power carrier
    /// pulse used to measure forward/reverse power and SWR).
    ///
    /// This is a capability flag only. The actual TX tune test protocol
    /// is not yet implemented; the UI skeleton is always disabled while
    /// this is `false`.
    pub supports_tx_tune_test: bool,

    /// Whether the source supports amateur Band Control + N2ADR filter board
    /// (HL2).  Gates the Band/N2ADR section in Source Control.
    #[serde(default)]
    pub supports_band_control: bool,

    /// Whether the source supports FDX / TX Monitor Spectrum (keeping RX
    /// spectrum/waterfall alive during Spot/SWR).  Gates the FDX section in
    /// Source Control.
    #[serde(default)]
    pub supports_fdx: bool,
}

impl SourceCapabilities {
    /// True if the source exposes any adjustable parameter that the Source
    /// Control "Configuration" section would draw (sample rate, gain/gain-mode,
    /// PPM, direct sampling, band control, or transmit controls).  Used to hide
    /// the section entirely for fixed sources that expose nothing — e.g. WAV
    /// playback and the fake-tone generator.  Keep in sync with
    /// `source_control::draw_configuration_section`.
    pub fn has_configuration_controls(&self) -> bool {
        self.supports_sample_rate
            || self.supports_gain_mode
            || self.supports_gain
            || self.supports_ppm_correction
            || self.supports_direct_sampling
            || self.supports_band_control
            || self.supports_transmit
    }

    pub fn none() -> Self {
        Self {
            supports_sample_rate: false,
            sample_rates_hz: Vec::new(),
            supports_gain_mode: false,
            supports_gain: false,
            gain_values_db: Vec::new(),
            supports_ppm_correction: false,
            ppm_min: 0,
            ppm_max: 0,
            supports_direct_sampling: false,
            direct_sampling_modes: Vec::new(),
            direct_sampling_freq_hz_max: 0,

            tuner_freq_hz_min: 0,
            tuner_freq_hz_max: 0,

            supports_transmit: false,
            supports_dual_watch: false,
            supports_tx_tune_test: false,
            supports_band_control: false,
            supports_fdx: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n2adr_defaults_on() {
        // Safe-by-default: harmonic filtering engaged by default on the HL2.
        assert!(SourceControlState::default().n2adr_enabled);
        // Legacy persisted state that predates the field also defaults on
        // (serde fills the missing field from `default_n2adr_enabled`).
        let json = serde_json::to_value(SourceControlState::default()).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        obj.remove("n2adr_enabled");
        let restored: SourceControlState =
            serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert!(restored.n2adr_enabled);
    }
}
