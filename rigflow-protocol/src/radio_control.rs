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
    },
    dsp::modes::{DemodMode, Sideband, DeemphasisMode},
};

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

	source_control: SourceControlState,

        /// Current source telemetry / status fields.
        source_status: SourceStatus,
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

	source_control: Option<SourceControlState>,

        /// Changed source telemetry; `None` means no change since last update.
        source_status: Option<SourceStatus>,
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
