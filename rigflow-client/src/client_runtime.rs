use std::net::UdpSocket;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
//use std::time::{Duration, Instant}; // Instant is needed for periodic jitter logging
use std::time::Duration;

use log::{error, info};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{
        MAGIC, STREAM_TYPE_AUDIO, STREAM_TYPE_MIC_AUDIO, STREAM_TYPE_REGISTER_AUDIO,
        STREAM_TYPE_TIME_SYNC_RESPONSE, VERSION, audio_send_wall_ns, build_time_sync_request,
        clock_offset_rtt, epoch_nanos, parse_media_header, parse_time_sync_response,
    },
};

use crate::{
    net::udp::{MediaPacketStats, WaterfallReassembler, handle_media_packet},
    sidetone::SidetoneShared,
    ui::{
        layout::{
            HEIGHT, SPECTRUM_DB_MIN, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1, WATERFALL_TOP, WIDTH,
        },
        state::UiState,
        stats::ClientStatsLogger,
    },
};

/// UDP socket listen address for media plane (audio + waterfall)
const LISTEN_ADDR: &str = "0.0.0.0:50000";

/// Audio packet size (samples per packet)
const PACKET_SAMPLES: usize = 240; //480;

/// Target jitter buffer size (latency control)
/// 4800 samples @ 48kHz ≈ 100 ms
/// To improve latency, tried reducing to 3,360, and then 2,800
/// 2,800 @ 48kHz = 60 ms
const TARGET_BUFFER_SAMPLES: usize = 2_880; //3_360; //4_800;

/// Maximum jitter buffer size (latency ceiling)
const MAX_BUFFER_SAMPLES: usize = 24_000;

/// Audio output configuration
const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;

/// Commands sent from UI → media runtime thread
#[derive(Debug, Clone)]
pub enum MediaCommand {
    /// Register this client with the server's UDP media plane
    RegisterUdp {
        server_ip: String,
        server_udp_port: u16,
    },
}

/// Handles returned to the UI for interacting with the media runtime
pub struct MediaRuntimeHandles {
    /// Control channel into the media thread
    pub media_cmd_tx: mpsc::UnboundedSender<MediaCommand>,

    /// Waterfall pixel buffer (ARGB)
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,

    /// Spectrum dB values (for plotting)
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,

    /// Audio output stream (held to keep playback alive). `None` when no output
    /// device could be opened — the client runs without local speaker audio
    /// rather than aborting.
    pub _audio_stream: Option<cpal::Stream>,

    /// Generation counter used to reset audio state on radio switch
    pub audio_session_generation: Arc<AtomicU64>,
}

/// Start the media runtime:
/// - UDP receive loop (audio + waterfall)
/// - jitter buffer + audio playback
/// - shared buffers for UI rendering
pub fn start_media_runtime(
    ui_state: Arc<Mutex<UiState>>,
) -> Result<MediaRuntimeHandles, Box<dyn std::error::Error>> {
    // --- UDP setup ---------------------------------------------------------

    let socket = UdpSocket::bind(LISTEN_ADDR)?;
    socket.set_read_timeout(Some(Duration::from_millis(5)))?;

    let udp_listen_port = socket.local_addr()?.port();

    {
        let mut state = ui_state.lock().unwrap();
        state.udp_listen_port = udp_listen_port;
    }

    info!("Media runtime listening on {}", socket.local_addr()?);

    // --- Jitter buffer -----------------------------------------------------

    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        PACKET_SAMPLES,
        TARGET_BUFFER_SAMPLES,
        MAX_BUFFER_SAMPLES,
    )));

    // --- Periodic client stats logger -------------------------------------

    let stats_logger = Arc::new(Mutex::new(ClientStatsLogger::new()));

    // --- Audio output (CPAL) ----------------------------------------------

    let host = cpal::default_host();

    // Lock-free CW sidetone control, shared with the UI thread via UiState.
    // Extract the Arc once here so the real-time audio callback never locks.
    let sidetone = ui_state
        .lock()
        .map(|s| Arc::clone(&s.sidetone))
        .unwrap_or_default();

    // Receive Volume is applied here on the client (speaker path only), so the
    // Digital Audio Interface RX tap stays at fixed unity gain.  The media
    // thread mirrors `UiState.volume_percent` into this lock-free atomic; the
    // real-time audio callback reads it without locking.
    let rx_volume = Arc::new(AtomicU8::new(50));

    // Open the speaker output, degrading gracefully instead of aborting. A
    // machine whose *default* ALSA device can't be opened — e.g. a headless Pi
    // whose default is an unconnected HDMI sink — should still run, since radio
    // control and the digital (FT8) paths don't need local playback. `None`
    // means no local speaker audio; the rest of the media runtime continues.
    let audio_stream = open_audio_output(&host, &jitter, &stats_logger, &sidetone, &rx_volume);
    if audio_stream.is_none() {
        log::error!(
            "audio: no usable output device found — running without local speaker \
             audio (radio control and digital modes still work)."
        );
    }

    // --- UI buffers --------------------------------------------------------

    let waterfall_width = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
    let waterfall_height = HEIGHT - WATERFALL_TOP;

    let waterfall_buffer = Arc::new(Mutex::new(vec![0u32; waterfall_width * waterfall_height]));

    let spectrum_db = Arc::new(Mutex::new(vec![SPECTRUM_DB_MIN; WIDTH]));
    let media_stats = Arc::new(Mutex::new(MediaPacketStats::new()));

    // --- Control channel ---------------------------------------------------

    let (media_cmd_tx, mut media_cmd_rx) = mpsc::unbounded_channel::<MediaCommand>();

    // --- Shared state clones for thread -----------------------------------

    let jitter_for_thread = Arc::clone(&jitter);
    let waterfall_for_thread = Arc::clone(&waterfall_buffer);
    let spectrum_for_thread = Arc::clone(&spectrum_db);
    let stats_for_thread = Arc::clone(&media_stats);
    let stats_logger_for_thread = Arc::clone(&stats_logger);

    // --- Audio session generation (for radio switching) -------------------

    let audio_session_generation = Arc::new(AtomicU64::new(0));
    let audio_session_generation_for_thread = Arc::clone(&audio_session_generation);

    // CW decoder: shared control/output lives in UiState; the decoder DSP state
    // is owned by the media thread and fed the received audio.
    let cw_decode_shared = ui_state
        .lock()
        .map(|s| Arc::clone(&s.cw_decode))
        .unwrap_or_default();

    // Mic capture shares an outbound TX-audio ring; the media thread drains it
    // and sends mic packets to the server while SSB mic TX is keyed.
    let mic_shared = ui_state
        .lock()
        .map(|s| Arc::clone(&s.mic_shared))
        .unwrap_or_default();

    // Lock-free audio/latency metrics, published by the media thread and read by
    // the UI's Latency panel.
    let audio_metrics = ui_state
        .lock()
        .map(|s| Arc::clone(&s.audio_metrics))
        .unwrap_or_else(|_| crate::audio_metrics::AudioMetrics::new());

    // Digital RX router: when enabled, a copy of received audio is mirrored to
    // the RigflowDigitalOutput sink (Digital Audio Interface Phase 2).
    let digital_rx = ui_state
        .lock()
        .map(|s| Arc::clone(&s.digital_rx))
        .unwrap_or_else(|_| crate::digital_rx::DigitalRxOutput::new());

    // TCI server RX-audio tap: a copy of received audio is mirrored here while a
    // TCI digital app (JTDX) is streaming.  Shared with the TCI server task.
    let tci_rx_audio = ui_state
        .lock()
        .map(|s| Arc::clone(&s.tci_rx_audio))
        .unwrap_or_else(|_| crate::tci_server::TciRxAudio::new());

    // --- Dedicated mic-TX send thread -------------------------------------
    // Mic / digital-TX audio is sent from its own paced thread, NOT the media
    // loop.  The media loop processes the inbound full-duplex RX/waterfall flood
    // and locks `ui_state` every iteration, so a slow UI frame holding that lock
    // stalled the mic send for tens of ms — long enough to drain the server's TX
    // queue and starve the modulator (underruns).  This thread only drains the
    // mic ring and sends UDP, never locking `ui_state`, so delivery stays smooth.
    // The server address is learned by the media loop at RegisterUdp and shared.
    let mic_server_addr: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let mic_socket = socket.try_clone()?;
        let mic_addr = Arc::clone(&mic_server_addr);
        let mic_metrics = Arc::clone(&audio_metrics);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(5));
                let samples = mic_shared.drain_tx();
                // Publish the TX-ring depth (drained this cycle, 0 while idle so the
                // peak decays) for the Latency panel.
                mic_metrics.note_tx_ring(samples.len());
                if samples.is_empty() {
                    continue;
                }
                let Some(addr) = mic_addr.lock().ok().and_then(|g| g.clone()) else {
                    continue;
                };
                for chunk in samples.chunks(256) {
                    let mut pkt = Vec::with_capacity(4 + chunk.len() * 4);
                    pkt.extend_from_slice(&MAGIC.to_be_bytes());
                    pkt.push(VERSION);
                    pkt.push(STREAM_TYPE_MIC_AUDIO);
                    for &s in chunk {
                        pkt.extend_from_slice(&s.to_le_bytes());
                    }
                    let _ = mic_socket.send_to(&pkt, &addr);
                }
            }
        });
    }

    // --- Clock-offset probe thread ----------------------------------------
    // Periodically sends a tiny TIME_SYNC request (stamped with the client send
    // time T1) to the server, which echoes T1 plus its receive/send times.  The
    // reply arrives back on the media socket and is handled by the media loop,
    // which computes the clock offset + RTT.  Probing once a second keeps the
    // offset EMA converged and bounds clock-drift error without meaningful cost.
    {
        let probe_socket = socket.try_clone()?;
        let probe_addr = Arc::clone(&mic_server_addr);
        thread::spawn(move || {
            let mut probe_id: u32 = 0;
            loop {
                thread::sleep(Duration::from_millis(1000));
                let Some(addr) = probe_addr.lock().ok().and_then(|g| g.clone()) else {
                    continue;
                };
                probe_id = probe_id.wrapping_add(1);
                let pkt = build_time_sync_request(probe_id, epoch_nanos());
                let _ = probe_socket.send_to(&pkt, &addr);
            }
        });
    }

    // --- Media thread ------------------------------------------------------

    let audio_metrics_for_thread = Arc::clone(&audio_metrics);

    thread::spawn(move || {
        let mut udp_buf = [0u8; 65536];

        // CW decoder DSP state, owned by this thread; fed received audio per
        // packet (no-op unless the operator enabled decode).
        let mut cw_decoder =
            crate::cw_decode::CwDecoder::new(cw_decode_shared, OUTPUT_SAMPLE_RATE as f32);

        // Reassembles waterfall rows from their sub-MTU chunks; owned by this thread.
        let mut waterfall_reasm = WaterfallReassembler::new();

        let mut last_audio_session_generation =
            audio_session_generation_for_thread.load(Ordering::Relaxed);

        // let mut last_stats_log = Instant::now();  // Needed for periodic jitter logging

        loop {
            // Mirror the current receive volume into the lock-free atomic the
            // audio callback reads (volume is applied client-side, speaker only).
            if let Ok(s) = ui_state.lock() {
                rx_volume.store(s.volume_percent, Ordering::Relaxed);
            }

            // --- Session change detection (radio switch) -------------------

            let current = audio_session_generation_for_thread.load(Ordering::Relaxed);

            if current != last_audio_session_generation {
                if let Ok(mut jb) = jitter.lock() {
                    jb.reset();
                }

                last_audio_session_generation = current;
                info!("[client] jitter buffer reset for new radio session");
            }

            // --- Existing ad hoc jitter diagnostics -----------------------

            /*
                if last_stats_log.elapsed() >= Duration::from_secs(2) {
                    if let Ok(jb) = jitter.lock() {
                        let current_samples = jb.buffered_samples();
                        let max_samples = jb.max_buffered_samples();

                        let sr = 48_000.0;

                        let current_ms = current_samples as f32 / sr * 1000.0;
                        let max_ms = max_samples as f32 / sr * 1000.0;

                        println!(
                            "[client-audio] jitter: current={:.1} ms max={:.1} ms \
                             started={} rx={} inserted={} late={} overflow={}",
                            current_ms,
                            max_ms,
                            jb.started(),
                            jb.packets_received,
                            jb.packets_inserted,
                            jb.packets_dropped_late,
                            jb.packets_dropped_overflow,
                        );
                    }

                    last_stats_log = Instant::now();
                }
            */

            // --- Additional interval-based stats logger -------------------

            if let Ok(jb) = jitter_for_thread.lock() {
                // Publish live RX jitter-buffer occupancy + health for the UI.
                audio_metrics_for_thread.publish_jitter(
                    jb.buffered_samples(),
                    jb.packets_missing_concealed,
                    jb.packets_dropped_late,
                    jb.packets_dropped_overflow,
                    jb.resync_count,
                );
                if let (Ok(mut logger), Ok(mut media_stats)) =
                    (stats_logger_for_thread.lock(), stats_for_thread.lock())
                {
                    logger.maybe_log(&mut media_stats, &jb, OUTPUT_SAMPLE_RATE as f32);
                }
            }

            // --- Handle control commands ----------------------------------

            while let Ok(cmd) = media_cmd_rx.try_recv() {
                match cmd {
                    MediaCommand::RegisterUdp {
                        server_ip,
                        server_udp_port,
                    } => {
                        let mut reg = Vec::with_capacity(4);
                        reg.extend_from_slice(&MAGIC.to_be_bytes());
                        reg.push(VERSION);
                        reg.push(STREAM_TYPE_REGISTER_AUDIO);

                        let addr = format!("{}:{}", server_ip, server_udp_port);

                        match socket.send_to(&reg, &addr) {
                            Ok(_) => info!("Sent UDP registration to {}", addr),
                            Err(e) => error!("UDP registration failed to {}: {}", addr, e),
                        }
                        // Same endpoint receives mic-audio packets — hand it to
                        // the dedicated mic-send thread.
                        if let Ok(mut g) = mic_server_addr.lock() {
                            *g = Some(addr);
                        }
                    }
                }
            }

            // (Mic-TX audio is sent from the dedicated thread above, not here.)

            // --- Receive UDP packets --------------------------------------

            match socket.recv_from(&mut udp_buf) {
                Ok((len, src)) => {
                    // Capture the client receive time ASAP (T4 / audio recv stamp).
                    let recv_ns = epoch_nanos();
                    let stream_type = if len >= 4 { udp_buf[3] } else { 0 };

                    // Registration ACK (4-byte control packet)
                    if len == 4 {
                        let magic = u16::from_be_bytes([udp_buf[0], udp_buf[1]]);
                        let version = udp_buf[2];

                        if magic == MAGIC
                            && version == VERSION
                            && stream_type == STREAM_TYPE_REGISTER_AUDIO
                        {
                            info!("Received UDP registration ACK from {}", src);
                        }
                    }
                    // Clock-offset probe reply: compute offset + RTT and feed the
                    // metrics (T4 = recv_ns captured above).
                    else if stream_type == STREAM_TYPE_TIME_SYNC_RESPONSE {
                        if let Some((_id, t1, t2, t3)) = parse_time_sync_response(&udp_buf[..len]) {
                            let (offset, rtt) = clock_offset_rtt(t1, t2, t3, recv_ns);
                            audio_metrics_for_thread.record_probe(offset, rtt);
                        }
                    }
                    // Media packet (audio or waterfall)
                    else if len >= 16 {
                        // One-way RX network latency from the v2 audio send-stamp.
                        if stream_type == STREAM_TYPE_AUDIO {
                            if let Some(h) = parse_media_header(&udp_buf[..len]) {
                                if let Some(send_ns) = audio_send_wall_ns(&h, &udp_buf[..len]) {
                                    audio_metrics_for_thread.record_audio_one_way(recv_ns, send_ns);
                                }
                            }
                        }

                        let ui_state_for_thread = Arc::clone(&ui_state);
                        handle_media_packet(
                            &udp_buf[..len],
                            &jitter_for_thread,
                            &waterfall_for_thread,
                            &spectrum_for_thread,
                            &ui_state_for_thread,
                            &stats_for_thread,
                            &mut cw_decoder,
                            &digital_rx,
                            &tci_rx_audio,
                            &mut waterfall_reasm,
                        );
                    }
                }

                // Normal non-blocking timeout
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}

                // Unexpected error
                Err(e) => {
                    error!("UDP receive error: {}", e);
                }
            }
        }
    });

    // --- Return handles to UI ---------------------------------------------

    Ok(MediaRuntimeHandles {
        media_cmd_tx,
        waterfall_buffer,
        spectrum_db,
        _audio_stream: audio_stream,
        audio_session_generation,
    })
}

/// Open a CPAL output stream, degrading gracefully instead of aborting the
/// client. Tries the host default device first, then every other output device,
/// using the first that opens. Returns `None` if none can be opened (e.g. a
/// headless Pi whose default ALSA device is an unconnected HDMI sink) — the
/// client then runs without local speaker audio.
fn open_audio_output(
    host: &cpal::Host,
    jitter: &Arc<Mutex<JitterBuffer>>,
    stats_logger: &Arc<Mutex<ClientStatsLogger>>,
    sidetone: &Arc<SidetoneShared>,
    rx_volume: &Arc<AtomicU8>,
) -> Option<cpal::Stream> {
    let mut tried: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1) Prefer the sound-server devices: they route through PipeWire/PulseAudio
    //    to the user's chosen default sink and "just work", avoiding a raw hw:/
    //    HDMI default (which may not open) and the `jack` plugin (which spews
    //    connection errors when no JACK server is running). Try in priority order.
    for want in ["pipewire", "pulse"] {
        if let Ok(devices) = host.output_devices() {
            for device in devices {
                if device.name().map(|n| n == want).unwrap_or(false) {
                    if let Some(stream) = try_open_output(
                        device,
                        jitter,
                        stats_logger,
                        sidetone,
                        rx_volume,
                        &mut tried,
                    ) {
                        return Some(stream);
                    }
                }
            }
        }
    }
    // 2) Host default.
    if let Some(device) = host.default_output_device() {
        if let Some(stream) = try_open_output(
            device,
            jitter,
            stats_logger,
            sidetone,
            rx_volume,
            &mut tried,
        ) {
            return Some(stream);
        }
    }
    // 3) Any other output device, skipping `jack` (never auto-selected: it fails
    //    on our mono config and probing it prints noisy errors when no JACK server
    //    is up; a JACK user could force it via an explicit device flag later).
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if device.name().map(|n| n.contains("jack")).unwrap_or(false) {
                continue;
            }
            if let Some(stream) = try_open_output(
                device,
                jitter,
                stats_logger,
                sidetone,
                rx_volume,
                &mut tried,
            ) {
                return Some(stream);
            }
        }
    }
    None
}

/// Attempt to open one output device; logs and returns `None` on failure.
/// `tried` dedups by device name so the default isn't retried during the sweep.
fn try_open_output(
    device: cpal::Device,
    jitter: &Arc<Mutex<JitterBuffer>>,
    stats_logger: &Arc<Mutex<ClientStatsLogger>>,
    sidetone: &Arc<SidetoneShared>,
    rx_volume: &Arc<AtomicU8>,
    tried: &mut std::collections::HashSet<String>,
) -> Option<cpal::Stream> {
    let name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
    if !tried.insert(name.clone()) {
        return None;
    }
    let config = match device.default_output_config() {
        Ok(c) => c.config(),
        Err(e) => {
            log::warn!("audio: output device '{name}' config query failed: {e}");
            return None;
        }
    };
    match build_output_stream(
        &device,
        &config,
        Arc::clone(jitter),
        Arc::clone(stats_logger),
        Arc::clone(sidetone),
        Arc::clone(rx_volume),
    ) {
        Ok(stream) => match stream.play() {
            Ok(()) => Some(stream),
            Err(e) => {
                log::warn!("audio: output device '{name}' failed to start: {e}");
                None
            }
        },
        Err(e) => {
            log::warn!("audio: output device '{name}' could not be opened: {e}");
            None
        }
    }
}

/// Build the CPAL output stream.
///
/// This:
/// - selects a compatible output config
/// - pulls audio from the jitter buffer
/// - fills silence on lock failure or underflow
/// - feeds audio sample counts into the periodic stats logger
fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    jitter: Arc<Mutex<JitterBuffer>>,
    stats_logger: Arc<Mutex<ClientStatsLogger>>,
    sidetone: Arc<SidetoneShared>,
    rx_volume: Arc<AtomicU8>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let supported_configs = device.supported_output_configs()?;

    let mut selected = None;

    // Try to find an exact match for our preferred format
    for cfg in supported_configs {
        if cfg.channels() == CHANNELS
            && cfg.sample_format() == cpal::SampleFormat::F32
            && OUTPUT_SAMPLE_RATE >= cfg.min_sample_rate().0
            && OUTPUT_SAMPLE_RATE <= cfg.max_sample_rate().0
        {
            selected = Some(cfg.with_sample_rate(cpal::SampleRate(OUTPUT_SAMPLE_RATE)));
            break;
        }
    }

    let selected_config = if let Some(cfg) = selected {
        cfg.config()
    } else {
        // Fallback: adapt default config
        let mut cfg = config.clone();
        cfg.channels = CHANNELS;
        cfg.sample_rate = cpal::SampleRate(OUTPUT_SAMPLE_RATE);
        cfg
    };

    log::info!(
        "Using output device: {} @ {} Hz",
        device.name().unwrap_or_else(|_| "<unknown>".to_string()),
        selected_config.sample_rate.0
    );

    let err_fn = |err| {
        log::error!("audio stream error: {err}");
    };

    let jitter_for_audio = Arc::clone(&jitter);
    let stats_logger_for_audio = Arc::clone(&stats_logger);

    // CW sidetone oscillator state, owned by the audio callback (real-time):
    // continuous phase + a smooth rise/fall envelope so start/stop never clicks.
    let stream_sample_rate = selected_config.sample_rate.0 as f32;
    // 5 ms raised-cosine rise/fall → linear `env` ramp shaped by sin².
    let env_step = 1.0 / (0.005 * stream_sample_rate).max(1.0);
    let mut phase: f32 = 0.0;
    let mut env: f32 = 0.0;
    // Client-side receive-Volume gain, ramped across each callback (click-free).
    let mut applied_gain: f32 = 1.0;

    let stream = device.build_output_stream(
        &selected_config,
        move |data: &mut [f32], _| {
            if let Ok(mut jb) = jitter_for_audio.lock() {
                jb.pop_samples(data);
            } else {
                // Fallback: output silence if lock fails
                for sample in data.iter_mut() {
                    *sample = 0.0;
                }
            }

            // --- Receive Volume (applied here, speaker path only) ----------
            // Volume moved from the server to the client so the Digital Audio
            // Interface RX tap stays at fixed unity gain.  Ramp toward the
            // target across the callback (no clicks) and soft-limit so a boost
            // can't hard-clip the speaker.  Mapping: 0% silence, 50% unity,
            // 100% +12 dB.  Applied before the sidetone mix (sidetone has its
            // own volume).
            let target_gain = volume_gain(rx_volume.load(Ordering::Relaxed));
            let n = data.len();
            if n > 0 {
                let step = (target_gain - applied_gain) / n as f32;
                let mut g = applied_gain;
                for sample in data.iter_mut() {
                    g += step;
                    *sample = soft_clip(*sample * g);
                }
                applied_gain = target_gain;
            }

            // --- Mix in the local CW sidetone (never sent to the server) ---
            // Read the lock-free control once per callback; phase/env persist
            // across callbacks for continuity.
            let target = if sidetone.keyed() { 1.0 } else { 0.0 };
            let volume = sidetone.volume();
            let phase_inc = std::f32::consts::TAU * sidetone.pitch_hz() / stream_sample_rate;
            for sample in data.iter_mut() {
                // Ramp the envelope toward the keyed target (5 ms), then shape
                // it with a raised cosine for a click-free attack/decay.
                if env < target {
                    env = (env + env_step).min(target);
                } else if env > target {
                    env = (env - env_step).max(target);
                }
                let shaped = 0.5 * (1.0 - (std::f32::consts::PI * env).cos());
                let tone = volume * shaped * phase.sin();
                *sample = (*sample + tone).clamp(-1.0, 1.0);

                phase += phase_inc;
                if phase >= std::f32::consts::TAU {
                    phase -= std::f32::consts::TAU;
                }
            }

            if let Ok(mut logger) = stats_logger_for_audio.lock() {
                logger.add_audio_samples(data.len());
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

/// Receive-volume gain from a percent (0–100).  Moved client-side (from the
/// server) so the digital RX tap stays at unity.  `0%` → silence, `50%` →
/// unity, `100%` → +12 dB: `gain = 10^(((vp-50)/50)*12/20)`.
fn volume_gain(volume_percent: u8) -> f32 {
    if volume_percent == 0 {
        return 0.0;
    }
    let db = ((volume_percent.min(100) as f32 - 50.0) / 50.0) * 12.0;
    10.0_f32.powf(db / 20.0)
}

/// Soft limiter: transparent for `|x| <= 0.95`, then a smooth tanh knee that
/// asymptotes to ±1.0 so a volume boost can't hard-clip the speaker.  Matches
/// the server's `soft_clip` so behaviour is identical after moving Volume.
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
