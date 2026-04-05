use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use tokio::sync::mpsc;

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{MAGIC, STREAM_TYPE_REGISTER_AUDIO, VERSION},
};

use crate::{
    app::{
        layout::{HEIGHT, SPECTRUM_DB_MIN, WATERFALL_TOP, WIDTH},
        state::UiState,
    },
    net::udp::{handle_media_packet, MediaPacketStats},
};

const LISTEN_ADDR: &str = "0.0.0.0:50000";

const PACKET_SAMPLES: usize = 480;
const TARGET_BUFFER_SAMPLES: usize = 4_800;
const MAX_BUFFER_SAMPLES: usize = 24_000;

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;

#[derive(Debug, Clone)]
pub enum MediaCommand {
    RegisterUdp {
        server_ip: String,
        server_udp_port: u16,
    },
}

pub struct MediaRuntimeHandles {
    pub media_cmd_tx: mpsc::UnboundedSender<MediaCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    pub _audio_stream: cpal::Stream,
}

pub fn start_media_runtime(
    ui_state: Arc<Mutex<UiState>>,
) -> Result<MediaRuntimeHandles, Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(LISTEN_ADDR)?;
    socket.set_read_timeout(Some(Duration::from_millis(5)))?;

    let udp_listen_port = socket.local_addr()?.port();

    {
        let mut state = ui_state.lock().unwrap();
        state.udp_listen_port = udp_listen_port;
    }

    println!("Media runtime listening on {}", socket.local_addr()?);

    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        PACKET_SAMPLES,
        TARGET_BUFFER_SAMPLES,
        MAX_BUFFER_SAMPLES,
    )));

    let host = cpal::default_host();
    let device = host
	.default_output_device()
	.ok_or("No default output device available")?;
    let config = device.default_output_config()?.config();

    let audio_stream = build_output_stream(&device, &config, Arc::clone(&jitter))?;
    audio_stream.play()?;

    let waterfall_buffer = Arc::new(Mutex::new(vec![0u32; WIDTH * HEIGHT]));
    let spectrum_db = Arc::new(Mutex::new(vec![SPECTRUM_DB_MIN; WIDTH]));
    let media_stats = Arc::new(Mutex::new(MediaPacketStats::new()));

    let (media_cmd_tx, mut media_cmd_rx) = mpsc::unbounded_channel::<MediaCommand>();

    let ui_state_for_thread = Arc::clone(&ui_state);
    let jitter_for_thread = Arc::clone(&jitter);
    let waterfall_for_thread = Arc::clone(&waterfall_buffer);
    let spectrum_for_thread = Arc::clone(&spectrum_db);
    let stats_for_thread = Arc::clone(&media_stats);

    thread::spawn(move || {
        let mut udp_buf = [0u8; 65536];

        loop {
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
                            Ok(_) => {
                                println!("Sent UDP registration to {}", addr);
                            }
                            Err(e) => {
                                eprintln!("UDP registration failed to {}: {}", addr, e);
                            }
                        }
                    }
                }
            }

            match socket.recv_from(&mut udp_buf) {
                Ok((len, src)) => {
                    if len == 4 {
                        let magic = u16::from_be_bytes([udp_buf[0], udp_buf[1]]);
                        let version = udp_buf[2];
                        let stream_type = udp_buf[3];

                        if magic == MAGIC
                            && version == VERSION
                            && stream_type == STREAM_TYPE_REGISTER_AUDIO
                        {
                            println!("Received UDP registration ACK from {}", src);
                        }
                    } else if len >= 16 {
                        handle_media_packet(
                            &udp_buf[..len],
                            &jitter_for_thread,
                            &waterfall_for_thread,
                            &spectrum_for_thread,
                            &ui_state_for_thread,
                            &stats_for_thread,
                            WIDTH,
                            HEIGHT,
                            WATERFALL_TOP,
                        );
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    eprintln!("UDP receive error: {}", e);
                }
            }
        }
    });

    Ok(MediaRuntimeHandles {
        media_cmd_tx,
        waterfall_buffer,
        spectrum_db,
	_audio_stream: audio_stream,
    })
}

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    jitter: Arc<Mutex<JitterBuffer>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let supported_configs = device.supported_output_configs()?;

    let mut selected = None;

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

    let stream = device.build_output_stream(
        &selected_config,
        move |data: &mut [f32], _| {
            if let Ok(mut jb) = jitter_for_audio.lock() {
                jb.pop_samples(data);
            } else {
                for s in data.iter_mut() {
                    *s = 0.0;
                }
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
