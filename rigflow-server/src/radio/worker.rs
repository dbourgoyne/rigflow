use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    mpsc as std_mpsc, Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, trace, warn};
use num_complex::Complex32;
use tokio::sync::{mpsc, oneshot, watch};

use rigflow_core::dsp::modes::{default_deemphasis_mode, DeemphasisMode};
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::{HardwareKind, RadioDescriptor};

use crate::config::{
    choose_block_size, choose_decimation, ServerConfig, SourceKind, WATERFALL_BINS,
    WATERFALL_FRAME_RATE_HZ,
};
use crate::dsp::audio::nr2::Nr2;
use crate::dsp::pipeline::{DspPipeline, DspPipelineConfig};
use crate::net::udp::udp_audio::UdpAudioSender;
use crate::net::udp::udp_waterfall::UdpWaterfallSender;
use crate::radio::types::{
    AcquireRequest, StopReason, WorkerCommand, WorkerExit, WorkerReadyInfo, WorkerRuntimeState,
    WorkerStartResult, WorkerStatus,
};
use crate::source::factory::{create_source, SourceConfig};
use crate::source::wav_metadata::parse_center_freq_hz_from_filename;
use crate::source::IqSource;
use crate::waterfall::generator::WaterfallGenerator;
use rigflow_core::radio::amplifier::AmplifierStatus;
use rigflow_core::radio::ham_band::{band_from_frequency, n2adr_filter_value_for_band, HamBand};
use rigflow_core::radio::iq_recording::IqRecordingStatus;
use rigflow_core::radio::source_control::{DirectSamplingMode, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::swr_sweep::{
    sweep_frequency_hz, validate_sweep_range, SwrSweepPoint, SwrSweepProgress, SwrSweepResult,
    SWR_SWEEP_POINTS,
};
use rigflow_core::radio::tx_tune::TxTuneResult;

/// How often to re-send a C&C packet to the HL2 when the user isn't tuning.
/// Observed watchdog timeout is ~15 s; 1 s gives a large safety margin.
const HL2_CC_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);

/// Sustained RX-loss window on a realtime source before the worker gives up.
/// Below this a transient gap is tolerated in place (worker stays alive,
/// surfacing "not responding"); beyond it the worker fails so the client
/// re-acquires and re-initializes a power-cycled device.
const RX_STALL_GIVE_UP: Duration = Duration::from_secs(10);

/// Baud used for an explicit `--hr50-serial <path>` with no `:baud` suffix, and
/// the baud tried first when auto-detecting.  (The amp's ACC default.)
const HR50_DEFAULT_BAUD: u32 = 19200;

/// Spot/SWR pulse duration used for each SWR-sweep point (same as a single Spot).
const SWR_SWEEP_SPOT_MS: u32 = 250;

#[derive(Debug, Clone)]
struct SharedControlState {
    center_freq_hz: u64,
    target_freq_hz: u64,
    demod_mode: DemodMode,
    sideband: Sideband,
    ssb_pitch_hz: f32,
    cw_pitch_hz: f32,
    filter_bandwidth_hz: f32,
    pub deemphasis_mode: DeemphasisMode,
    /// Receive squelch (radio control).  enabled/threshold set by the command
    /// thread; `squelch_open` is the live gate state written by the DSP thread.
    squelch_enabled: bool,
    squelch_threshold_db: f32,
    squelch_open: bool,
    /// NR2 spectral noise reduction enabled (radio control, DSP-side).
    nr2_enabled: bool,
    /// NR2 strength in [0.0, 1.0] (0 = none, 1 = max suppression).
    nr2_strength: f32,
    /// AGC (automatic gain control) — radio control, DSP-side.
    agc_enabled: bool,
    agc_strength: f32,
    /// S-meter (read-only status): smoothed channel signal strength.
    /// `signal_dbm` is an uncalibrated relative dBm; `signal_s_units` is 0..=9.
    /// Written by the DSP thread.
    signal_dbm: f32,
    signal_s_units: i32,
    /// Receive-audio volume in percent (0–100) — final audio gain stage.
    volume_percent: u8,
    pub source_control: SourceControlState,
    /// Latest telemetry polled from the IQ source (read-only, written by capture thread).
    pub source_status: SourceStatus,
    /// Latest amplifier status (read-only, written by the amplifier poller thread).
    pub amplifier_status: AmplifierStatus,

    /// Set by command thread when a Spot/SWR measurement arrives; consumed
    /// (taken) by the capture thread which owns the IQ source.  TX drive comes
    /// from `source_control.tx_drive_percent`, not from the request.
    pending_tx_tune_test: Option<u32>, // duration_ms

    /// Written by capture thread after executing a TX tune test dry run;
    /// read by DSP thread to detect changes and publish RuntimeChanged.
    last_tx_tune_result: Option<TxTuneResult>,

    /// SWR sweep: pending request (start,stop) set by the command thread and
    /// taken by the capture thread; cancel flag; result + live progress.
    pending_swr_sweep: Option<(u64, u64)>,
    swr_sweep_cancel: bool,
    last_swr_sweep_result: Option<SwrSweepResult>,
    swr_sweep_progress: Option<SwrSweepProgress>,

    /// TX test tone (FDX Phase 2): pending start request `(tone_hz, usb)` set by
    /// the command thread and taken by the capture thread; `tx_tone_stop` is the
    /// stop signal the capture thread polls while the (open-ended) tone runs.
    pending_tx_tone: Option<(f32, bool)>,
    tx_tone_stop: Arc<AtomicBool>,

    /// CW keying: `pending_cw_key` starts a keying session; `cw_key_held` is the
    /// live key state the session polls.  `cw_hang_ms` is the semi-break-in hang
    /// time — PTT stays asserted this long after the last element before release
    /// (0 = release immediately, like per-element keying).
    pending_cw_key: bool,
    cw_key_held: Arc<AtomicBool>,
    cw_hang_ms: u32,

    /// SSB mic TX (Phase 3): `pending_mic_tx` starts a session; `mic_tx_active`
    /// is the live PTT state the session polls (cleared on key-up / stop).
    pending_mic_tx: bool,
    mic_tx_active: Arc<AtomicBool>,

    /// SSB two-tone test generator.  When `two_tone_enabled`, the mic-TX path
    /// generates `Tone A + Tone B` instead of draining the mic queue, so it
    /// flows through the identical DcBlocker → SSB FIR → diagnostics → HL2
    /// path as microphone audio.  `two_tone_level` is a 0..1 amplitude scale.
    two_tone_enabled: bool,
    two_tone_a_hz: f32,
    two_tone_b_hz: f32,
    two_tone_level: f32,

    /// TX soft peak limiter (ALC Phase 1).  `tx_limiter_threshold` is a fraction
    /// of full scale (UI percent / 100).  Read at mic-TX session start.
    tx_limiter_enabled: bool,
    tx_limiter_threshold: f32,

    /// Speech compressor (before the limiter).  `compressor_level` is the UI
    /// 0–10 level (mapped to a ratio).  Read at mic-TX session start.
    compressor_enabled: bool,
    compressor_level: u8,

    /// Receive IQ recording (Phase 1).  Start/stop are one-shot requests set by
    /// the command thread and consumed by the capture thread (which owns the
    /// recorder + IQ stream).  `iq_recording_status` is published telemetry.
    pending_iq_record_start: bool,
    pending_iq_record_stop: bool,
    iq_recording_status: IqRecordingStatus,
}

#[derive(Debug, Clone)]
struct StartupInfo {
    input_sample_rate_hz: f32,
    runtime: WorkerRuntimeState,
}

/// Async entry point that bridges the Tokio-facing server code into the
/// blocking multi-threaded radio worker implementation.
///
/// The rest of this worker is intentionally thread-based because the SDR
/// capture/DSP/waterfall pipeline is mostly blocking, CPU-oriented work.
pub async fn run_radio_worker(
    descriptor: RadioDescriptor,
    request: AcquireRequest,
    server_cfg: ServerConfig,
    mut worker_rx: mpsc::Receiver<WorkerCommand>,
    status_tx: watch::Sender<WorkerStatus>,
    mut stop_rx: oneshot::Receiver<()>,
    startup_tx: oneshot::Sender<WorkerStartResult>,
) -> WorkerExit {
    debug!(
        "[radio-worker {}] starting worker center={} target={}",
        descriptor.id.0, request.center_freq_hz, request.target_freq_hz
    );

    let (cmd_tx_std, cmd_rx_std) = std_mpsc::channel::<WorkerCommand>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_reason = Arc::new(Mutex::new(StopReason::InternalFault));
    let (exit_tx, exit_rx) = oneshot::channel::<WorkerExit>();

    // Forward commands from the async WebSocket/control world into the
    // blocking worker thread world.
    {
        let stop_flag = stop_flag.clone();

        tokio::spawn(async move {
            while let Some(cmd) = worker_rx.recv().await {
                if cmd_tx_std.send(cmd).is_err() {
                    break;
                }

                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
            }
        });
    }

    {
        let thread_descriptor = descriptor.clone();
        let thread_request = request.clone();
        let thread_server_cfg = server_cfg.clone();
        let thread_status_tx = status_tx.clone();
        let thread_stop_flag = stop_flag.clone();
        let thread_stop_reason = stop_reason.clone();

        thread::spawn(move || {
            let worker_name = thread_descriptor.id.0.clone();

            let exit = run_iq_worker_threads(
                thread_descriptor,
                thread_request,
                thread_server_cfg,
                cmd_rx_std,
                thread_status_tx,
                startup_tx,
                thread_stop_flag,
                thread_stop_reason,
            );

            debug!(
                "[radio-worker {}] async wrapper exiting with {:?}",
                worker_name, exit
            );

            let _ = exit_tx.send(exit);
        });
    }

    tokio::select! {
        _ = &mut stop_rx => {
            stop_flag.store(true, Ordering::Relaxed);

            if let Ok(mut reason) = stop_reason.lock() {
                *reason = StopReason::InternalFault;
            }

            WorkerExit::Clean {
                reason: StopReason::InternalFault,
            }
        }

        result = exit_rx => {
            match result {
                Ok(exit) => exit,
                Err(_) => WorkerExit::Failed {
                    reason: "worker thread exit channel closed".to_string(),
                },
            }
        }
    }
}

fn source_kind_for_descriptor(descriptor: &RadioDescriptor) -> Result<SourceKind, String> {
    if descriptor.id.0.starts_with("fake:") {
        Ok(SourceKind::Fake)
    } else if descriptor.id.0.starts_with("rtl:") {
        Ok(SourceKind::RtlSdr)
    } else if descriptor.id.0.starts_with("wav:") {
        Ok(SourceKind::Wav)
    } else if descriptor.id.0.starts_with("hl2:") {
        Ok(SourceKind::HermesLite2)
    } else {
        Err(format!("unsupported radio id '{}'", descriptor.id.0))
    }
}

fn create_worker_source(
    descriptor: &RadioDescriptor,
    server_cfg: &ServerConfig,
    block_size: usize,
    initial_center_freq_hz: u64,
) -> Result<Box<dyn IqSource>, String> {
    let config = if descriptor.id.0.starts_with("fake:") {
        SourceConfig::Fake {
            sample_rate_hz: server_cfg.fake_sample_rate_hz,
            tone_hz: server_cfg.fake_tone_hz,
        }
    } else if descriptor.id.0.starts_with("rtl:") {
        SourceConfig::RtlSdr {
            // Open the device this radio was enumerated as (so rtl:N → device N),
            // not a single global default.
            device_index: descriptor.index as usize,
            sample_rate_hz: server_cfg.rtlsdr_sample_rate_hz,
            center_freq_hz: initial_center_freq_hz as u32,
            gain_tenths_db: server_cfg.rtlsdr_gain_tenths_db,
            ppm_correction: server_cfg.rtlsdr_ppm_correction,
            direct_sampling: server_cfg.rtlsdr_direct_sampling,
            block_complex_samples: block_size,
        }
    } else if descriptor.id.0.starts_with("wav:") {
        let wav_path = descriptor
            .serial
            .as_ref()
            .ok_or_else(|| "wav radio missing file path".to_string())?
            .clone();

        SourceConfig::WavFile { path: wav_path }
    } else if descriptor.id.0.starts_with("hl2:") {
        let addr = descriptor
            .serial
            .as_ref()
            .ok_or_else(|| "HL2 radio missing device address in serial field".to_string())?
            .clone();
        SourceConfig::HermesLite2 {
            addr,
            sample_rate_hz: server_cfg.hl2_sample_rate_hz as f32,
            center_freq_hz: initial_center_freq_hz as f32,
        }
    } else {
        return Err(format!("source for {} not implemented", descriptor.id.0));
    };

    create_source(config)
}

fn pipeline_cfg_for_source(
    server_cfg: &ServerConfig,
    center_freq_hz: u64,
    target_freq_hz: u64,
    input_sample_rate_hz: f32,
) -> DspPipelineConfig {
    let (channel_cutoff_hz, audio_cutoff_hz) = match server_cfg.demod {
        DemodMode::Wfm => (100_000.0, 15_000.0),
        DemodMode::Nfm => (12_500.0, 5_000.0),
        DemodMode::Usb | DemodMode::DgtU => (4_000.0, 3_000.0),
        DemodMode::Lsb => (4_000.0, 3_000.0),
        DemodMode::Am => (6_000.0, 5_000.0),
        DemodMode::Cwu | DemodMode::Cwl => (1_200.0, 900.0),
    };

    DspPipelineConfig {
        center_freq_hz: center_freq_hz as f32,
        target_freq_hz: target_freq_hz as f32,
        input_sample_rate_hz,

        channel_cutoff_hz,
        fir_taps: 129,
        decimation_factor: choose_decimation(input_sample_rate_hz),

        audio_cutoff_hz,
        audio_fir_taps: 129,

        client_output_sample_rate_hz: 48_000.0,
        mode: server_cfg.demod,
    }
}

/// Soft limiter applied after the volume boost so loud/boosted audio can't
/// hard-clip harshly at the i16 conversion.
///
/// Transparent (identity) for `|x| <= THRESHOLD`, then a smooth tanh knee that
/// asymptotes to ±1.0.  Continuous in value and slope at the threshold; output
/// is always finite and bounded to [-1, 1].
fn soft_clip(x: f32) -> f32 {
    const THRESHOLD: f32 = 0.95;
    let a = x.abs();
    if a <= THRESHOLD {
        x
    } else {
        let over = (a - THRESHOLD) / (1.0 - THRESHOLD);
        x.signum() * (THRESHOLD + (1.0 - THRESHOLD) * over.tanh())
    }
}

fn build_runtime_state(
    control: &SharedControlState,
    input_sample_rate_hz: f32,
) -> WorkerRuntimeState {
    WorkerRuntimeState {
        center_freq_hz: control.center_freq_hz,
        target_freq_hz: control.target_freq_hz,
        demod_mode: control.demod_mode,
        sideband: control.sideband,
        ssb_pitch_hz: control.ssb_pitch_hz,
        cw_pitch_hz: control.cw_pitch_hz,
        filter_bandwidth_hz: control.filter_bandwidth_hz,
        deemphasis_mode: control.deemphasis_mode,
        squelch_enabled: control.squelch_enabled,
        squelch_threshold_db: control.squelch_threshold_db,
        squelch_open: control.squelch_open,
        nr2_enabled: control.nr2_enabled,
        nr2_strength: control.nr2_strength,
        agc_enabled: control.agc_enabled,
        agc_strength: control.agc_strength,
        signal_dbm: control.signal_dbm,
        signal_s_units: control.signal_s_units,
        volume_percent: control.volume_percent,
        source_control: control.source_control.clone(),
        source_status: control.source_status.clone(),
        amplifier_status: control.amplifier_status.clone(),
        iq_recording_status: control.iq_recording_status.clone(),
        tx_audio_diag: crate::tx_diag::snapshot(),
        last_tx_tune_result: control.last_tx_tune_result.clone(),
        last_swr_sweep_result: control.last_swr_sweep_result.clone(),
        swr_sweep_progress: control.swr_sweep_progress,

        input_sample_rate_hz,
        audio_sample_rate_hz: 48_000,
        audio_format: "i16".to_string(),
        waterfall_bins: WATERFALL_BINS as u32,
        waterfall_frame_rate_hz: WATERFALL_FRAME_RATE_HZ,
    }
}

fn current_control(control: &Arc<Mutex<SharedControlState>>) -> SharedControlState {
    control.lock().unwrap().clone()
}

fn set_stop_reason(stop_reason: &Arc<Mutex<StopReason>>, reason: StopReason) {
    if let Ok(mut current_reason) = stop_reason.lock() {
        *current_reason = reason;
    }
}

fn stop_requested(stop_flag: &Arc<AtomicBool>) -> bool {
    stop_flag.load(Ordering::Relaxed)
}

/// Coordinates the worker subthreads and owns the overall worker lifecycle.
fn run_iq_worker_threads(
    descriptor: RadioDescriptor,
    request: AcquireRequest,
    server_cfg: ServerConfig,
    cmd_rx: std_mpsc::Receiver<WorkerCommand>,
    status_tx: watch::Sender<WorkerStatus>,
    startup_tx: oneshot::Sender<WorkerStartResult>,
    stop_flag: Arc<AtomicBool>,
    stop_reason: Arc<Mutex<StopReason>>,
) -> WorkerExit {
    let source_kind = match source_kind_for_descriptor(&descriptor) {
        Ok(kind) => kind,
        Err(reason) => {
            let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
            return WorkerExit::Failed { reason };
        }
    };

    debug!("descriptor = {:?}", descriptor);

    let block_size = choose_block_size(&source_kind);

    let wav_center_freq_hz = if descriptor.id.0.starts_with("wav:") {
        descriptor
            .serial
            .as_ref()
            .and_then(|p| parse_center_freq_hz_from_filename(std::path::Path::new(p)))
    } else {
        None
    };

    let kind = descriptor.hardware_kind;

    let (initial_center_freq_hz, initial_target_freq_hz) =
        normalize_initial_frequencies(&request, &server_cfg, kind, wav_center_freq_hz);

    let control = Arc::new(Mutex::new(SharedControlState {
        center_freq_hz: initial_center_freq_hz,
        target_freq_hz: initial_target_freq_hz,
        demod_mode: server_cfg.demod,
        sideband: Sideband::Lsb,
        ssb_pitch_hz: 0.0,
        cw_pitch_hz: 600.0,
        filter_bandwidth_hz: 3000.0, // sensible default
        deemphasis_mode: default_deemphasis_mode(server_cfg.demod).unwrap_or(DeemphasisMode::Off),
        squelch_enabled: false,
        squelch_threshold_db: -90.0,
        squelch_open: true,
        nr2_enabled: false,
        nr2_strength: 0.5,
        agc_enabled: true,
        agc_strength: 0.5,
        signal_dbm: crate::dsp::smeter::MIN_DBM,
        signal_s_units: 0,
        volume_percent: 50,
        source_control: SourceControlState::default(),
        source_status: SourceStatus::default(),
        amplifier_status: AmplifierStatus::default(),
        pending_tx_tune_test: None,
        last_tx_tune_result: None,
        pending_swr_sweep: None,
        swr_sweep_cancel: false,
        last_swr_sweep_result: None,
        swr_sweep_progress: None,
        pending_tx_tone: None,
        tx_tone_stop: Arc::new(AtomicBool::new(false)),
        pending_cw_key: false,
        cw_key_held: Arc::new(AtomicBool::new(false)),
        cw_hang_ms: 300,
        pending_mic_tx: false,
        mic_tx_active: Arc::new(AtomicBool::new(false)),
        two_tone_enabled: false,
        two_tone_a_hz: 700.0,
        two_tone_b_hz: 1900.0,
        two_tone_level: 0.5,
        tx_limiter_enabled: true,
        tx_limiter_threshold: 0.9,
        compressor_enabled: false,
        compressor_level: 3,
        pending_iq_record_start: false,
        pending_iq_record_stop: false,
        iq_recording_status: IqRecordingStatus::default(),
    }));

    let (iq_audio_tx, iq_audio_rx) = std_mpsc::sync_channel::<Vec<Complex32>>(2);
    let (iq_wf_tx, iq_wf_rx) = std_mpsc::sync_channel::<Vec<Complex32>>(4);
    let (startup_info_tx, startup_info_rx) =
        std_mpsc::sync_channel::<Result<StartupInfo, String>>(1);
    let (capture_start_tx, capture_start_rx) = std_mpsc::sync_channel::<()>(1);
    let (fatal_tx, fatal_rx) = std_mpsc::channel::<WorkerExit>();

    // Written by capture thread (after hardware confirms the rate), read by DSP
    // thread. Using AtomicU32 avoids a mutex on the hot path.
    let confirmed_sample_rate_hz = Arc::new(AtomicU32::new(0));

    // Amplifier control channel + current-frequency mirror (for the HR50 poller,
    // which owns the serial port). The command thread sends control commands and
    // mirrors the tuned frequency here; the poller drains them.
    let (amp_cmd_tx, amp_cmd_rx) = std_mpsc::channel::<crate::amplifier::AmpCommand>();
    let amp_freq = Arc::new(AtomicU64::new(
        control.lock().map(|c| c.target_freq_hz).unwrap_or(0),
    ));

    // HL2 has a watchdog: if no C&C packets arrive in ~60 s it stops streaming.
    // The capture thread sends a keepalive C&C every 30 s to prevent this.
    let needs_cc_keepalive = matches!(source_kind, SourceKind::HermesLite2);

    let cmd_thread = spawn_command_thread(
        cmd_rx,
        control.clone(),
        stop_flag.clone(),
        stop_reason.clone(),
        fatal_tx.clone(),
        amp_cmd_tx,
        amp_freq.clone(),
    );

    let capture_thread = spawn_capture_thread(
        descriptor.clone(),
        server_cfg.clone(),
        control.clone(),
        stop_flag.clone(),
        stop_reason.clone(),
        fatal_tx.clone(),
        startup_info_tx,
        capture_start_rx,
        iq_audio_tx,
        iq_wf_tx,
        block_size,
        initial_center_freq_hz,
        confirmed_sample_rate_hz.clone(),
        needs_cc_keepalive,
    );

    let startup_info = match startup_info_rx.recv() {
        Ok(Ok(info)) => info,
        Ok(Err(reason)) => {
            let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
            let _ = cmd_thread.join();
            let _ = capture_thread.join();
            return WorkerExit::Failed { reason };
        }
        Err(_) => {
            let reason = "capture thread failed before startup".to_string();
            let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
            let _ = cmd_thread.join();
            let _ = capture_thread.join();
            return WorkerExit::Failed { reason };
        }
    };

    let _ = startup_tx.send(WorkerStartResult::Ready(WorkerReadyInfo {
        runtime: startup_info.runtime.clone(),
    }));

    let _ = status_tx.send(WorkerStatus::Running {
        runtime: startup_info.runtime.clone(),
    });

    let _ = capture_start_tx.send(());

    let dsp_thread = spawn_dsp_thread(
        descriptor.clone(),
        server_cfg.clone(),
        control.clone(),
        stop_flag.clone(),
        fatal_tx.clone(),
        status_tx.clone(),
        iq_audio_rx,
        request.audio_udp_peer,
        startup_info.clone(),
        confirmed_sample_rate_hz.clone(),
    );

    let waterfall_thread = spawn_waterfall_thread(
        descriptor.clone(),
        iq_wf_rx,
        stop_flag.clone(),
        fatal_tx.clone(),
        request.waterfall_udp_peer,
        startup_info.runtime.waterfall_bins as usize,
        startup_info.runtime.waterfall_frame_rate_hz,
    );

    // Amplifier (HR50) poller — HL2 only, when a serial device is configured.
    // Detects + polls status, tracks frequency (FA) and applies control commands;
    // updates `control.amplifier_status`, which the status path publishes.
    let amplifier_thread = if matches!(source_kind, SourceKind::HermesLite2) {
        spawn_amplifier_thread(
            descriptor.clone(),
            server_cfg.clone(),
            control.clone(),
            stop_flag.clone(),
            amp_cmd_rx,
            amp_freq.clone(),
        )
    } else {
        None
    };

    let exit = loop {
        if stop_requested(&stop_flag) {
            break WorkerExit::Clean {
                reason: stop_reason.lock().unwrap().clone(),
            };
        }

        match fatal_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(exit) => {
                stop_flag.store(true, Ordering::Relaxed);
                break exit;
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                break WorkerExit::Failed {
                    reason: "fatal error channel disconnected".to_string(),
                };
            }
        }
    };

    let _ = cmd_thread.join();
    let _ = capture_thread.join();
    let _ = dsp_thread.join();
    let _ = waterfall_thread.join();
    if let Some(h) = amplifier_thread {
        let _ = h.join();
    }

    match &exit {
        WorkerExit::Clean { reason } => {
            let _ = status_tx.send(WorkerStatus::Stopping {
                reason: reason.clone(),
            });
            let _ = status_tx.send(WorkerStatus::Stopped {
                reason: reason.clone(),
            });
        }
        WorkerExit::Failed { reason } => {
            let _ = status_tx.send(WorkerStatus::Faulted {
                reason: reason.clone(),
            });
        }
    }

    debug!(
        "[radio-worker {}] worker threads exiting with {:?}",
        descriptor.id.0, exit
    );

    exit
}

/// Parse an explicit `--hr50-serial` value of the form `<path>` or `<path>:baud`
/// into `(path, baud)`.  A missing/unparseable baud suffix falls back to
/// [`HR50_DEFAULT_BAUD`]; Linux device paths contain no colons.
fn parse_hr50_path_baud(value: &str) -> (String, u32) {
    if let Some((path, baud_str)) = value.rsplit_once(':') {
        if let Ok(baud) = baud_str.parse::<u32>() {
            return (path.to_string(), baud);
        }
    }
    (value.to_string(), HR50_DEFAULT_BAUD)
}

/// Publish an amplifier serial-open failure to the client.
///
/// Writes the reason into `amplifier_status.last_error` (and clears `model`) so
/// the existing runtime pipeline carries it to the client's on-screen Problems
/// area, instead of the failure being log-only.
fn publish_amplifier_open_error(control: &Arc<Mutex<SharedControlState>>, message: String) {
    if let Ok(mut cs) = control.lock() {
        cs.amplifier_status.model = None;
        cs.amplifier_status.last_error = Some(message);
    }
}

/// Spawn the amplifier status poller for an HL2 worker (Phase 1).
///
/// Returns `None` (no thread) when amplifier polling is disabled
/// (`--hr50-serial none`).  With the default `auto`, the thread runs VID/PID
/// narrowed probing ([`crate::amplifier::autodetect_serial`]) to find an HR50
/// and its baud; with an explicit path it opens that device directly.  If
/// nothing is found / can't be opened the thread exits and `amplifier_status`
/// stays at its default (`model: None`) so the UI shows "Amplifier: None".  The
/// poller writes `control.amplifier_status` on change; the existing status path
/// publishes it.
fn spawn_amplifier_thread(
    descriptor: RadioDescriptor,
    server_cfg: ServerConfig,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    amp_cmd_rx: std_mpsc::Receiver<crate::amplifier::AmpCommand>,
    amp_freq: Arc<AtomicU64>,
) -> Option<thread::JoinHandle<()>> {
    let configured = server_cfg.hr50_serial.clone()?;
    let radio_id = descriptor.id.0;

    Some(thread::spawn(move || {
        // Resolve the serial port. "auto" runs VID/PID-narrowed probing and also
        // discovers the baud; an explicit "<path>[:baud]" opens that device
        // directly at the given (or default 19200) baud — the user has taken
        // responsibility for it.
        let (path, baud) = if configured.eq_ignore_ascii_case("auto") {
            match crate::amplifier::autodetect_serial(HR50_DEFAULT_BAUD) {
                Some(found) => found,
                None => {
                    // Expected for any station without an HR50 (auto is the
                    // default), so this is log-only — not surfaced as a problem.
                    info!(
                        "[radio-worker {radio_id}] HR50 auto-detect: no amplifier responded on any \
                         USB-serial port; amplifier polling disabled"
                    );
                    return;
                }
            }
        } else {
            parse_hr50_path_baud(&configured)
        };

        let transport = match crate::amplifier::serial::SerialTransport::open(&path, baud) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("HR50 serial '{path}' open failed: {e}");
                warn!("[radio-worker {radio_id}] {msg}");
                publish_amplifier_open_error(&control, msg);
                return;
            }
        };
        info!("[radio-worker {radio_id}] HR50 amplifier polling on {path} @ {baud} 8N1");
        // The HR50 hangs if sent serial while keyed, so the poller goes silent
        // during TX.  Reuse the existing per-path keying flags (mic/FT8/two-tone
        // and CW) — both are reliably cleared on key-up and on worker stop.
        let tx_keyed = match control.lock() {
            Ok(cs) => vec![Arc::clone(&cs.mic_tx_active), Arc::clone(&cs.cw_key_held)],
            Err(_) => Vec::new(),
        };
        crate::amplifier::run_amplifier_poller(
            Box::new(transport),
            stop_flag,
            amp_cmd_rx,
            amp_freq,
            tx_keyed,
            |status| {
                if let Ok(mut cs) = control.lock() {
                    cs.amplifier_status = status.clone();
                }
            },
        );
        debug!("[radio-worker {radio_id}] HR50 amplifier poller stopped");
    }))
}

#[allow(clippy::too_many_arguments)]
fn spawn_command_thread(
    cmd_rx: std_mpsc::Receiver<WorkerCommand>,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    stop_reason: Arc<Mutex<StopReason>>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
    amp_cmd_tx: std_mpsc::Sender<crate::amplifier::AmpCommand>,
    amp_freq: Arc<AtomicU64>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop_requested(&stop_flag) {
            match cmd_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(cmd) => match cmd {
                    WorkerCommand::SetTargetFrequency { hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.target_freq_hz = hz;
                        }
                        // Mirror to the amplifier poller for FA band tracking.
                        amp_freq.store(hz, Ordering::Relaxed);
                    }
                    WorkerCommand::SetCenterFrequency { hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.center_freq_hz = hz;
                        }
                    }
                    WorkerCommand::SetDemodMode { mode } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.demod_mode = mode;
                        }
                    }
                    WorkerCommand::SetSideband { sideband } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.sideband = sideband;
                        }
                    }
                    WorkerCommand::SetPitch { pitch_hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            match control_state.demod_mode {
                                DemodMode::Usb | DemodMode::Lsb => {
                                    control_state.ssb_pitch_hz = pitch_hz.clamp(-1500.0, 1500.0);
                                }
                                DemodMode::Cwu | DemodMode::Cwl => {
                                    control_state.cw_pitch_hz = pitch_hz.clamp(300.0, 1200.0);
                                }
                                _ => {}
                            }
                        }
                    }
                    WorkerCommand::Stop { reason } => {
                        set_stop_reason(&stop_reason, reason);
                        stop_flag.store(true, Ordering::Relaxed);
                        // Also break any in-flight TX test tone / CW key so the
                        // capture thread (blocked in tx_test_tone / tx_cw_key)
                        // runs its fall + releases PTT promptly.
                        if let Ok(cs) = control.lock() {
                            cs.tx_tone_stop.store(true, Ordering::Relaxed);
                            cs.cw_key_held.store(false, Ordering::Relaxed);
                            cs.mic_tx_active.store(false, Ordering::Relaxed);
                        }
                        break;
                    }
                    WorkerCommand::SetFilterBandwidth { bandwidth_hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.filter_bandwidth_hz = bandwidth_hz.clamp(100.0, 20000.0);
                        }
                    }
                    WorkerCommand::SetSquelchEnabled { enabled } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.squelch_enabled = enabled;
                        }
                    }
                    WorkerCommand::SetSquelchThreshold { threshold_db } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.squelch_threshold_db = threshold_db.clamp(-120.0, 0.0);
                        }
                    }
                    WorkerCommand::SetNr2Enabled { enabled } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.nr2_enabled = enabled;
                        }
                    }
                    WorkerCommand::SetNr2Strength { strength } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.nr2_strength = strength.clamp(0.0, 1.0);
                        }
                    }
                    WorkerCommand::SetAgcEnabled { enabled } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.agc_enabled = enabled;
                        }
                    }
                    WorkerCommand::SetAgcStrength { strength } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.agc_strength = strength.clamp(0.0, 1.0);
                        }
                    }
                    WorkerCommand::SetVolume { volume_percent } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.volume_percent = volume_percent.min(100);
                        }
                    }
                    WorkerCommand::SetDeemphasisMode { mode } => {
                        if let Ok(mut control_state) = control.lock() {
                            //let before = control_state.deemphasis_mode;

                            control_state.deemphasis_mode = mode;
                            /*
                            info!(
                            "[worker] SetDeemphasisMode: {:?} -> {:?}",
                            before,
                            control_state.deemphasis_mode
                            );
                            */
                        }
                    }
                    WorkerCommand::SetSourceSampleRate { sample_rate_hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.sample_rate_hz = sample_rate_hz;
                        }
                    }

                    WorkerCommand::SetSourceGainMode { mode } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.gain_mode = mode;
                        }
                    }

                    WorkerCommand::SetSourceGain { gain_db } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.gain_db = gain_db;
                        }
                    }

                    WorkerCommand::SetSourcePpmCorrection { ppm } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.ppm_correction = ppm;
                        }
                    }

                    WorkerCommand::SetSourceDirectSampling { mode } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.direct_sampling = mode;
                        }
                    }

                    WorkerCommand::SetSourceTxDrive { tx_drive_percent } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.tx_drive_percent =
                                tx_drive_percent.clamp(0.0, 100.0);
                        }
                    }

                    WorkerCommand::SetSourceSpotLevel { spot_level_percent } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.spot_level_percent =
                                spot_level_percent.clamp(0.0, 100.0);
                        }
                    }

                    WorkerCommand::SetSourceTxSequencing { lead_ms, tail_ms } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.tx_ptt_lead_ms = lead_ms.min(100);
                            control_state.source_control.tx_ptt_tail_ms = tail_ms.min(100);
                        }
                    }

                    WorkerCommand::SetSourceN2adrEnabled { enabled } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.n2adr_enabled = enabled;
                        }
                    }

                    WorkerCommand::SetSourceFdxEnabled { enabled } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.source_control.fdx_enabled = enabled;
                        }
                    }

                    WorkerCommand::RequestTxTuneTest { duration_ms } => {
                        info!(
                            "[radio-worker] RequestTxTuneTest queued: duration_ms={}",
                            duration_ms
                        );
                        if let Ok(mut cs) = control.lock() {
                            // Replace any previous pending request; only one
                            // measurement runs at a time.
                            cs.pending_tx_tune_test = Some(duration_ms);
                        }
                    }

                    WorkerCommand::RequestSwrSweep { start_hz, stop_hz } => {
                        info!("[radio-worker] RequestSwrSweep queued: {start_hz}..{stop_hz} Hz");
                        if let Ok(mut cs) = control.lock() {
                            cs.pending_swr_sweep = Some((start_hz, stop_hz));
                            cs.swr_sweep_cancel = false;
                        }
                    }

                    WorkerCommand::CancelSwrSweep => {
                        if let Ok(mut cs) = control.lock() {
                            cs.swr_sweep_cancel = true;
                            cs.pending_swr_sweep = None;
                        }
                    }

                    WorkerCommand::StartTxTestTone { tone_hz, usb } => {
                        info!(
                            "[radio-worker] StartTxTestTone queued: tone={tone_hz} Hz mode={}",
                            if usb { "USB" } else { "LSB" }
                        );
                        if let Ok(mut cs) = control.lock() {
                            cs.tx_tone_stop.store(false, Ordering::Relaxed);
                            cs.pending_tx_tone = Some((tone_hz, usb));
                        }
                    }

                    WorkerCommand::StopTxTestTone => {
                        if let Ok(mut cs) = control.lock() {
                            // Signal a running tone to stop, and cancel any
                            // not-yet-started request.
                            cs.tx_tone_stop.store(true, Ordering::Relaxed);
                            cs.pending_tx_tone = None;
                        }
                    }

                    WorkerCommand::StartCwKey => {
                        if let Ok(mut cs) = control.lock() {
                            cs.cw_key_held.store(true, Ordering::Relaxed);
                            cs.pending_cw_key = true;
                        }
                    }

                    WorkerCommand::StopCwKey => {
                        if let Ok(mut cs) = control.lock() {
                            // Drop the key: a running session runs the fall
                            // envelope and (after the hang) releases PTT; a
                            // not-yet-started one is cancelled.
                            cs.cw_key_held.store(false, Ordering::Relaxed);
                            cs.pending_cw_key = false;
                        }
                    }

                    WorkerCommand::SetCwHangTime { hang_ms } => {
                        if let Ok(mut cs) = control.lock() {
                            cs.cw_hang_ms = hang_ms.min(2_000);
                        }
                    }

                    WorkerCommand::StartMicTx => {
                        // Drop any stale buffered mic audio from a prior over.
                        crate::net::udp::mic_audio::clear_mic_samples();
                        if let Ok(mut cs) = control.lock() {
                            cs.mic_tx_active.store(true, Ordering::Relaxed);
                            cs.pending_mic_tx = true;
                        }
                    }

                    WorkerCommand::StopMicTx => {
                        if let Ok(mut cs) = control.lock() {
                            cs.mic_tx_active.store(false, Ordering::Relaxed);
                            cs.pending_mic_tx = false;
                        }
                    }

                    WorkerCommand::ResetTxAudioDiag => {
                        crate::tx_diag::reset_counters();
                    }

                    WorkerCommand::SetTwoToneTest {
                        enabled,
                        tone_a_hz,
                        tone_b_hz,
                        level_percent,
                    } => {
                        if let Ok(mut cs) = control.lock() {
                            cs.two_tone_enabled = enabled;
                            cs.two_tone_a_hz = tone_a_hz.clamp(100.0, 4000.0);
                            cs.two_tone_b_hz = tone_b_hz.clamp(100.0, 4000.0);
                            cs.two_tone_level = (level_percent.clamp(0.0, 100.0)) / 100.0;
                        }
                    }

                    WorkerCommand::SetTxLimiter {
                        enabled,
                        threshold_percent,
                    } => {
                        if let Ok(mut cs) = control.lock() {
                            cs.tx_limiter_enabled = enabled;
                            cs.tx_limiter_threshold = threshold_percent.clamp(50.0, 99.0) / 100.0;
                        }
                    }

                    WorkerCommand::SetCompression { enabled, level } => {
                        let level = level.min(10);
                        if let Ok(mut cs) = control.lock() {
                            cs.compressor_enabled = enabled;
                            cs.compressor_level = level;
                        }
                        debug!("[radio-worker] compressor enabled={enabled} level={level}");
                    }

                    WorkerCommand::StartIqRecording => {
                        if let Ok(mut cs) = control.lock() {
                            cs.pending_iq_record_start = true;
                            cs.pending_iq_record_stop = false;
                        }
                    }

                    WorkerCommand::StopIqRecording => {
                        if let Ok(mut cs) = control.lock() {
                            cs.pending_iq_record_stop = true;
                            cs.pending_iq_record_start = false;
                        }
                    }

                    // Amplifier control — forwarded to the poller thread (which
                    // owns the serial port).  Harmlessly dropped if no poller.
                    WorkerCommand::SetAmplifierKeyingMode { mode } => {
                        let _ = amp_cmd_tx.send(crate::amplifier::AmpCommand::SetKeyingMode(mode));
                    }
                    WorkerCommand::SetAmplifierAtuMode { mode } => {
                        let _ = amp_cmd_tx.send(crate::amplifier::AmpCommand::SetAtuMode(mode));
                    }
                    WorkerCommand::TuneAmplifierAtu => {
                        let _ = amp_cmd_tx.send(crate::amplifier::AmpCommand::TuneAtu);
                    }
                },
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                    let reason = "worker command channel closed".to_string();
                    stop_flag.store(true, Ordering::Relaxed);
                    let _ = fatal_tx.send(WorkerExit::Failed { reason });
                    break;
                }
            }
        }
    })
}

fn spawn_waterfall_thread(
    descriptor: RadioDescriptor,
    iq_wf_rx: std_mpsc::Receiver<Vec<Complex32>>,
    stop_flag: Arc<AtomicBool>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
    wf_target: std::net::SocketAddr,
    waterfall_window_len: usize,
    waterfall_frame_rate_hz: f32,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut waterfall_gen = WaterfallGenerator::new(WATERFALL_BINS);

        let mut waterfall = match UdpWaterfallSender::new() {
            Ok(sender) => sender,
            Err(error) => {
                let reason = format!("failed to create UDP waterfall sender: {error}");
                stop_flag.store(true, Ordering::Relaxed);
                let _ = fatal_tx.send(WorkerExit::Failed { reason });
                return;
            }
        };

        let mut waterfall_iq_buffer: VecDeque<Complex32> = VecDeque::new();
        let waterfall_max_len = waterfall_window_len * 8;
        let waterfall_period =
            Duration::from_secs_f32((1.0 / waterfall_frame_rate_hz.max(1.0)).max(0.001));
        let mut next_waterfall_tick = Instant::now();

        loop {
            if stop_requested(&stop_flag) {
                break;
            }

            while let Ok(iq) = iq_wf_rx.try_recv() {
                for sample in iq {
                    waterfall_iq_buffer.push_back(sample);
                }

                while waterfall_iq_buffer.len() > waterfall_max_len {
                    waterfall_iq_buffer.pop_front();
                }
            }

            let now = Instant::now();

            if now >= next_waterfall_tick {
                if waterfall_iq_buffer.len() >= waterfall_window_len {
                    let start = waterfall_iq_buffer.len() - waterfall_window_len;
                    let mut fft_input = Vec::with_capacity(waterfall_window_len);

                    for sample in waterfall_iq_buffer.iter().skip(start) {
                        fft_input.push(*sample);
                    }

                    let row_db = waterfall_gen.generate_row_db(&fft_input);
                    if !row_db.is_empty() {
                        let min_db = row_db.iter().copied().fold(f32::INFINITY, f32::min);
                        let max_db = row_db.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                        let avg_db = row_db.iter().copied().sum::<f32>() / row_db.len() as f32;

                        trace!(
			    "[radio-worker {}] waterfall row: bins={} min={:.1} max={:.1} avg={:.1}",
			    descriptor.id.0,
			    row_db.len(),
			    min_db,
			    max_db,
			    avg_db
			);

                        waterfall.send_row_db_to(wf_target, &row_db);
                    }
                }

                next_waterfall_tick += waterfall_period;
            } else {
                thread::sleep((next_waterfall_tick - now).min(Duration::from_millis(5)));
            }
        }

        debug!(
            "[radio-worker {}] waterfall thread exiting",
            descriptor.id.0
        );
    })
}

/// FDX / TX Monitor Spectrum: forward RX IQ captured during a transmit into the
/// **waterfall** thread, which produces BOTH the spectrum and waterfall data the
/// client renders.  Deliberately not sent to the audio/DSP path, so transmit
/// audio behaviour is unchanged (this feature is visual monitoring only).
///
/// Non-blocking (`try_send`): never stalls the capture thread, so Spot/SWR
/// timing and the TX FIFO are unaffected.  Called after each `tx_tune_test`;
/// `iq` is empty unless FDX is enabled, in which case this is a no-op.
fn forward_fdx_iq(
    iq: Vec<Complex32>,
    iq_wf_tx: &std_mpsc::SyncSender<Vec<Complex32>>,
    radio_id: &str,
) {
    if iq.is_empty() {
        return;
    }
    let n = iq.len();
    match iq_wf_tx.try_send(iq) {
        Ok(()) => {
            debug!("[hl2 fdx] forwarded {n} RX IQ samples to spectrum/waterfall ({radio_id})")
        }
        Err(_) => {
            debug!("[hl2 fdx] waterfall queue full — dropped {n} FDX IQ samples ({radio_id})")
        }
    }
}

fn spawn_capture_thread(
    descriptor: RadioDescriptor,
    server_cfg: ServerConfig,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    stop_reason: Arc<Mutex<StopReason>>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
    startup_info_tx: std_mpsc::SyncSender<Result<StartupInfo, String>>,
    capture_start_rx: std_mpsc::Receiver<()>,
    iq_audio_tx: std_mpsc::SyncSender<Vec<Complex32>>,
    iq_wf_tx: std_mpsc::SyncSender<Vec<Complex32>>,
    block_size: usize,
    initial_center_freq_hz: u64,
    confirmed_sample_rate_hz: Arc<AtomicU32>,
    needs_cc_keepalive: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut source = match create_worker_source(
            &descriptor,
            &server_cfg,
            block_size,
            initial_center_freq_hz,
        ) {
            Ok(source) => source,
            Err(reason) => {
                let _ = startup_info_tx.send(Err(reason.clone()));
                let _ = fatal_tx.send(WorkerExit::Failed { reason });
                return;
            }
        };

        // This thread owns the IQ source, so source-level controls are applied here.
        let initial_source_control = source.source_control_state();

        if let Ok(mut control_state) = control.lock() {
            control_state.source_control = initial_source_control.clone();
        }

        let mut applied_source_control = initial_source_control;

        confirmed_sample_rate_hz.store(source.sample_rate() as u32, Ordering::Release);

        if let Err(reason) = source.set_center_frequency(initial_center_freq_hz as f32) {
            let _ = startup_info_tx.send(Err(reason.clone()));
            let _ = fatal_tx.send(WorkerExit::Failed { reason });
            return;
        }

        let startup_runtime = build_runtime_state(&current_control(&control), source.sample_rate());

        let _ = startup_info_tx.send(Ok(StartupInfo {
            input_sample_rate_hz: source.sample_rate(),
            runtime: startup_runtime,
        }));

        let _ = capture_start_rx.recv();

        let realtime = source.is_realtime();
        let mut source_block_period =
            Duration::from_secs_f32((block_size as f32 / source.sample_rate()).max(0.001));
        let mut next_source_tick = Instant::now();
        let mut last_center_freq_hz = initial_center_freq_hz;
        let mut blocks_read: u64 = 0;
        // Tracks a sustained RX gap on a realtime source so a brief link blip is
        // tolerated (worker stays up, surfaces "not responding") while a long
        // outage still fails the worker for a clean re-acquire.
        let mut rx_stall_start: Option<Instant> = None;
        let mut last_cc_sent = Instant::now();
        // N2ADR HF filter board (HL2): track the last band we programmed and the
        // enable state so we only reprogram on band change or (re)enable.
        let mut last_n2adr_band: Option<HamBand> = None;
        let mut last_n2adr_enabled = false;
        // FDX / TX Monitor Spectrum (HL2): mirror the enable state into the
        // source so it retains RX IQ during transmit.  Only pushed on change.
        let mut last_fdx_enabled = false;
        // TX PTT sequencing (HL2): mirror lead/tail delays into the source so all
        // transmit paths use them.  Pushed on change.  `u32::MAX` forces the
        // first push.
        let mut last_tx_lead_ms = u32::MAX;
        let mut last_tx_tail_ms = u32::MAX;
        // Receive IQ recording (Phase 1): the recorder is owned here (the
        // capture thread owns the IQ stream).  Start/stop are driven by control
        // flags; status is published periodically with the telemetry poll.
        let mut iq_rec: Option<crate::recording::iq_recorder::IqRecorder> = None;
        // Poll source_status() at ~4 Hz to avoid flooding SharedControlState.
        let status_poll_interval = Duration::from_millis(250);
        let mut last_status_poll = Instant::now();

        debug!(
            "[radio-worker {}] source running: sample_rate={} block_size={} realtime={}",
            descriptor.id.0,
            source.sample_rate(),
            block_size,
            realtime,
        );

        loop {
            if stop_requested(&stop_flag) {
                break;
            }

            let control_snapshot = current_control(&control);
            let current_source_control = control_snapshot.source_control.clone();

            if current_source_control.sample_rate_hz != applied_source_control.sample_rate_hz {
                if let Err(err) = source.set_sample_rate(current_source_control.sample_rate_hz) {
                    log::error!(
                        "[radio-worker {}] failed to set source sample rate to {} Hz: {}",
                        descriptor.id.0,
                        current_source_control.sample_rate_hz,
                        err
                    );
                } else {
                    applied_source_control.sample_rate_hz = current_source_control.sample_rate_hz;
                    source_block_period = Duration::from_secs_f32(
                        (block_size as f32 / source.sample_rate()).max(0.001),
                    );
                    next_source_tick = Instant::now();
                    // Signal the DSP thread that the hardware rate has changed.
                    confirmed_sample_rate_hz.store(source.sample_rate() as u32, Ordering::Release);

                    log::info!(
                        "[radio-worker {}] source sample rate set to {} Hz",
                        descriptor.id.0,
                        source.sample_rate(),
                    );
                }
            }

            if current_source_control.gain_mode != applied_source_control.gain_mode {
                match source.set_gain_mode(current_source_control.gain_mode) {
                    Ok(()) => {
                        applied_source_control.gain_mode = current_source_control.gain_mode;
                    }
                    Err(reason) => {
                        warn!("failed to set source gain mode: {reason}");
                    }
                }
            }

            if (current_source_control.gain_db - applied_source_control.gain_db).abs()
                > f32::EPSILON
            {
                match source.set_gain_db(current_source_control.gain_db) {
                    Ok(()) => {
                        applied_source_control.gain_db = current_source_control.gain_db;
                    }
                    Err(reason) => {
                        warn!("failed to set source gain: {reason}");
                    }
                }
            }

            if current_source_control.ppm_correction != applied_source_control.ppm_correction {
                match source.set_ppm_correction(current_source_control.ppm_correction) {
                    Ok(()) => {
                        applied_source_control.ppm_correction =
                            current_source_control.ppm_correction;
                    }
                    Err(reason) => {
                        warn!("failed to set source ppm correction: {reason}");
                    }
                }
            }

            if current_source_control.direct_sampling != applied_source_control.direct_sampling {
                let caps = source.source_capabilities();
                let sr = source.sample_rate();
                let new_mode = current_source_control.direct_sampling;
                let half_bw = (sr / 2.0) as u64;

                let (center_min, center_max) = if new_mode == DirectSamplingMode::Off {
                    (caps.tuner_freq_hz_min as u64, caps.tuner_freq_hz_max as u64)
                } else {
                    (0u64, caps.direct_sampling_freq_hz_max as u64)
                };

                let clamped_center = if center_max > 0 {
                    control_snapshot
                        .center_freq_hz
                        .clamp(center_min, center_max)
                } else {
                    control_snapshot.center_freq_hz
                };

                let clamped_target = if center_max > 0 {
                    let target_lo = clamped_center.saturating_sub(half_bw);
                    let target_hi = clamped_center.saturating_add(half_bw);
                    control_snapshot
                        .target_freq_hz
                        .clamp(target_lo, target_hi)
                        .clamp(center_min, center_max)
                } else {
                    control_snapshot.target_freq_hz
                };

                if let Ok(mut state) = control.lock() {
                    state.center_freq_hz = clamped_center;
                    state.target_freq_hz = clamped_target;
                }

                // Pre-load clamped center into the source so that set_direct_sampling(Off)'s
                // LO-restore targets the clamped value rather than a stale or out-of-range one.
                let _ = source.set_center_frequency(clamped_center as f32);

                match source.set_direct_sampling(new_mode) {
                    Ok(()) => {
                        applied_source_control.direct_sampling = new_mode;
                        last_center_freq_hz = clamped_center;
                    }
                    Err(reason) => {
                        warn!("failed to set source direct sampling: {reason}");
                    }
                }
            }

            if control_snapshot.center_freq_hz != last_center_freq_hz {
                if let Err(reason) =
                    source.set_center_frequency(control_snapshot.center_freq_hz as f32)
                {
                    stop_flag.store(true, Ordering::Relaxed);
                    let _ = fatal_tx.send(WorkerExit::Failed { reason });
                    break;
                }

                last_center_freq_hz = control_snapshot.center_freq_hz;
                last_cc_sent = Instant::now();
            } else if needs_cc_keepalive && last_cc_sent.elapsed() >= HL2_CC_KEEPALIVE_INTERVAL {
                source.keepalive();
                last_cc_sent = Instant::now();
            }

            // N2ADR HF filter board (HL2): when enabled, follow the tuned band.
            // Reprogram only when the band changes or N2ADR was just enabled.
            // Outside the supported bands the filter is left unchanged and no
            // update is sent (per spec).  No-op on sources without N2ADR.
            {
                let just_enabled = current_source_control.n2adr_enabled && !last_n2adr_enabled;
                if current_source_control.n2adr_enabled {
                    if let Some(band) = band_from_frequency(control_snapshot.target_freq_hz) {
                        if last_n2adr_band != Some(band) || just_enabled {
                            let value = n2adr_filter_value_for_band(band);
                            match source.set_n2adr_filter(value) {
                                Ok(()) => {
                                    info!(
                                        "[hl2] band={} freq={} mode={:?} n2adr={}",
                                        band.label(),
                                        control_snapshot.target_freq_hz,
                                        control_snapshot.demod_mode,
                                        value
                                    );
                                    last_n2adr_band = Some(band);
                                }
                                Err(e) => warn!("HL2: N2ADR filter program failed: {e}"),
                            }
                        }
                    }
                } else if last_n2adr_enabled {
                    // Just disabled: leave the hardware filter as-is; reset
                    // tracking so a later re-enable reprograms.
                    last_n2adr_band = None;
                }
                last_n2adr_enabled = current_source_control.n2adr_enabled;
            }

            // FDX / TX Monitor Spectrum (HL2): push the enable flag into the
            // source on change so it knows whether to retain RX IQ during the
            // next Spot/SWR.  No-op on sources that don't support FDX.
            if current_source_control.fdx_enabled != last_fdx_enabled {
                source.set_fdx_enabled(current_source_control.fdx_enabled);
                last_fdx_enabled = current_source_control.fdx_enabled;
            }

            // TX PTT sequencing delays → source (used by all transmit paths).
            if current_source_control.tx_ptt_lead_ms != last_tx_lead_ms
                || current_source_control.tx_ptt_tail_ms != last_tx_tail_ms
            {
                source.set_tx_sequencing(
                    current_source_control.tx_ptt_lead_ms,
                    current_source_control.tx_ptt_tail_ms,
                );
                last_tx_lead_ms = current_source_control.tx_ptt_lead_ms;
                last_tx_tail_ms = current_source_control.tx_ptt_tail_ms;
            }

            // Receive IQ recording: honour pending start/stop requests.  The
            // capture thread owns the recorder (and the IQ stream), so all
            // file lifecycle happens here; the background writer does the I/O.
            {
                let (want_start, want_stop) = control
                    .lock()
                    .map(|mut cs| {
                        let s = cs.pending_iq_record_start;
                        let e = cs.pending_iq_record_stop;
                        cs.pending_iq_record_start = false;
                        cs.pending_iq_record_stop = false;
                        (s, e)
                    })
                    .unwrap_or((false, false));

                if want_stop {
                    if let Some(rec) = iq_rec.take() {
                        rec.stop();
                    }
                }
                if want_start && iq_rec.is_none() {
                    let params = crate::recording::iq_recorder::RecordParams {
                        sample_rate_hz: source.sample_rate() as u32,
                        center_freq_hz: control_snapshot.center_freq_hz as u32,
                        gain_db: control_snapshot.source_control.gain_db,
                        ppm: control_snapshot.source_control.ppm_correction as f32,
                        source: format!("{:?}", descriptor.hardware_kind),
                    };
                    // Record into the server's recordings directory so the file
                    // is auto-discovered as a `wav:N` radio for playback.
                    let rec_dir = std::path::Path::new(&server_cfg.recordings_dir);
                    match crate::recording::iq_recorder::IqRecorder::start(rec_dir, params) {
                        Ok(rec) => iq_rec = Some(rec),
                        Err(e) => warn!(
                            "[radio-worker {}] IQ record start failed: {e}",
                            descriptor.id.0
                        ),
                    }
                }
            }

            // Poll hardware telemetry at ~4 Hz and propagate to SharedControlState
            // so the DSP thread can include it in WorkerStatus updates.  The IQ
            // recording status is refreshed on the same cadence.
            if last_status_poll.elapsed() >= status_poll_interval {
                let mut new_status = source.source_status();
                // Generic "device not responding": while a realtime source is in
                // a sustained RX stall (e.g. an RTL dongle pulled mid-stream, or
                // an HL2 link drop), surface it on screen regardless of whether
                // the source reports its own telemetry.
                if rx_stall_start.is_some() {
                    new_status.device_responding = Some(false);
                }
                let rec_status = match &iq_rec {
                    Some(rec) => IqRecordingStatus {
                        recording: true,
                        filename: Some(rec.filename().to_string()),
                        elapsed_secs: rec.elapsed_secs(),
                        file_size_bytes: rec.file_size_bytes(),
                        dropped_buffers: rec.dropped_buffers(),
                    },
                    None => IqRecordingStatus::default(),
                };
                if let Ok(mut cs) = control.lock() {
                    cs.source_status = new_status;
                    // Preserve the last filename/size when idle so the UI can
                    // show the most recent recording; only `recording` flips.
                    if rec_status.recording {
                        cs.iq_recording_status = rec_status;
                    } else if cs.iq_recording_status.recording {
                        cs.iq_recording_status.recording = false;
                    }
                }
                last_status_poll = Instant::now();
            }

            // Execute any pending TX tune test.
            //
            // The capture thread owns the IQ source exclusively, so
            // tx_tune_test() must run here.  The result is stored in
            // SharedControlState; the DSP thread detects the change and
            // publishes a RuntimeChanged message to the client.
            {
                let pending = control
                    .lock()
                    .ok()
                    .and_then(|mut cs| cs.pending_tx_tune_test.take());

                if let Some(duration_ms) = pending {
                    // Spot/SWR transmits on the operator's target frequency (not
                    // the RX DDC centre).  TX power is the configured source
                    // drive percent, read from control state at measure time.
                    let target_freq_hz = control_snapshot.target_freq_hz;
                    let tx_drive_percent = control_snapshot.source_control.tx_drive_percent;
                    let spot_level_percent = control_snapshot.source_control.spot_level_percent;
                    info!(
                        "[radio-worker {}] Spot/SWR: target_freq={} dur_ms={} tx_drive_percent={:.0} spot_level_percent={:.0}",
                        descriptor.id.0, target_freq_hz, duration_ms, tx_drive_percent, spot_level_percent
                    );

                    let result = source.tx_tune_test(
                        target_freq_hz,
                        duration_ms,
                        tx_drive_percent,
                        spot_level_percent,
                    );

                    info!(
                        "[radio-worker {}] TX tune test complete: result={:?}",
                        descriptor.id.0, result.message
                    );

                    // FDX / TX Monitor Spectrum: forward any RX IQ the source
                    // captured during the pulse into the spectrum/waterfall path
                    // (see `forward_fdx_iq`).  Empty unless FDX is enabled.
                    forward_fdx_iq(source.take_fdx_iq(), &iq_wf_tx, &descriptor.id.0);

                    if let Ok(mut cs) = control.lock() {
                        cs.last_tx_tune_result = Some(result);
                    }
                }
            }

            // Execute any pending SWR sweep (server-side, reusing tx_tune_test).
            // Runs the existing Spot/SWR at SWR_SWEEP_POINTS frequencies across a
            // single (validated) band, at the configured TX drive.  Progress is
            // written to control each point; a cancel flag stops it early.
            {
                let pending = control
                    .lock()
                    .ok()
                    .and_then(|mut cs| cs.pending_swr_sweep.take());

                if let Some((start_hz, stop_hz)) = pending {
                    let drive = control_snapshot.source_control.tx_drive_percent;
                    let spot_level = control_snapshot.source_control.spot_level_percent;
                    let n2adr_enabled = control_snapshot.source_control.n2adr_enabled;
                    let total = SWR_SWEEP_POINTS;

                    if let Err(reason) = validate_sweep_range(start_hz, stop_hz) {
                        warn!(
                            "[radio-worker {}] SWR sweep rejected: {reason}",
                            descriptor.id.0
                        );
                    } else {
                        info!(
                            "[radio-worker {}] SWR sweep: {start_hz}..{stop_hz} Hz, \
                             {total} points, drive={drive:.0}%",
                            descriptor.id.0
                        );

                        // Single validated band → program its N2ADR filter once.
                        if n2adr_enabled {
                            if let Some(band) = band_from_frequency(start_hz) {
                                let _ = source.set_n2adr_filter(n2adr_filter_value_for_band(band));
                                last_n2adr_band = Some(band);
                            }
                        }

                        if let Ok(mut cs) = control.lock() {
                            cs.swr_sweep_progress = Some(SwrSweepProgress {
                                running: true,
                                done: 0,
                                total,
                            });
                        }

                        let mut points: Vec<SwrSweepPoint> = Vec::with_capacity(total as usize);
                        let mut cancelled = false;
                        for i in 0..total {
                            if control
                                .lock()
                                .map(|cs| cs.swr_sweep_cancel)
                                .unwrap_or(false)
                            {
                                cancelled = true;
                                break;
                            }
                            let freq = sweep_frequency_hz(start_hz, stop_hz, i, total);
                            let r = source.tx_tune_test(freq, SWR_SWEEP_SPOT_MS, drive, spot_level);
                            // FDX: keep spectrum/waterfall alive between points.
                            forward_fdx_iq(source.take_fdx_iq(), &iq_wf_tx, &descriptor.id.0);
                            points.push(SwrSweepPoint {
                                frequency_hz: freq,
                                swr: r.swr,
                                forward_raw: r.forward_raw,
                                reverse_raw: r.reverse_raw,
                            });
                            if let Ok(mut cs) = control.lock() {
                                cs.swr_sweep_progress = Some(SwrSweepProgress {
                                    running: true,
                                    done: i + 1,
                                    total,
                                });
                            }
                        }

                        let done = points.len() as u32;
                        let result = SwrSweepResult {
                            start_hz,
                            stop_hz,
                            points,
                        };
                        info!(
                            "[radio-worker {}] SWR sweep {}: {done}/{total} points",
                            descriptor.id.0,
                            if cancelled { "cancelled" } else { "complete" }
                        );
                        if let Ok(mut cs) = control.lock() {
                            cs.last_swr_sweep_result = Some(result);
                            cs.swr_sweep_progress = Some(SwrSweepProgress {
                                running: false,
                                done,
                                total,
                            });
                            cs.swr_sweep_cancel = false;
                        }
                    }
                }
            }

            // Execute any pending TX test tone (FDX Phase 2).  Open-ended: the
            // source transmits the SSB tone until `tx_tone_stop` is set by a
            // StopTxTestTone command.  RX IQ decoded during the tone is pushed
            // straight to the waterfall/spectrum path (FDX) each iteration via
            // the `forward` closure, so the tone is visible while it runs.  Audio
            // is untouched (we never feed iq_audio_tx here).
            {
                let pending = control
                    .lock()
                    .ok()
                    .and_then(|mut cs| cs.pending_tx_tone.take());

                if let Some((tone_hz, usb)) = pending {
                    let tx_drive_percent = control_snapshot.source_control.tx_drive_percent;
                    let spot_level_percent = control_snapshot.source_control.spot_level_percent;
                    let target_freq_hz = control_snapshot.target_freq_hz;
                    let stop = control.lock().ok().map(|cs| cs.tx_tone_stop.clone());

                    if let Some(stop) = stop {
                        let mut forward = |iq: Vec<Complex32>| {
                            let _ = iq_wf_tx.try_send(iq);
                        };
                        if let Err(e) = source.tx_test_tone(
                            target_freq_hz,
                            tone_hz,
                            usb,
                            tx_drive_percent,
                            spot_level_percent,
                            &stop,
                            &mut forward,
                        ) {
                            warn!("[radio-worker {}] TX test tone error: {e}", descriptor.id.0);
                        }
                    }
                }
            }

            // Execute any pending CW key (CW TX Phase 1).  Open-ended: the source
            // keys the CW carrier (rise → sustain) until `cw_key_held` clears on
            // key-up, then runs the fall envelope and releases PTT.  Validates CW
            // mode here; sideband selects USB/LSB placement.  Forwards RX IQ to
            // the waterfall/spectrum path (FDX) so the CW tone is visible.
            {
                let pending = control
                    .lock()
                    .ok()
                    .map(|mut cs| {
                        let p = cs.pending_cw_key;
                        cs.pending_cw_key = false;
                        p
                    })
                    .unwrap_or(false);

                if pending {
                    let cw_usb = match control_snapshot.demod_mode {
                        DemodMode::Cwu => Some(true),
                        DemodMode::Cwl => Some(false),
                        _ => None,
                    };
                    if cw_usb.is_none() {
                        warn!(
                            "[radio-worker {}] CW key ignored: mode is {:?}, not CWU/CWL",
                            descriptor.id.0, control_snapshot.demod_mode
                        );
                        // Clear the held flag so a stale key doesn't linger.
                        if let Ok(cs) = control.lock() {
                            cs.cw_key_held.store(false, Ordering::Relaxed);
                        }
                    } else {
                        let tx_drive_percent = control_snapshot.source_control.tx_drive_percent;
                        let spot_level_percent = control_snapshot.source_control.spot_level_percent;
                        // CW transmit pitch = the operator's CW pitch from Radio
                        // Control (the same value used for RX CW demod), so TX and
                        // RX share one pitch.
                        let pitch_hz = control_snapshot.cw_pitch_hz;
                        // CW TX side comes from the MODE (CWU=above, CWL=below),
                        // never the generic SSB sideband field.
                        let usb = cw_usb.unwrap();
                        let target_freq_hz = control_snapshot.target_freq_hz;
                        // Semi break-in hang time (PTT persists between elements).
                        let hang_ms = control_snapshot.cw_hang_ms;
                        let held = control.lock().ok().map(|cs| cs.cw_key_held.clone());

                        if let Some(held) = held {
                            let mut forward = |iq: Vec<Complex32>| {
                                let _ = iq_wf_tx.try_send(iq);
                            };
                            // The worker stop_flag aborts a session (clean fall +
                            // release, no hang wait) on shutdown.
                            if let Err(e) = source.tx_cw_key(
                                target_freq_hz,
                                pitch_hz,
                                usb,
                                tx_drive_percent,
                                spot_level_percent,
                                hang_ms,
                                &held,
                                &stop_flag,
                                &mut forward,
                            ) {
                                warn!("[radio-worker {}] CW key error: {e}", descriptor.id.0);
                            }
                            // Re-sync the start signal to the live key state: if a
                            // key-down arrived right as the session ended, start a
                            // fresh session next iteration; otherwise clear it.
                            if let Ok(mut cs) = control.lock() {
                                cs.pending_cw_key = cs.cw_key_held.load(Ordering::Relaxed);
                            }
                        }
                    }
                }
            }

            // Execute any pending SSB mic transmit (Phase 3).  Open-ended: the
            // source modulates pulled mic audio (USB above carrier / LSB below)
            // until `mic_tx_active` clears on key-up.  Sideband comes from the
            // current mode.  RX IQ is forwarded to FDX; audio is pulled from the
            // global mic queue fed by the UDP listener.
            {
                let pending = control
                    .lock()
                    .ok()
                    .map(|mut cs| {
                        let p = cs.pending_mic_tx;
                        cs.pending_mic_tx = false;
                        p
                    })
                    .unwrap_or(false);

                if pending {
                    let mic_usb = match control_snapshot.demod_mode {
                        // DgtU (data-USB) keys on the USB sideband — WSJT-X/FT8
                        // sets this mode when its Radio "Mode" is Data/Pkt.  The
                        // RX pipeline already treats DgtU as USB everywhere; the
                        // TX gate must match or digital TX is silently dropped.
                        DemodMode::Usb | DemodMode::DgtU => Some(true),
                        DemodMode::Lsb => Some(false),
                        _ => None,
                    };
                    if mic_usb.is_none() {
                        warn!(
                            "[radio-worker {}] mic TX ignored: mode is {:?}, not USB/LSB",
                            descriptor.id.0, control_snapshot.demod_mode
                        );
                        if let Ok(cs) = control.lock() {
                            cs.mic_tx_active.store(false, Ordering::Relaxed);
                        }
                    } else {
                        let usb = mic_usb.unwrap();
                        let tx_drive_percent = control_snapshot.source_control.tx_drive_percent;
                        let target_freq_hz = control_snapshot.target_freq_hz;
                        let active = control.lock().ok().map(|cs| cs.mic_tx_active.clone());

                        if let Some(active) = active {
                            let mut forward = |iq: Vec<Complex32>| {
                                let _ = iq_wf_tx.try_send(iq);
                            };
                            // Two-tone test: when enabled, the same TX path is
                            // fed generated tones instead of mic audio, so it
                            // runs through the identical DcBlocker → SSB FIR →
                            // diagnostics chain.  Phase accumulators persist
                            // across pulls for a glitch-free continuous tone.
                            let two_tone = control_snapshot.two_tone_enabled;
                            let tt_amp = control_snapshot.two_tone_level * 0.5;
                            let tt_inc_a =
                                std::f32::consts::TAU * control_snapshot.two_tone_a_hz / 48_000.0;
                            let tt_inc_b =
                                std::f32::consts::TAU * control_snapshot.two_tone_b_hz / 48_000.0;
                            let mut tt_phase_a = 0.0f32;
                            let mut tt_phase_b = 0.0f32;
                            // Pull up to `n` samples: two-tone generator if
                            // enabled, else the mic queue (padded with silence
                            // on underrun by the source).
                            let mut pull = |n: usize, out: &mut Vec<f32>| {
                                if two_tone {
                                    for _ in 0..n {
                                        out.push(tt_amp * (tt_phase_a.sin() + tt_phase_b.sin()));
                                        tt_phase_a += tt_inc_a;
                                        if tt_phase_a >= std::f32::consts::TAU {
                                            tt_phase_a -= std::f32::consts::TAU;
                                        }
                                        tt_phase_b += tt_inc_b;
                                        if tt_phase_b >= std::f32::consts::TAU {
                                            tt_phase_b -= std::f32::consts::TAU;
                                        }
                                    }
                                    n
                                } else {
                                    crate::net::udp::mic_audio::drain_mic_samples(out, n)
                                }
                            };
                            if let Err(e) = source.tx_ssb_mic(
                                target_freq_hz,
                                usb,
                                tx_drive_percent,
                                control_snapshot.tx_limiter_enabled,
                                control_snapshot.tx_limiter_threshold,
                                control_snapshot.compressor_enabled,
                                control_snapshot.compressor_level,
                                &active,
                                &stop_flag,
                                &mut pull,
                                &mut forward,
                            ) {
                                warn!("[radio-worker {}] mic TX error: {e}", descriptor.id.0);
                            }
                        }
                    }
                }
            }

            if !realtime {
                let now = Instant::now();
                if now < next_source_tick {
                    thread::sleep(next_source_tick - now);
                }
                next_source_tick += source_block_period;
            }

            let iq = match source.read_block(block_size) {
                Ok(samples) => {
                    if let Some(since) = rx_stall_start.take() {
                        info!(
                            "[radio-worker {}] RX recovered after {:?}",
                            descriptor.id.0,
                            since.elapsed()
                        );
                    }
                    samples
                }
                // Realtime sources (HL2): tolerate a transient RX gap. Keep the
                // worker alive so the ~4 Hz status poll surfaces "not responding"
                // and RX can resume in place; fail only after a sustained outage
                // so the client's re-acquire re-initializes a power-cycled device.
                Err(reason) if realtime => {
                    let since = *rx_stall_start.get_or_insert_with(|| {
                        warn!(
                            "[radio-worker {}] {reason}; tolerating up to {:?}",
                            descriptor.id.0, RX_STALL_GIVE_UP
                        );
                        Instant::now()
                    });
                    if since.elapsed() >= RX_STALL_GIVE_UP {
                        stop_flag.store(true, Ordering::Relaxed);
                        let _ = fatal_tx.send(WorkerExit::Failed { reason });
                        break;
                    }
                    // Bound the spin if a realtime source errors instantly rather
                    // than blocking out its receive timeout.
                    thread::sleep(Duration::from_millis(200));
                    continue;
                }
                Err(reason) => {
                    stop_flag.store(true, Ordering::Relaxed);
                    let _ = fatal_tx.send(WorkerExit::Failed { reason });
                    break;
                }
            };

            if iq.is_empty() {
                if realtime {
                    continue;
                } else {
                    set_stop_reason(&stop_reason, StopReason::UserRequested);
                    stop_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }

            blocks_read += 1;

            // IQ recording tap: capture the raw source IQ before any DSP.
            // Non-blocking — a full writer queue drops (and counts) the block.
            if let Some(rec) = &iq_rec {
                rec.record_block(&iq);
            }

            if blocks_read % 20 == 0 {
                trace!(
                    "[radio-worker {}] capture alive: blocks={} iq_samples={} center={} target={}",
                    descriptor.id.0,
                    blocks_read,
                    iq.len(),
                    control_snapshot.center_freq_hz,
                    control_snapshot.target_freq_hz,
                );
            }

            if iq_audio_tx.send(iq.clone()).is_err() {
                stop_flag.store(true, Ordering::Relaxed);
                break;
            }

            match iq_wf_tx.try_send(iq) {
                Ok(_) => {}
                Err(std_mpsc::TrySendError::Full(_)) => {}
                Err(std_mpsc::TrySendError::Disconnected(_)) => {
                    stop_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }

        // Finalize any in-progress recording on capture-thread exit so the WAV
        // header is always valid even if the worker stops mid-recording.
        if let Some(rec) = iq_rec.take() {
            rec.stop();
        }
    })
}

fn spawn_dsp_thread(
    descriptor: RadioDescriptor,
    server_cfg: ServerConfig,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
    status_tx: watch::Sender<WorkerStatus>,
    iq_audio_rx: std_mpsc::Receiver<Vec<Complex32>>,
    audio_target: std::net::SocketAddr,
    startup_info: StartupInfo,
    confirmed_sample_rate_hz: Arc<AtomicU32>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pipeline_sample_rate_hz = startup_info.input_sample_rate_hz;
        let mut pipeline = DspPipeline::new(pipeline_cfg_for_source(
            &server_cfg,
            startup_info.runtime.center_freq_hz,
            startup_info.runtime.target_freq_hz,
            pipeline_sample_rate_hz,
        ));

        pipeline.set_filter_bandwidth_hz(startup_info.runtime.filter_bandwidth_hz);
        pipeline.set_agc_enabled(startup_info.runtime.agc_enabled);
        pipeline.set_agc_strength(startup_info.runtime.agc_strength);

        if matches!(
            startup_info.runtime.demod_mode,
            DemodMode::Usb | DemodMode::Lsb
        ) {
            pipeline.set_sideband(startup_info.runtime.sideband);
            pipeline.set_ssb_pitch_hz(startup_info.runtime.ssb_pitch_hz);
        }

        info!(
            "[radio-worker {}] pipeline mode={:?} input_sr={} output_sr={} client_sr={}",
            descriptor.id.0,
            startup_info.runtime.demod_mode,
            startup_info.input_sample_rate_hz,
            pipeline.output_sample_rate(),
            pipeline.client_output_sample_rate(),
        );

        let mut audio = match UdpAudioSender::new(240) {
            Ok(sender) => sender,
            Err(error) => {
                let reason = format!("failed to create UDP audio sender: {error}");
                stop_flag.store(true, Ordering::Relaxed);
                let _ = fatal_tx.send(WorkerExit::Failed { reason });
                return;
            }
        };

        let mut applied = current_control(&control);
        // TX-audio diagnostics live in the process-global (written by the
        // mic-TX loop), not in SharedControlState; track the last-published
        // snapshot here so the client meters update during an over.
        let mut applied_tx_diag = crate::tx_diag::snapshot();

        // Receive-squelch gate state, owned by this (DSP) thread.  3 dB of
        // hysteresis between the open and close thresholds prevents chatter.
        const SQUELCH_HYSTERESIS_DB: f32 = 3.0;
        let mut squelch_open = true;
        let mut last_squelch_log = Instant::now();
        let mut last_agc_log = Instant::now();

        // S-meter: smoothed channel power (linear), with fast attack / slow
        // release, published (throttled) as signal_dbm / signal_s_units.
        let mut smoothed_channel_power: f32 = 0.0;
        let mut last_smeter_update = Instant::now();

        // NR2 spectral noise-reduction processor, owned by this (DSP) thread.
        // Only engaged while enabled; reset on demod-mode / sample-rate change.
        let mut nr2 = Nr2::new();

        loop {
            if stop_requested(&stop_flag) {
                break;
            }

            let current = current_control(&control);

            let mut changed = false;

            let confirmed_rate = confirmed_sample_rate_hz.load(Ordering::Acquire);
            if confirmed_rate != pipeline_sample_rate_hz as u32 {
                pipeline_sample_rate_hz = confirmed_rate as f32;
                let cfg = ServerConfig {
                    demod: current.demod_mode,
                    ..server_cfg.clone()
                };
                pipeline = DspPipeline::new(pipeline_cfg_for_source(
                    &cfg,
                    current.center_freq_hz,
                    current.target_freq_hz,
                    pipeline_sample_rate_hz,
                ));
                pipeline.set_filter_bandwidth_hz(current.filter_bandwidth_hz);
                pipeline.set_agc_enabled(current.agc_enabled);
                pipeline.set_agc_strength(current.agc_strength);
                match current.demod_mode {
                    DemodMode::Usb | DemodMode::Lsb => {
                        pipeline.set_sideband(current.sideband);
                        pipeline.set_ssb_pitch_hz(current.ssb_pitch_hz);
                    }
                    DemodMode::Cwu | DemodMode::Cwl => {
                        pipeline.set_cw_pitch_hz(current.cw_pitch_hz);
                    }
                    _ => {}
                }
                pipeline.set_deemphasis_mode(current.deemphasis_mode);
                // Drain IQ blocks that were captured at the old sample rate.
                while iq_audio_rx.try_recv().is_ok() {}
                // Audio config changed — clear NR2 and S-meter state.
                nr2.reset();
                smoothed_channel_power = 0.0;
                changed = true;
            }

            if current.source_control != applied.source_control {
                applied.source_control = current.source_control.clone();
                changed = true;
            }

            if current.source_status != applied.source_status {
                applied.source_status = current.source_status.clone();
                changed = true;
            }

            if current.iq_recording_status != applied.iq_recording_status {
                applied.iq_recording_status = current.iq_recording_status.clone();
                changed = true;
            }

            // TX audio diagnostics: republish whenever the live meters or
            // counters move (only changes while keyed / on overrun).
            let cur_tx_diag = crate::tx_diag::snapshot();
            if cur_tx_diag != applied_tx_diag {
                applied_tx_diag = cur_tx_diag;
                changed = true;
            }

            if current.last_tx_tune_result != applied.last_tx_tune_result {
                applied.last_tx_tune_result = current.last_tx_tune_result.clone();
                changed = true;
            }

            if current.last_swr_sweep_result != applied.last_swr_sweep_result
                || current.swr_sweep_progress != applied.swr_sweep_progress
            {
                applied.last_swr_sweep_result = current.last_swr_sweep_result.clone();
                applied.swr_sweep_progress = current.swr_sweep_progress;
                changed = true;
            }

            // Squelch is applied in the audio block (no pipeline change), but a
            // settings or gate-state change still republishes runtime so the
            // client stays in sync.  The live `squelch_open` is written to
            // control by the audio block and observed here on the next pass.
            if current.squelch_enabled != applied.squelch_enabled
                || (current.squelch_threshold_db - applied.squelch_threshold_db).abs()
                    > f32::EPSILON
                || current.squelch_open != applied.squelch_open
            {
                changed = true;
            }

            // NR2 toggle/strength: no pipeline change, just republish for sync.
            if current.nr2_enabled != applied.nr2_enabled
                || (current.nr2_strength - applied.nr2_strength).abs() > f32::EPSILON
            {
                changed = true;
            }

            // AGC enable/strength → apply to the in-pipeline AGC stage.
            if current.agc_enabled != applied.agc_enabled
                || (current.agc_strength - applied.agc_strength).abs() > f32::EPSILON
            {
                pipeline.set_agc_enabled(current.agc_enabled);
                pipeline.set_agc_strength(current.agc_strength);
                changed = true;
            }

            // S-meter status update → republish (read-only; ~10 Hz while it
            // moves, since the DSP thread refreshes it every 100 ms).
            if (current.signal_dbm - applied.signal_dbm).abs() > 0.1
                || current.signal_s_units != applied.signal_s_units
            {
                changed = true;
            }

            // Volume change → republish for client sync (gain applied below).
            if current.volume_percent != applied.volume_percent {
                changed = true;
            }

            if current.center_freq_hz != applied.center_freq_hz {
                pipeline.set_center_frequency(current.center_freq_hz as f32);
                changed = true;
            }

            if current.target_freq_hz != applied.target_freq_hz {
                pipeline.set_target_frequency(current.target_freq_hz as f32);
                changed = true;
            }

            /*
            info!(
            "[worker] apply check: current={:?} applied={:?}",
            current.deemphasis_mode,
            applied.deemphasis_mode
            );
            */
            if current.deemphasis_mode != applied.deemphasis_mode {
                /*
                info!(
                    "[worker] applying deemphasis change: {:?} -> {:?}",
                    applied.deemphasis_mode,
                    current.deemphasis_mode
                    );
                */
                pipeline.set_deemphasis_mode(current.deemphasis_mode);
                changed = true;
            }

            if current.demod_mode != applied.demod_mode {
                let cfg = ServerConfig {
                    demod: current.demod_mode,
                    ..server_cfg.clone()
                };

                pipeline = DspPipeline::new(pipeline_cfg_for_source(
                    &cfg,
                    current.center_freq_hz,
                    current.target_freq_hz,
                    pipeline_sample_rate_hz,
                ));

                pipeline.set_filter_bandwidth_hz(current.filter_bandwidth_hz);
                pipeline.set_agc_enabled(current.agc_enabled);
                pipeline.set_agc_strength(current.agc_strength);

                match current.demod_mode {
                    DemodMode::Usb | DemodMode::Lsb => {
                        pipeline.set_sideband(current.sideband);
                        pipeline.set_ssb_pitch_hz(current.ssb_pitch_hz);
                    }
                    DemodMode::Cwu | DemodMode::Cwl => {
                        pipeline.set_cw_pitch_hz(current.cw_pitch_hz);
                    }
                    _ => {}
                }

                // Reapply deemphasis because its effective behavior depends on demod mode.
                pipeline.set_deemphasis_mode(current.deemphasis_mode);

                // Demod mode changed — clear NR2 and S-meter state.
                nr2.reset();
                smoothed_channel_power = 0.0;

                //		if changed {
                //		    applied = current.clone();
                //		}
                changed = true;
            } else {
                if current.sideband != applied.sideband {
                    pipeline.set_sideband(current.sideband);
                    applied.sideband = current.sideband;
                    changed = true;
                }

                match current.demod_mode {
                    DemodMode::Usb | DemodMode::Lsb => {
                        if (current.ssb_pitch_hz - applied.ssb_pitch_hz).abs() > f32::EPSILON {
                            pipeline.set_ssb_pitch_hz(current.ssb_pitch_hz);
                            applied.ssb_pitch_hz = current.ssb_pitch_hz;
                            changed = true;
                        }
                    }
                    DemodMode::Cwu | DemodMode::Cwl => {
                        if (current.cw_pitch_hz - applied.cw_pitch_hz).abs() > f32::EPSILON {
                            pipeline.set_cw_pitch_hz(current.cw_pitch_hz);
                            applied.cw_pitch_hz = current.cw_pitch_hz;
                            changed = true;
                        }
                    }
                    _ => {}
                }

                if (current.filter_bandwidth_hz - applied.filter_bandwidth_hz).abs() > 1.0 {
                    pipeline.set_filter_bandwidth_hz(current.filter_bandwidth_hz);
                    applied.filter_bandwidth_hz = current.filter_bandwidth_hz;
                    changed = true;
                }
            }

            if changed {
                applied = current.clone();
                let runtime = build_runtime_state(&current, pipeline_sample_rate_hz);
                let _ = status_tx.send(WorkerStatus::Running { runtime });
            }

            match iq_audio_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(iq) => {
                    let mut audio_f32 = pipeline.process_audio(&iq);

                    // ── NR2 spectral noise reduction ─────────────────────────
                    // Post-demod / post-audio-filter, before squelch.  When
                    // disabled, audio passes through untouched (NR2 not called);
                    // when first disabled, drop any accumulated state so a later
                    // re-enable starts fresh.
                    // ── S-meter ──────────────────────────────────────────────
                    // Channel power was measured pre-demod inside process_audio.
                    // Smooth it (fast attack / slow release) and publish dBm +
                    // S-units (throttled).  Independent of demod/AGC/NR2/squelch.
                    {
                        let raw_power = pipeline.last_channel_power();
                        let block_secs = (audio_f32.len() as f32 / 48_000.0).max(1e-4);
                        let attack_a = 1.0 - (-block_secs / 0.1).exp(); // ~100 ms
                        let release_a = 1.0 - (-block_secs / 0.5).exp(); // ~500 ms
                        let a = if raw_power > smoothed_channel_power {
                            attack_a
                        } else {
                            release_a
                        };
                        smoothed_channel_power += a * (raw_power - smoothed_channel_power);

                        if last_smeter_update.elapsed() >= Duration::from_millis(100) {
                            let dbm =
                                crate::dsp::smeter::channel_power_to_dbm(smoothed_channel_power);
                            let s = crate::dsp::smeter::dbm_to_s_units(dbm);
                            if let Ok(mut cs) = control.lock() {
                                cs.signal_dbm = dbm;
                                cs.signal_s_units = s;
                            }
                            debug!(
                                "[smeter] signal_power={:.3e} dbm={:.1} s_units={}",
                                smoothed_channel_power, dbm, s
                            );
                            last_smeter_update = Instant::now();
                        }
                    }

                    // AGC runs inside process_audio (post-demod, before NR2);
                    // log its state at ~1 Hz for diagnostics.
                    if last_agc_log.elapsed() >= Duration::from_millis(1000) {
                        debug!(
                            "[agc] agc_enabled={} agc_strength={:.2} envelope={:.4} gain={:.2}",
                            current.agc_enabled,
                            current.agc_strength,
                            pipeline.agc_envelope(),
                            pipeline.agc_current_gain(),
                        );
                        last_agc_log = Instant::now();
                    }

                    if current.nr2_enabled {
                        nr2.set_strength(current.nr2_strength);
                        audio_f32 = nr2.process(&audio_f32);
                    } else if nr2.is_active() {
                        nr2.reset();
                    }

                    // ── Receive squelch ──────────────────────────────────────
                    // Measure this block's RMS level in dBFS (ref = full-scale
                    // 1.0), apply hysteresis (open at threshold, close 3 dB
                    // below), and mute the audio while the gate is closed.  A
                    // packet is still emitted every time (silence when muted) so
                    // audio timing stays stable; waterfall/spectrum are unaffected.
                    if !audio_f32.is_empty() {
                        let sum_sq: f32 = audio_f32.iter().map(|s| s * s).sum();
                        let rms = (sum_sq / audio_f32.len() as f32).sqrt();
                        let level_db = 20.0 * rms.max(1e-9).log10();

                        if current.squelch_enabled {
                            let open_thr = current.squelch_threshold_db;
                            let close_thr = open_thr - SQUELCH_HYSTERESIS_DB;
                            if squelch_open {
                                if level_db < close_thr {
                                    squelch_open = false;
                                }
                            } else if level_db >= open_thr {
                                squelch_open = true;
                            }
                        } else {
                            // Disabled → always pass; never mute.
                            squelch_open = true;
                        }

                        // Mirror the gate state into control so the next loop's
                        // change-detection republishes runtime to the client.
                        if current.squelch_open != squelch_open {
                            if let Ok(mut cs) = control.lock() {
                                cs.squelch_open = squelch_open;
                            }
                        }

                        if current.squelch_enabled && !squelch_open {
                            audio_f32.iter_mut().for_each(|s| *s = 0.0);
                        }

                        if last_squelch_log.elapsed() >= Duration::from_millis(1000) {
                            debug!(
                                "[squelch] level={level_db:.1} dBFS open={squelch_open} \
                                 enabled={} thr={:.1} dB",
                                current.squelch_enabled, current.squelch_threshold_db
                            );
                            last_squelch_log = Instant::now();
                        }
                    }

                    // ── Soft limiter (safety) ────────────────────────────────
                    // Receive **Volume is now applied on the client** (so the
                    // Digital Audio Interface RX tap stays at fixed unity gain,
                    // independent of speaker volume).  The server streams the
                    // unity-level audio; we still soft-limit here so any demod
                    // overshoot can't hard-clip at the i16 conversion below.
                    for s in audio_f32.iter_mut() {
                        *s = soft_clip(*s);
                    }

                    let mut audio_i16 = Vec::with_capacity(audio_f32.len());
                    for sample in audio_f32 {
                        let value = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        audio_i16.push(value);
                    }

                    if !audio_i16.is_empty() {
                        audio.send_audio_to(audio_target, &audio_i16);
                    }
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                    stop_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    })
}

fn normalize_initial_frequencies(
    request: &AcquireRequest,
    server_cfg: &ServerConfig,
    src_kind: HardwareKind,
    wav_center_freq_hz: Option<u64>,
) -> (u64, u64) {
    debug!("request = {:?}", request);
    debug!("server_cfg = {:?}", server_cfg);
    debug!("wav_center_freq_hz = {:?}", wav_center_freq_hz);

    let mut initial_center_freq_hz = server_cfg.center_freq_hz;

    if src_kind == HardwareKind::FakeTone {
        debug!("In FakeTone");
        initial_center_freq_hz = server_cfg.fake_center_freq_hz;
    }

    if src_kind == HardwareKind::WavFile {
        debug!("In WavFile");
        initial_center_freq_hz = if let Some(wav_center) = wav_center_freq_hz {
            wav_center as f32
        } else if request.center_freq_hz != 0 {
            request.center_freq_hz as f32
        } else {
            server_cfg.center_freq_hz as f32
        };
    }

    if src_kind == HardwareKind::HermesLite2 {
        let caps = crate::source::hermeslite2::hl2_source_capabilities();
        let min = caps.tuner_freq_hz_min as f32;
        let max = caps.tuner_freq_hz_max as f32;
        let clamped = initial_center_freq_hz.clamp(min, max);
        if (clamped - initial_center_freq_hz).abs() > 1.0 {
            warn!(
                "HL2: initial center {:.0} Hz is outside hardware range \
                 [{:.0}, {:.0}] Hz — clamping to {:.0} Hz",
                initial_center_freq_hz, min, max, clamped
            );
        }
        initial_center_freq_hz = clamped;
    }

    let initial_target_freq_hz = initial_center_freq_hz;

    debug!(
        "init_cf = {:?}, init_tf = {:?}",
        initial_center_freq_hz, initial_target_freq_hz
    );

    (initial_center_freq_hz as u64, initial_target_freq_hz as u64)
}

#[cfg(test)]
mod hr50_path_tests {
    use super::*;

    #[test]
    fn parses_path_and_optional_baud() {
        assert_eq!(
            parse_hr50_path_baud("/dev/ttyUSB0"),
            ("/dev/ttyUSB0".to_string(), HR50_DEFAULT_BAUD)
        );
        assert_eq!(
            parse_hr50_path_baud("/dev/ttyUSB0:9600"),
            ("/dev/ttyUSB0".to_string(), 9600)
        );
        // A non-numeric suffix is part of the path, not a baud.
        assert_eq!(
            parse_hr50_path_baud("/dev/serial/by-id/usb-FTDI"),
            ("/dev/serial/by-id/usb-FTDI".to_string(), HR50_DEFAULT_BAUD)
        );
    }
}

#[cfg(test)]
mod volume_tests {
    use super::*;

    #[test]
    fn soft_clip_transparent_below_threshold() {
        for &x in &[-0.9_f32, -0.5, 0.0, 0.3, 0.95] {
            assert!(
                (soft_clip(x) - x).abs() < 1e-6,
                "identity below threshold for {x}"
            );
        }
    }

    #[test]
    fn soft_clip_bounded_finite_and_sign_preserving() {
        for &x in &[1.0_f32, 2.0, 4.0, 100.0, -4.0, -100.0] {
            let y = soft_clip(x);
            assert!(y.is_finite(), "finite for {x}");
            assert!(y.abs() <= 1.0, "bounded for {x}: {y}");
            assert_eq!(y.signum(), x.signum(), "sign preserved for {x}");
        }
        // Continuity-ish: just above threshold stays close to threshold.
        assert!((soft_clip(0.96) - 0.95).abs() < 0.02);
    }
}
