use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc as std_mpsc,
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, trace};
use num_complex::Complex32;
use tokio::sync::{mpsc, oneshot, watch};

use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::{HardwareKind, RadioDescriptor};

use crate::dsp::pipeline::{DspPipeline, DspPipelineConfig};
use crate::config::{
    choose_block_size, choose_decimation, ServerConfig, SourceKind, WATERFALL_BINS,
    WATERFALL_FRAME_RATE_HZ,
};
use crate::radio::types::{
    AcquireRequest, StopReason, WorkerCommand, WorkerExit, WorkerReadyInfo,
    WorkerRuntimeState, WorkerStartResult, WorkerStatus,
};
use crate::source::factory::{create_source, SourceConfig};
use crate::source::wav_metadata::parse_center_freq_hz_from_filename;
use crate::source::IqSource;
use crate::net::udp::udp_audio::UdpAudioSender;
use crate::net::udp::udp_waterfall::UdpWaterfallSender;
use crate::waterfall::generator::WaterfallGenerator;

#[derive(Debug, Clone)]
struct SharedControlState {
    center_freq_hz: u64,
    target_freq_hz: u64,
    demod_mode: DemodMode,
    sideband: Sideband,
    ssb_pitch_hz: f32,
    filter_bandwidth_hz: f32,
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
            device_index: server_cfg.rtlsdr_device_index,
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
    } else {
        return Err(format!("source for {} not implemented", descriptor.id.0));
    };

    create_source(config)
}

fn pipeline_cfg_for_source(
    server_cfg: &ServerConfig,
    source_kind: &SourceKind,
    center_freq_hz: u64,
    target_freq_hz: u64,
    input_sample_rate_hz: f32,
) -> DspPipelineConfig {
    let (channel_cutoff_hz, audio_cutoff_hz) = match server_cfg.demod {
        DemodMode::Wfm => (100_000.0, 15_000.0),
        DemodMode::Nfm => (12_500.0, 5_000.0),
        DemodMode::Usb => (4_000.0, 3_000.0),
        DemodMode::Lsb => (4_000.0, 3_000.0),
	DemodMode::Am => (6_000.0, 5_000.0),
	DemodMode::Cw => (1_200.0, 900.0),
    };

    DspPipelineConfig {
        center_freq_hz: center_freq_hz as f32,
        target_freq_hz: target_freq_hz as f32,
        input_sample_rate_hz,

        channel_cutoff_hz,
        fir_taps: 129,
        decimation_factor: choose_decimation(source_kind),

        audio_cutoff_hz,
        audio_fir_taps: 129,

        client_output_sample_rate_hz: 48_000.0,
        mode: server_cfg.demod,
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
	filter_bandwidth_hz: control.filter_bandwidth_hz,

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
	filter_bandwidth_hz: 3000.0 // sensible default
    }));

    let (iq_audio_tx, iq_audio_rx) = std_mpsc::sync_channel::<Vec<Complex32>>(2);
    let (iq_wf_tx, iq_wf_rx) = std_mpsc::sync_channel::<Vec<Complex32>>(4);
    let (startup_info_tx, startup_info_rx) =
        std_mpsc::sync_channel::<Result<StartupInfo, String>>(1);
    let (capture_start_tx, capture_start_rx) = std_mpsc::sync_channel::<()>(1);
    let (fatal_tx, fatal_rx) = std_mpsc::channel::<WorkerExit>();

    let cmd_thread = spawn_command_thread(
        cmd_rx,
        control.clone(),
        stop_flag.clone(),
        stop_reason.clone(),
        fatal_tx.clone(),
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
        source_kind.clone(),
        control.clone(),
        stop_flag.clone(),
        fatal_tx.clone(),
        status_tx.clone(),
        iq_audio_rx,
        request.audio_udp_peer,
        startup_info.clone(),
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

fn spawn_command_thread(
    cmd_rx: std_mpsc::Receiver<WorkerCommand>,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    stop_reason: Arc<Mutex<StopReason>>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop_requested(&stop_flag) {
            match cmd_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(cmd) => match cmd {
                    WorkerCommand::SetTargetFrequency { hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.target_freq_hz = hz;
                        }
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
                    WorkerCommand::SetSsbPitch { pitch_hz } => {
                        if let Ok(mut control_state) = control.lock() {
                            control_state.ssb_pitch_hz = pitch_hz.clamp(-1000.0, 1000.0);
                        }
                    }
                    WorkerCommand::Stop { reason } => {
                        set_stop_reason(&stop_reason, reason);
                        stop_flag.store(true, Ordering::Relaxed);
                        break;
                    }
		    WorkerCommand::SetFilterBandwidth { bandwidth_hz } => {
			if let Ok(mut control_state) = control.lock() {
			    control_state.filter_bandwidth_hz = bandwidth_hz.clamp(100.0, 20000.0);
			}
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

        debug!("[radio-worker {}] waterfall thread exiting", descriptor.id.0);
    })
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

        // Wait until the coordinator has published startup state and spun up
        // the downstream threads before capture begins feeding data.
        let _ = capture_start_rx.recv();

        let realtime = source.is_realtime();
        let source_block_period =
            Duration::from_secs_f32((block_size as f32 / source.sample_rate()).max(0.001));
        let mut next_source_tick = Instant::now();
        let mut last_center_freq_hz = initial_center_freq_hz;
        let mut blocks_read: u64 = 0;

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

            if control_snapshot.center_freq_hz != last_center_freq_hz {
                if let Err(reason) =
                    source.set_center_frequency(control_snapshot.center_freq_hz as f32)
                {
                    stop_flag.store(true, Ordering::Relaxed);
                    let _ = fatal_tx.send(WorkerExit::Failed { reason });
                    break;
                }

                last_center_freq_hz = control_snapshot.center_freq_hz;
            }

            if !realtime {
                let now = Instant::now();
                if now < next_source_tick {
                    thread::sleep(next_source_tick - now);
                }
                next_source_tick += source_block_period;
            }

            let iq = match source.read_block(block_size) {
                Ok(samples) => samples,
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
                    // WAV EOF or similar: treat as a clean stop.
                    set_stop_reason(&stop_reason, StopReason::UserRequested);
                    stop_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }

            blocks_read += 1;
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
    })
}

fn spawn_dsp_thread(
    descriptor: RadioDescriptor,
    server_cfg: ServerConfig,
    source_kind: SourceKind,
    control: Arc<Mutex<SharedControlState>>,
    stop_flag: Arc<AtomicBool>,
    fatal_tx: std_mpsc::Sender<WorkerExit>,
    status_tx: watch::Sender<WorkerStatus>,
    iq_audio_rx: std_mpsc::Receiver<Vec<Complex32>>,
    audio_target: std::net::SocketAddr,
    startup_info: StartupInfo,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pipeline = DspPipeline::new(pipeline_cfg_for_source(
            &server_cfg,
            &source_kind,
            startup_info.runtime.center_freq_hz,
            startup_info.runtime.target_freq_hz,
            startup_info.input_sample_rate_hz,
        ));

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

        loop {
            if stop_requested(&stop_flag) {
                break;
            }

            let current = current_control(&control);
            let mut changed = false;

            if current.center_freq_hz != applied.center_freq_hz {
                pipeline.set_center_frequency(current.center_freq_hz as f32);
                applied.center_freq_hz = current.center_freq_hz;
                changed = true;
            }

            if current.target_freq_hz != applied.target_freq_hz {
                pipeline.set_target_frequency(current.target_freq_hz as f32);
                applied.target_freq_hz = current.target_freq_hz;
                changed = true;
            }

            if current.demod_mode != applied.demod_mode {
                let cfg = ServerConfig {
                    demod: current.demod_mode,
                    ..server_cfg.clone()
                };

                pipeline = DspPipeline::new(pipeline_cfg_for_source(
                    &cfg,
                    &source_kind,
                    current.center_freq_hz,
                    current.target_freq_hz,
                    startup_info.input_sample_rate_hz,
                ));

                if matches!(current.demod_mode, DemodMode::Usb | DemodMode::Lsb) {
                    pipeline.set_sideband(current.sideband);
                    pipeline.set_ssb_pitch_hz(current.ssb_pitch_hz);
                }

                applied.demod_mode = current.demod_mode;
                applied.sideband = current.sideband;
                applied.ssb_pitch_hz = current.ssb_pitch_hz;
                applied.center_freq_hz = current.center_freq_hz;
                applied.target_freq_hz = current.target_freq_hz;
		applied.filter_bandwidth_hz = current.filter_bandwidth_hz;
                changed = true;
            } else {
                if current.sideband != applied.sideband {
                    pipeline.set_sideband(current.sideband);
                    applied.sideband = current.sideband;
                    changed = true;
                }

                if (current.ssb_pitch_hz - applied.ssb_pitch_hz).abs() > f32::EPSILON {
                    pipeline.set_ssb_pitch_hz(current.ssb_pitch_hz);
                    applied.ssb_pitch_hz = current.ssb_pitch_hz;
                    changed = true;
                }

		if (current.filter_bandwidth_hz - applied.filter_bandwidth_hz).abs() > f32::EPSILON {
		    pipeline.set_filter_bandwidth_hz(current.filter_bandwidth_hz);
		    applied.filter_bandwidth_hz = current.filter_bandwidth_hz;
		    changed = true;
		}
            }

            if changed {
                let runtime = build_runtime_state(&current, startup_info.input_sample_rate_hz);
                let _ = status_tx.send(WorkerStatus::Running { runtime });
            }

            match iq_audio_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(iq) => {
                    let audio_f32 = pipeline.process_audio(&iq);

                    let mut audio_i16 = Vec::with_capacity(audio_f32.len());
                    for sample in audio_f32 {
                        let value =
                            (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
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

    let initial_target_freq_hz = initial_center_freq_hz;

    debug!(
        "init_cf = {:?}, init_tf = {:?}",
        initial_center_freq_hz, initial_target_freq_hz
    );

    (
        initial_center_freq_hz as u64,
        initial_target_freq_hz as u64,
    )
}
