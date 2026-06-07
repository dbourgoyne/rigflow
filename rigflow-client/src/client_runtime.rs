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
    net::udp_framing::{MAGIC, STREAM_TYPE_MIC_AUDIO, STREAM_TYPE_REGISTER_AUDIO, VERSION},
};

use crate::{
    net::udp::{MediaPacketStats, handle_media_packet},
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

    /// Audio stream (must be held to keep playback alive)
    pub _audio_stream: cpal::Stream,

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
    let device = host
        .default_output_device()
        .ok_or("No default output device available")?;

    let config = device.default_output_config()?.config();

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

    let audio_stream = build_output_stream(
        &device,
        &config,
        Arc::clone(&jitter),
        Arc::clone(&stats_logger),
        sidetone,
        Arc::clone(&rx_volume),
    )?;
    audio_stream.play()?;

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

    // Digital RX router: when enabled, a copy of received audio is mirrored to
    // the RigflowDigitalOutput sink (Digital Audio Interface Phase 2).
    let digital_rx = ui_state
        .lock()
        .map(|s| Arc::clone(&s.digital_rx))
        .unwrap_or_else(|_| crate::digital_rx::DigitalRxOutput::new());

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
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(5));
                let samples = mic_shared.drain_tx();
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

    // --- Media thread ------------------------------------------------------

    thread::spawn(move || {
        let mut udp_buf = [0u8; 65536];

        // CW decoder DSP state, owned by this thread; fed received audio per
        // packet (no-op unless the operator enabled decode).
        let mut cw_decoder =
            crate::cw_decode::CwDecoder::new(cw_decode_shared, OUTPUT_SAMPLE_RATE as f32);

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

            if let (Ok(mut logger), Ok(mut media_stats), Ok(jb)) = (
                stats_logger_for_thread.lock(),
                stats_for_thread.lock(),
                jitter_for_thread.lock(),
            ) {
                logger.maybe_log(&mut media_stats, &jb, OUTPUT_SAMPLE_RATE as f32);
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
                    // Registration ACK (4-byte control packet)
                    if len == 4 {
                        let magic = u16::from_be_bytes([udp_buf[0], udp_buf[1]]);
                        let version = udp_buf[2];
                        let stream_type = udp_buf[3];

                        if magic == MAGIC
                            && version == VERSION
                            && stream_type == STREAM_TYPE_REGISTER_AUDIO
                        {
                            info!("Received UDP registration ACK from {}", src);
                        }
                    }
                    // Media packet (audio or waterfall)
                    else if len >= 16 {
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
