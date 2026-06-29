use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rigflow_core::dsp::modes::DeemphasisMode;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::{
    amplifier::{AmplifierAtuMode, AmplifierKeyingMode, AmplifierStatus},
    iq_recording::IqRecordingStatus,
    source_control::{DirectSamplingMode, GainMode, SourceControlState},
    source_status::SourceStatus,
    swr_sweep::{SwrSweepProgress, SwrSweepResult},
    tx_audio_diag::TxAudioDiag,
    tx_tune::TxTuneResult,
    vfo::VfoSelect,
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

/// Dual-VFO / split / RIT-XIT state (VFO B is independent: own mode + filter).
/// Grouped so the many worker / snapshot / delta sites touch a single field.
/// Defaults make pre-dual-watch behavior byte-identical.
#[derive(Debug, Clone, PartialEq)]
pub struct VfoSplitState {
    pub vfo_b_target_freq_hz: u64,
    /// VFO B's own hardware-receiver center (RX1 NCO); independent of VFO A.
    pub vfo_b_center_freq_hz: u64,
    pub vfo_b_demod_mode: DemodMode,
    pub vfo_b_sideband: Sideband,
    pub vfo_b_filter_bandwidth_hz: f32,
    pub vfo_b_ssb_pitch_hz: f32,
    pub vfo_b_cw_pitch_hz: f32,
    /// VFO B independent receive processing (own DSP pipeline under dual-watch).
    /// Mirrors the like-named `SharedControlState` fields; copied into `control_b`
    /// each loop so the reused DSP-B thread honours them. `nb_*` / `notch_auto`
    /// are control-only (not echoed in the protocol snapshot, matching VFO A).
    pub vfo_b_deemphasis_mode: DeemphasisMode,
    pub vfo_b_squelch_enabled: bool,
    pub vfo_b_squelch_threshold_db: f32,
    /// Live VFO-B squelch gate (open = audio passing); written by the DSP-B
    /// thread and read back into the snapshot, like `vfo_b_signal_*`.
    pub vfo_b_squelch_open: bool,
    pub vfo_b_nr2_enabled: bool,
    pub vfo_b_nr2_strength: f32,
    pub vfo_b_nb_enabled: bool,
    pub vfo_b_nb_threshold: f32,
    pub vfo_b_notch_auto_enabled: bool,
    pub vfo_b_agc_enabled: bool,
    pub vfo_b_agc_strength: f32,
    /// RIT for VFO B (independent of VFO A's `rit_*`); offsets only VFO B's RX NCO.
    pub vfo_b_rit_enabled: bool,
    pub vfo_b_rit_offset_hz: i32,
    /// VFO B waterfall frame rate (Hz, 0 = off); paces the DSP-B waterfall thread
    /// independently of VFO A.
    pub vfo_b_waterfall_frame_rate_hz: f32,
    pub vfo_b_signal_dbm: f32,
    pub vfo_b_signal_s_units: i32,
    pub rit_enabled: bool,
    pub rit_offset_hz: i32,
    pub xit_enabled: bool,
    pub xit_offset_hz: i32,
    pub split_enabled: bool,
    pub tx_vfo: VfoSelect,
    pub dual_watch_enabled: bool,
    /// True when the source has a second hardware receiver (HL2). Static per
    /// source; set at worker start from `IqSource::max_receivers() >= 2`.
    pub dual_watch_supported: bool,
}

impl Default for VfoSplitState {
    fn default() -> Self {
        Self {
            vfo_b_target_freq_hz: 0,
            vfo_b_center_freq_hz: 0,
            vfo_b_demod_mode: DemodMode::Usb,
            vfo_b_sideband: Sideband::Usb,
            vfo_b_filter_bandwidth_hz: 2700.0,
            vfo_b_ssb_pitch_hz: 0.0,
            vfo_b_cw_pitch_hz: 600.0,
            vfo_b_deemphasis_mode: DeemphasisMode::Off,
            vfo_b_squelch_enabled: false,
            vfo_b_squelch_threshold_db: -90.0,
            vfo_b_squelch_open: true,
            vfo_b_nr2_enabled: false,
            vfo_b_nr2_strength: 0.5,
            vfo_b_nb_enabled: false,
            vfo_b_nb_threshold: 0.5,
            vfo_b_notch_auto_enabled: false,
            vfo_b_agc_enabled: true,
            vfo_b_agc_strength: 0.5,
            vfo_b_rit_enabled: false,
            vfo_b_rit_offset_hz: 0,
            vfo_b_waterfall_frame_rate_hz: 20.0,
            vfo_b_signal_dbm: -140.0,
            vfo_b_signal_s_units: 0,
            rit_enabled: false,
            rit_offset_hz: 0,
            xit_enabled: false,
            xit_offset_hz: 0,
            split_enabled: false,
            tx_vfo: VfoSelect::A,
            dual_watch_enabled: false,
            dual_watch_supported: false,
        }
    }
}

/// Runtime state snapshot of a worker.
///
/// This is sent to clients via RuntimeSnapshot / RuntimeChanged.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerRuntimeState {
    pub center_freq_hz: u64,
    pub target_freq_hz: u64,
    /// Dual-VFO / split / RIT-XIT state.
    pub vfo: VfoSplitState,
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
    /// Attached amplifier status (Phase 1: HR50). `model: None` = no amp.
    pub amplifier_status: AmplifierStatus,
    /// Live receive IQ recording status (Phase 1).
    pub iq_recording_status: IqRecordingStatus,
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
    SetNoiseBlankerEnabled {
        enabled: bool,
    },
    SetNoiseBlankerThreshold {
        threshold: f32,
    },
    SetNotchAutoEnabled {
        enabled: bool,
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
    SetWaterfallFrameRate {
        rate_hz: f32,
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
    // ── Dual-VFO / split / RIT-XIT ──
    SetVfoBFrequency {
        hz: u64,
    },
    SetVfoBCenterFrequency {
        hz: u64,
    },
    SetVfoBDemodMode {
        mode: DemodMode,
    },
    SetVfoBSideband {
        sideband: Sideband,
    },
    SetVfoBFilterBandwidth {
        bandwidth_hz: f32,
    },
    SetVfoBPitch {
        pitch_hz: f32,
    },
    /// VFO B independent receive controls (mirror the VFO-A setters).
    SetVfoBDeemphasisMode {
        mode: DeemphasisMode,
    },
    SetVfoBSquelchEnabled {
        enabled: bool,
    },
    SetVfoBSquelchThreshold {
        threshold_db: f32,
    },
    SetVfoBNr2Enabled {
        enabled: bool,
    },
    SetVfoBNr2Strength {
        strength: f32,
    },
    SetVfoBNoiseBlankerEnabled {
        enabled: bool,
    },
    SetVfoBNoiseBlankerThreshold {
        threshold: f32,
    },
    SetVfoBNotchAutoEnabled {
        enabled: bool,
    },
    SetVfoBAgcEnabled {
        enabled: bool,
    },
    SetVfoBAgcStrength {
        strength: f32,
    },
    SetVfoBRit {
        enabled: bool,
        offset_hz: i32,
    },
    SetVfoBWaterfallFrameRate {
        rate_hz: f32,
    },
    /// Clone VFO A's entire receiver state onto VFO B (the "A=B" copy).
    CopyVfoAToB,
    SetRit {
        enabled: bool,
        offset_hz: i32,
    },
    SetXit {
        enabled: bool,
        offset_hz: i32,
    },
    SetSplit {
        enabled: bool,
    },
    SetTxVfo {
        vfo: VfoSelect,
    },
    SetDualWatch {
        enabled: bool,
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
    /// Configure the SSB two-tone test generator (mic-TX path tone source).
    SetTwoToneTest {
        enabled: bool,
        tone_a_hz: f32,
        tone_b_hz: f32,
        level_percent: f32,
    },
    /// Configure the TX soft peak limiter (ALC Phase 1).
    SetTxLimiter {
        enabled: bool,
        threshold_percent: f32,
    },
    /// Configure the SSB speech compressor (before the limiter).
    SetCompression {
        enabled: bool,
        level: u8,
    },
    /// Start / stop receive IQ recording (Phase 1).
    StartIqRecording,
    StopIqRecording,
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

    /// Set the attached amplifier's keying mode (HR50: OFF/PTT/COR/QRP).
    SetAmplifierKeyingMode {
        mode: AmplifierKeyingMode,
    },
    /// Set the amplifier ATU engagement mode (bypass/active).
    SetAmplifierAtuMode {
        mode: AmplifierAtuMode,
    },
    /// Ask the amplifier ATU to tune on the next transmission.
    TuneAmplifierAtu,
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

/// Transient, fire-and-forget event from a worker thread to the leasing client —
/// an asynchronous failure not tied to a specific command (e.g. a cross-band TX
/// aborted because the HR50 band change couldn't be confirmed before RF).
/// Delivered over a broadcast channel and surfaced as `ServerRadioMessage::RadioError`.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Error { code: String, message: String },
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
