use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rigflow_core::dsp::demod::{DemodMode, Sideband};
use rigflow_core::radio::{LeaseId, RadioDescriptor, RadioId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

#[derive(Debug, Clone)]
pub enum RadioState {
    Available,
    Starting,
    Running,
    Stopping,
    Faulted { reason: String },
}

#[derive(Debug, Clone)]
pub struct LeaseRecord {
    pub lease_id: LeaseId,
    pub client_id: ClientId,
    pub acquired_at: Instant,
    pub last_renewed_at: Instant,
    pub expires_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AcquireRequest {
    pub center_freq_hz: u64,
    pub target_freq_hz: u64,
    pub audio_udp_peer: SocketAddr,
    pub waterfall_udp_peer: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct AcquireRadioResult {
    pub radio_id: RadioId,
    pub lease_id: LeaseId,
    pub lease_expires_at: Instant,
}

#[derive(Debug, Clone)]
pub struct RadioSummary {
    pub descriptor: RadioDescriptor,
    pub state: RadioState,
    pub is_leased: bool,
}

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

#[derive(Debug, Clone)]
pub struct WorkerRuntimeState {
    pub center_freq_hz: u64,
    pub target_freq_hz: u64,
    pub demod_mode: DemodMode,
    pub sideband: Sideband,
    pub ssb_pitch_hz: f32,

    pub input_sample_rate_hz: f32,
    pub audio_sample_rate_hz: u32,
    pub audio_format: String,
    pub waterfall_bins: u32,
    pub waterfall_frame_rate_hz: f32,
}

#[derive(Debug, Clone)]
pub enum WorkerCommand {
    SetTargetFrequency { hz: u64 },
    SetCenterFrequency { hz: u64 },
    SetDemodMode { mode: DemodMode },
    SetSideband { sideband: Sideband },
    SetSsbPitch { pitch_hz: f32 },
    Stop { reason: StopReason },
}

#[derive(Debug, Clone)]
pub enum WorkerStatus {
    Starting,
    Running {
        runtime: WorkerRuntimeState,
    },
    Stopping {
        reason: StopReason,
    },
    Stopped {
        reason: StopReason,
    },
    Faulted {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct WorkerReadyInfo {
    pub runtime: WorkerRuntimeState,
}

#[derive(Debug)]
pub enum WorkerStartResult {
    Ready(WorkerReadyInfo),
    Failed(String),
}

#[derive(Debug)]
pub enum WorkerExit {
    Clean { reason: StopReason },
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub struct RadioManagerConfig {
    pub lease_ttl: Duration,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
}

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
