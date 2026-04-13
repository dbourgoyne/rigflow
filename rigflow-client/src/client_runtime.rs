use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
//use std::time::{Duration, Instant}; // Instant is needed for periodic jitter logging
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{MAGIC, STREAM_TYPE_REGISTER_AUDIO, VERSION},
};

use crate::{
    net::udp::{handle_media_packet, MediaPacketStats},
    ui::{
	layout::{
            HEIGHT, SPECTRUM_DB_MIN, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1,
            WATERFALL_TOP, WIDTH,
        },
	state::UiState,
	stats::ClientStatsLogger,
    },
};

/// UDP socket listen address for media plane (audio + waterfall)
const LISTEN_ADDR: &str = "0.0.0.0:50000";

/// Audio packet size (samples per packet)
const PACKET_SAMPLES: usize = 480;

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

    println!("Media runtime listening on {}", socket.local_addr()?);

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

    let audio_stream = build_output_stream(
        &device,
        &config,
        Arc::clone(&jitter),
        Arc::clone(&stats_logger),
    )?;
    audio_stream.play()?;

    // --- UI buffers --------------------------------------------------------

    let waterfall_width = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
    let waterfall_height = HEIGHT - WATERFALL_TOP;

    let waterfall_buffer =
        Arc::new(Mutex::new(vec![0u32; waterfall_width * waterfall_height]));

    let spectrum_db = Arc::new(Mutex::new(vec![SPECTRUM_DB_MIN; WIDTH]));
    let media_stats = Arc::new(Mutex::new(MediaPacketStats::new()));

    // --- Control channel ---------------------------------------------------

    let (media_cmd_tx, mut media_cmd_rx) =
        mpsc::unbounded_channel::<MediaCommand>();

    // --- Shared state clones for thread -----------------------------------

    let jitter_for_thread = Arc::clone(&jitter);
    let waterfall_for_thread = Arc::clone(&waterfall_buffer);
    let spectrum_for_thread = Arc::clone(&spectrum_db);
    let stats_for_thread = Arc::clone(&media_stats);
    let stats_logger_for_thread = Arc::clone(&stats_logger);

    // --- Audio session generation (for radio switching) -------------------

    let audio_session_generation = Arc::new(AtomicU64::new(0));
    let audio_session_generation_for_thread =
        Arc::clone(&audio_session_generation);

    // --- Media thread ------------------------------------------------------

    thread::spawn(move || {
        let mut udp_buf = [0u8; 65536];

        let mut last_audio_session_generation =
            audio_session_generation_for_thread.load(Ordering::Relaxed);

        // let mut last_stats_log = Instant::now();  // Needed for periodic jitter logging

        loop {
            // --- Session change detection (radio switch) -------------------

            let current =
                audio_session_generation_for_thread.load(Ordering::Relaxed);

            if current != last_audio_session_generation {
                if let Ok(mut jb) = jitter.lock() {
                    jb.reset();
                }

                last_audio_session_generation = current;
                println!("[client] jitter buffer reset for new radio session");
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
                logger.maybe_log(
                    &mut media_stats,
                    jb.buffered_samples(),
                    OUTPUT_SAMPLE_RATE as f32,
                );
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
                            Ok(_) => println!("Sent UDP registration to {}", addr),
                            Err(e) => eprintln!(
                                "UDP registration failed to {}: {}",
                                addr, e
                            ),
                        }
                    }
                }
            }

            // --- Receive UDP packets --------------------------------------

            match socket.recv_from(&mut udp_buf) {
                Ok((len, src)) => {
                    // Registration ACK (4-byte control packet)
                    if len == 4 {
                        let magic =
                            u16::from_be_bytes([udp_buf[0], udp_buf[1]]);
                        let version = udp_buf[2];
                        let stream_type = udp_buf[3];

                        if magic == MAGIC
                            && version == VERSION
                            && stream_type == STREAM_TYPE_REGISTER_AUDIO
                        {
                            println!(
                                "Received UDP registration ACK from {}",
                                src
                            );
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
			);
                    }
                }

                // Normal non-blocking timeout
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}

                // Unexpected error
                Err(e) => {
                    eprintln!("UDP receive error: {}", e);
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
            selected = Some(
                cfg.with_sample_rate(cpal::SampleRate(OUTPUT_SAMPLE_RATE)),
            );
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

            if let Ok(mut logger) = stats_logger_for_audio.lock() {
                logger.add_audio_samples(data.len());
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
