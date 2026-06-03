use serde::{Deserialize, Serialize};
use rigflow_core::{
    radio::{
	HardwareKind,
	LeaseId,
	RadioCapabilities,
	RadioId,
	source_control::{
            DirectSamplingMode,
            GainMode,
            SourceCapabilities,
            SourceControlState,
        },
        source_status::SourceStatus,
        swr_sweep::{SwrSweepProgress, SwrSweepResult},
        tx_tune::TxTuneResult,
    },
    dsp::modes::{DemodMode, Sideband, DeemphasisMode},
};

/// Default squelch open threshold (dBFS) for `#[serde(default)]` decoding.
pub fn default_squelch_threshold_db() -> f32 {
    -90.0
}

/// Default squelch gate state (open = audio passing) for `#[serde(default)]`.
pub fn default_squelch_open() -> bool {
    true
}

/// Default NR2 strength for `#[serde(default)]` decoding.
pub fn default_nr2_strength() -> f32 {
    0.5
}

/// Default AGC enabled state for `#[serde(default)]` decoding.
pub fn default_agc_enabled() -> bool {
    true
}

/// Default AGC strength for `#[serde(default)]` decoding.
pub fn default_agc_strength() -> f32 {
    0.5
}

/// Default S-meter signal level (dBm) for `#[serde(default)]` decoding.
pub fn default_signal_dbm() -> f32 {
    -140.0
}

/// Default receive-audio volume percent for `#[serde(default)]` decoding.
pub fn default_volume_percent() -> u8 {
    50
}

/// Messages sent from client → server over WebSocket.
///
/// These drive:
/// - radio discovery
/// - lease lifecycle
/// - initial stream configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRadioMessage {
    /// Request a list of available radios.
    ListRadios,

    /// Acquire a lease on a radio and start streaming.
    ///
    /// Includes initial tuning parameters and UDP endpoints.
    AcquireRadio {
        radio_id: RadioId,

        /// Initial center frequency (Hz)
        center_freq_hz: u64,

        /// Initial tuned frequency (Hz)
        target_freq_hz: u64,

        /// UDP endpoint for audio (e.g. "ip:port")
        audio_udp_peer: String,

        /// UDP endpoint for waterfall (e.g. "ip:port")
        waterfall_udp_peer: String,
    },

    /// Release the currently held radio lease.
    ReleaseRadio,

    /// Renew the current lease before expiration.
    RenewLease,

    SetCenterFrequency {
        center_freq_hz: u64,
    },

    SetTargetFrequency {
        target_freq_hz: u64,
    },

    SetDemodMode {
        mode: DemodMode,
    },

    SetSideband {
        sideband: Sideband,
    },

    SetPitch {
        pitch_hz: f32,
    },

    SetFilterBandwidth {
        bandwidth_hz: f32,
    },

    SetDeemphasisMode {
    mode: DeemphasisMode,
    },

    /// Enable/disable receive squelch (radio control, DSP-side).
    SetSquelchEnabled {
        enabled: bool,
    },

    /// Set the squelch open threshold in dBFS (radio control, DSP-side).
    SetSquelchThreshold {
        threshold_db: f32,
    },

    /// Enable/disable NR2 spectral noise reduction (radio control, DSP-side).
    SetNr2Enabled {
        enabled: bool,
    },

    /// Set NR2 strength in [0.0, 1.0] (0 = none, 1 = max) (radio control).
    SetNr2Strength {
        strength: f32,
    },

    /// Enable/disable AGC (radio control, DSP-side).
    SetAgcEnabled {
        enabled: bool,
    },

    /// Set AGC strength in [0.0, 1.0] (radio control, DSP-side).
    SetAgcStrength {
        strength: f32,
    },

    /// Set receive-audio volume in percent (0–100) (radio control, DSP-side).
    SetVolume {
        volume_percent: u8,
    },

    SetSourceSampleRate {
	sample_rate_hz: u32,
    },

    SetSourceGainMode {
	mode: GainMode,
    },

    SetSourceGain {
	gain_db: f32,
    },

    SetSourcePpmCorrection {
	ppm: i32,
    },

    SetSourceDirectSampling {
	mode: DirectSamplingMode,
    },

    /// Set the transmit drive level in percent (0–100).  Part of source
    /// control; the server applies/persists it like other source settings and
    /// uses it for transmit operations (Spot/SWR).
    SetSourceTxDrive {
        tx_drive_percent: f32,
    },

    /// Enable/disable the N2ADR HF filter board (HL2).  Part of source control;
    /// when enabled the server programs the band filter from the tuned freq.
    SetSourceN2adrEnabled {
        enabled: bool,
    },

    /// Enable/disable FDX (TX Monitor Spectrum, HL2).  Part of source control;
    /// when enabled the server forwards RX IQ captured during Spot/SWR into the
    /// receive DSP pipeline so the spectrum/waterfall stay live during transmit.
    SetSourceFdxEnabled {
        enabled: bool,
    },

    /// Request a Spot / SWR measurement: a short, pure, unmodulated carrier at
    /// the current TX frequency.  TX power comes from the configured source
    /// `tx_drive_percent`; the server clamps duration and drive to safe limits.
    RequestTxTuneTest {
        /// Pulse duration in milliseconds; server clamps to a safe maximum.
        duration_ms: u32,
    },

    /// Request an SWR sweep across `[start_hz, stop_hz]` (one band, 25 points).
    /// The server validates the range and runs Spot/SWR at each point.
    RequestSwrSweep {
        start_hz: u64,
        stop_hz: u64,
    },

    /// Cancel an in-flight SWR sweep.
    CancelSwrSweep,
}

/// Messages sent from server → client over WebSocket.
///
/// These cover:
/// - discovery results
/// - lease lifecycle
/// - runtime state updates
/// - error reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerRadioMessage {
    /// Response to `ListRadios`.
    RadiosListed {
        radios: Vec<RadioInfo>,
    },

    /// Lease successfully acquired.
    RadioAcquired {
        radio_id: RadioId,
        lease_id: LeaseId,

        /// Lease time-to-live in milliseconds
        lease_ttl_ms: u64,
    },

    /// Lease released (either by client or server).
    RadioReleased {
        radio_id: RadioId,
    },

    /// Lease successfully renewed.
    LeaseRenewed {
        radio_id: RadioId,

        /// Updated lease TTL in milliseconds
        lease_ttl_ms: u64,
    },

    /// Full runtime state snapshot.
    ///
    /// Sent:
    /// - immediately after acquiring a radio
    /// - when a client reconnects or needs full state
    RuntimeSnapshot {
        radio_id: RadioId,

        center_freq_hz: u64,
        target_freq_hz: u64,

        input_sample_rate_hz: f32,

        /// Audio output configuration
        audio_sample_rate_hz: u32,
        audio_format: String,

        /// Waterfall configuration
        waterfall_bins: u32,
        waterfall_frame_rate_hz: f32,

        /// Current demodulation state
        demod_mode: DemodMode,
        sideband: Sideband,
	
        ssb_pitch_hz: f32,
	cw_pitch_hz: f32,
	filter_bandwidth_hz: f32,
	deemphasis_mode: DeemphasisMode,

        /// Receive squelch (radio control).  `#[serde(default)]` keeps older
        /// peers that omit these decodable.
        #[serde(default)]
        squelch_enabled: bool,
        #[serde(default = "default_squelch_threshold_db")]
        squelch_threshold_db: f32,
        /// Whether the squelch gate is currently open (audio passing).
        #[serde(default = "default_squelch_open")]
        squelch_open: bool,

        /// NR2 spectral noise reduction enabled (radio control).
        #[serde(default)]
        nr2_enabled: bool,
        /// NR2 strength in [0.0, 1.0].
        #[serde(default = "default_nr2_strength")]
        nr2_strength: f32,

        /// AGC (automatic gain control) — radio control.
        #[serde(default = "default_agc_enabled")]
        agc_enabled: bool,
        #[serde(default = "default_agc_strength")]
        agc_strength: f32,

        /// S-meter (read-only status): uncalibrated relative dBm + S-units 0..=9.
        #[serde(default = "default_signal_dbm")]
        signal_dbm: f32,
        #[serde(default)]
        signal_s_units: i32,

        /// Receive-audio volume in percent (0–100).
        #[serde(default = "default_volume_percent")]
        volume_percent: u8,

	source_control: SourceControlState,

        /// Current source telemetry / status fields.
        source_status: SourceStatus,

        /// Result of the most recent TX tune test, if any.
        /// `None` means no test has been run since acquisition.
        tx_tune_result: Option<TxTuneResult>,

        /// Result of the most recent SWR sweep, if any.
        #[serde(default)]
        swr_sweep_result: Option<SwrSweepResult>,
        /// Live SWR-sweep progress (`running=false` when idle/done).
        #[serde(default)]
        swr_sweep_progress: Option<SwrSweepProgress>,
    },

    /// Incremental runtime update.
    ///
    /// Only fields that changed are populated.
    RuntimeChanged {
        radio_id: RadioId,

        center_freq_hz: Option<u64>,
        target_freq_hz: Option<u64>,

        demod_mode: Option<DemodMode>,
        sideband: Option<Sideband>,

        ssb_pitch_hz: Option<f32>,
	cw_pitch_hz: Option<f32>,
	filter_bandwidth_hz: Option<f32>,
	deemphasis_mode: Option<DeemphasisMode>,

        #[serde(default)]
        squelch_enabled: Option<bool>,
        #[serde(default)]
        squelch_threshold_db: Option<f32>,
        #[serde(default)]
        squelch_open: Option<bool>,
        #[serde(default)]
        nr2_enabled: Option<bool>,
        #[serde(default)]
        nr2_strength: Option<f32>,
        #[serde(default)]
        agc_enabled: Option<bool>,
        #[serde(default)]
        agc_strength: Option<f32>,
        #[serde(default)]
        signal_dbm: Option<f32>,
        #[serde(default)]
        signal_s_units: Option<i32>,
        #[serde(default)]
        volume_percent: Option<u8>,

	source_control: Option<SourceControlState>,

        /// Changed source telemetry; `None` means no change since last update.
        source_status: Option<SourceStatus>,

        /// New TX tune test result; `None` means no change since last update.
        tx_tune_result: Option<TxTuneResult>,

        #[serde(default)]
        swr_sweep_result: Option<SwrSweepResult>,
        #[serde(default)]
        swr_sweep_progress: Option<SwrSweepProgress>,
    },

    /// Error message related to radio control or streaming.
    RadioError {
        /// Machine-readable error code
        code: String,

        /// Human-readable description
        message: String,
    },
}

/// Information about a radio exposed to clients.
///
/// This extends the core `RadioDescriptor` with runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioInfo {
    /// Unique identifier
    pub id: RadioId,

    /// Display name for UI
    pub display_name: String,

    /// Hardware/source type
    pub hardware_kind: HardwareKind,

    /// Device index (e.g., RTL device index)
    pub index: u32,

    /// Optional hardware serial number
    pub serial: Option<String>,

    /// Static capabilities
    pub radio_capabilities: RadioCapabilities,

    /// Source capabilities
    pub source_capabilities: SourceCapabilities,

    /// Current availability state
    pub state: RadioAvailability,

    /// Whether this radio is currently leased by any client
    pub is_leased: bool,
}

/// Runtime availability state of a radio.
///
/// This represents the lifecycle of a radio worker.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioAvailability {
    /// Ready for acquisition
    Available,

    /// Worker is starting up
    Starting,

    /// Actively running and streaming
    Running,

    /// Worker is shutting down
    Stopping,

    /// Error state (requires recovery or restart)
    Faulted,
}
