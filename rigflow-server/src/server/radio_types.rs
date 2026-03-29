use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RadioId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeaseId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareKind {
    RtlSdr,
    Soapy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioState {
    Available,
    Starting,
    Running,
    Stopping,
    Faulted,
}

#[derive(Debug, Clone)]
pub struct RadioCapabilities {
    pub min_freq_hz: u64,
    pub max_freq_hz: u64,
    pub max_sample_rate_hz: u32,
    pub supports_wfm: bool,
    pub supports_nfm: bool,
    pub supports_usb: bool,
    pub supports_lsb: bool,
}

#[derive(Debug, Clone)]
pub struct RadioDescriptor {
    pub id: RadioId,
    pub display_name: String,
    pub hardware_kind: HardwareKind,
    pub index: u32,
    pub serial: Option<String>,
    pub capabilities: RadioCapabilities,
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
