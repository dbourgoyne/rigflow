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
}

impl Default for SourceControlState {
    fn default() -> Self {
        Self {
            sample_rate_hz: 2_048_000,
            gain_mode: GainMode::Auto,
            gain_db: 0.0,
            ppm_correction: 0,
            direct_sampling: DirectSamplingMode::Off,
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

    /// Whether the source supports a TX tune test (short low-power carrier
    /// pulse used to measure forward/reverse power and SWR).
    ///
    /// This is a capability flag only. The actual TX tune test protocol
    /// is not yet implemented; the UI skeleton is always disabled while
    /// this is `false`.
    pub supports_tx_tune_test: bool,
}

impl SourceCapabilities {
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

            supports_tx_tune_test: false,
        }
    }
}
