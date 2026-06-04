use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rigflow_core::dsp::modes::DeemphasisMode;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::{
    source_control::{DirectSamplingMode, GainMode, SourceControlState},
    source_status::SourceStatus,
    swr_sweep::{SwrSweepProgress, SwrSweepResult},
    tx_audio_diag::TxAudioDiag,
    tx_tune::TxTuneResult,
    LeaseId, RadioDescriptor, RadioId,
};

/// Unique identifier for a connected client/session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

/// Lifecycle state of a radio within the manager.
#[derive(Debug, Clone)]
pub enum RadioState {
    Available,
    Starting,
    Running,
    Stopping,
    Faulted { reason: String },
}

/// Active lease for a radio.
#[derive(Debug, Clone)]
pub struct LeaseRecord {
    pub lease_id: LeaseId,
    pub client_id: ClientId,
    pub acquired_at: Instant,
    pub last_renewed_at: Instant,
    pub expires_at: Instant,
}

/// Request to acquire and start a radio worker.
#[derive(Debug, Clone)]
pub struct AcquireRequest {
    pub center_freq_hz: u64,
    pub target_freq_hz: u64,
    pub audio_udp_peer: SocketAddr,
    pub waterfall_udp_peer: SocketAddr,
}

/// Result returned when a radio is successfully acquired.
#[derive(Debug, Clone)]
pub struct AcquireRadioResult {
    pub radio_id: RadioId,
    pub lease_id: LeaseId,
    pub lease_expires_at: Instant,
}

/// Summary of a radio for listing APIs.
#[derive(Debug, Clone)]
pub struct RadioSummary {
    pub descriptor: RadioDescriptor,
    pub state: RadioState,
    pub is_leased: bool,
}

/// Reason a worker is being stopped.
#[derive(Debug, Clone)]
pub enum StopReason {
    ClientRelease,
    LeaseExpired,
    ClientDisconnected,
    ServerShutdown,
    StartupFailed,
    InternalFault,
    UserRequested,
}

/// Runtime state snapshot of a worker.
///
/// This is sent to clients via RuntimeSnapshot / RuntimeChanged.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerRuntimeState {
    pub center_freq_hz: u64,
    pub target_freq_hz: u64,
    pub demod_mode: DemodMode,
    pub sideband: Sideband,
    pub ssb_pitch_hz: f32,
    pub cw_pitch_hz: f32,
    pub filter_bandwidth_hz: f32,
    pub deemphasis_mode: DeemphasisMode,
    pub squelch_enabled: bool,
    pub squelch_threshold_db: f32,
    pub squelch_open: bool,
    pub nr2_enabled: bool,
    pub nr2_strength: f32,
    pub agc_enabled: bool,
    pub agc_strength: f32,
    pub signal_dbm: f32,
    pub signal_s_units: i32,
    pub volume_percent: u8,
    pub source_control: SourceControlState,
    pub source_status: SourceStatus,
    /// Live TX-audio diagnostics for SSB mic transmit (zero when idle).
    pub tx_audio_diag: TxAudioDiag,
    /// Result of the most recent TX tune test executed by this worker.
    /// `None` until a RequestTxTuneTest command has been processed.
    pub last_tx_tune_result: Option<TxTuneResult>,
    /// Result of the most recent SWR sweep, and live progress.
    pub last_swr_sweep_result: Option<SwrSweepResult>,
    pub swr_sweep_progress: Option<SwrSweepProgress>,

    pub input_sample_rate_hz: f32,
    pub audio_sample_rate_hz: u32,
    pub audio_format: String,
    pub waterfall_bins: u32,
    pub waterfall_frame_rate_hz: f32,
}

/// Commands sent from server/session → worker.
#[derive(Debug, Clone)]
pub enum WorkerCommand {
    SetTargetFrequency {
        hz: u64,
    },
    SetCenterFrequency {
        hz: u64,
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
    SetSquelchEnabled {
        enabled: bool,
    },
    SetSquelchThreshold {
        threshold_db: f32,
    },
    SetNr2Enabled {
        enabled: bool,
    },
    SetNr2Strength {
        strength: f32,
    },
    SetAgcEnabled {
        enabled: bool,
    },
    SetAgcStrength {
        strength: f32,
    },
    SetVolume {
        volume_percent: u8,
    },
    Stop {
        reason: StopReason,
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
    SetSourceTxDrive {
        tx_drive_percent: f32,
    },
    SetSourceSpotLevel {
        spot_level_percent: f32,
    },
    SetSourceTxSequencing {
        lead_ms: u32,
        tail_ms: u32,
    },
    /// CW key down / up (Space bar).  The server keys the CW carrier with
    /// envelope shaping; `tx_drive_percent` / `spot_level_percent` set power.
    StartCwKey,
    StopCwKey,
    /// CW semi-break-in hang time in ms (0–2000): PTT persists this long after
    /// the last element before release.
    SetCwHangTime {
        hang_ms: u32,
    },
    /// SSB microphone transmit start/stop (Space bar in USB/LSB).  The server
    /// modulates the mic-audio UDP stream; sideband comes from the current mode.
    StartMicTx,
    StopMicTx,
    /// Reset the TX-audio underrun/overrun diagnostic counters.
    ResetTxAudioDiag,
    SetSourceN2adrEnabled {
        enabled: bool,
    },
    SetSourceFdxEnabled {
        enabled: bool,
    },
    RequestSwrSweep {
        start_hz: u64,
        stop_hz: u64,
    },
    CancelSwrSweep,
    /// Request a Spot / SWR measurement (pure carrier pulse).  TX power comes
    /// from the configured source `tx_drive_percent`.
    RequestTxTuneTest {
        duration_ms: u32,
    },
    /// Start an open-ended SSB test tone (FDX Phase 2).  `usb = true` places the
    /// tone above the carrier (USB), `false` below (LSB).  Amplitude comes from
    /// `source_control.spot_level_percent`, drive from `tx_drive_percent`.
    StartTxTestTone {
        tone_hz: f32,
        usb: bool,
    },
    /// Stop a running TX test tone (release PTT, return to RX).
    StopTxTestTone,
}

/// Worker lifecycle/status updates.
#[derive(Debug, Clone)]
pub enum WorkerStatus {
    Starting,

    Running { runtime: WorkerRuntimeState },

    Stopping { reason: StopReason },

    Stopped { reason: StopReason },

    Faulted { reason: String },
}

/// Initial readiness payload returned during worker startup.
#[derive(Debug, Clone)]
pub struct WorkerReadyInfo {
    pub runtime: WorkerRuntimeState,
}

/// Result of worker startup handshake.
#[derive(Debug)]
pub enum WorkerStartResult {
    Ready(WorkerReadyInfo),
    Failed(String),
}

/// Final exit state of a worker.
#[derive(Debug)]
pub enum WorkerExit {
    Clean { reason: StopReason },
    Failed { reason: String },
}

/// Configuration for RadioManager timing behavior.
#[derive(Debug, Clone)]
pub struct RadioManagerConfig {
    pub lease_ttl: Duration,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
}

/// Errors returned by RadioManager operations.
#[derive(Debug)]
pub enum RadioManagerError {
    RadioNotFound,
    RadioBusy,
    NotLeaseOwner,
    NoActiveLease,
    InvalidLease,
    RadioNotRunning,
    StartupFailed(String),
    StartupTimedOut,
    ShutdownTimedOut,
    WorkerChannelClosed,
    Internal(String),
}
