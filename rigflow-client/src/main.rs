use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use minifb::{Key, Window, WindowOptions};
use rigflow_core::audio::jitter_buffer::JitterBuffer;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

mod app;
mod input;
mod net;
mod render;
mod widgets;

use crate::net::websocket::websocket_control_task;
use crate::net::udp::handle_media_packet;
use crate::app::state::UiState;
use crate::input::keyboard::collect_keyboard_actions;
use crate::input::mouse::{collect_mouse_actions, collect_waterfall_wheel_actions};
use crate::render::frame::render_frame;
use crate::app::title::build_window_title;
use crate::input::keyboard::UiAction;
use crate::app::stats::ClientStatsLogger;
use crate::net::udp::MediaPacketStats;
use crate::app::actions::ui_action_to_control_command;

use rigflow_core::net::udp_framing::{
    MAGIC, VERSION,
//    STREAM_TYPE_AUDIO,
//    STREAM_TYPE_WATERFALL,
    STREAM_TYPE_REGISTER_AUDIO,
};

const LISTEN_ADDR: &str = "0.0.0.0:50000";
const SERVER_UDP_REGISTRATION_ADDR: &str = "192.168.0.225:9001";
const SERVER_WS_URL: &str = "ws://192.168.0.225:9000/ws";

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;

const PACKET_SAMPLES: usize = 480;
const TARGET_BUFFER_SAMPLES: usize = 4_800;
const MAX_BUFFER_SAMPLES: usize = 24_000;

use crate::app::layout::{
    HEIGHT, WIDTH,
    WATERFALL_TOP,
    SPECTRUM_DB_MIN
};

use rigflow_protocol::ClientMessage;
use crate::net::control::ControlCommand;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_default_env()
	.format_timestamp_millis()
	.init();

    let media_stats = Arc::new(Mutex::new(MediaPacketStats::new()));
    let client_stats_logger = Arc::new(Mutex::new(ClientStatsLogger::new()));
    
    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        PACKET_SAMPLES,
        TARGET_BUFFER_SAMPLES,
        MAX_BUFFER_SAMPLES,
    )));

    let waterfall_buffer = Arc::new(Mutex::new(vec![0u32; WIDTH * HEIGHT]));
    let spectrum_db = Arc::new(Mutex::new(vec![SPECTRUM_DB_MIN; WIDTH]));
    let mut display_buffer = vec![0u32; WIDTH * HEIGHT];
    let ui_state = Arc::new(Mutex::new(UiState::default()));

    let host = cpal::default_host();
    let device = host
	.default_output_device()
	.ok_or("No default output device available")?;
    let config = device.default_output_config()?.config();
    let stream = build_output_stream(
	&device,
	&config,
	Arc::clone(&jitter),
	Arc::clone(&client_stats_logger),
    )?;
    stream.play()?;

    let socket = UdpSocket::bind(LISTEN_ADDR)?;
    socket.set_read_timeout(Some(Duration::from_millis(5)))?;

    let mut reg = Vec::with_capacity(4);
    reg.extend_from_slice(&MAGIC.to_be_bytes());
    reg.push(VERSION);
    reg.push(STREAM_TYPE_REGISTER_AUDIO);
    socket.send_to(&reg, SERVER_UDP_REGISTRATION_ADDR)?;

    println!("Sent UDP registration to {}", SERVER_UDP_REGISTRATION_ADDR);
    println!("Listening on {}", LISTEN_ADDR);

    let rt = Runtime::new()?;
    let (ws_cmd_tx, ws_cmd_rx) = mpsc::unbounded_channel::<ControlCommand>();

    ws_cmd_tx
        .send(ControlCommand::Connect {
            server_ip: "192.168.0.225".to_string(),
        })
        .unwrap();

    let ui_state_for_ws = Arc::clone(&ui_state);

    rt.spawn(async move {
        if let Err(e) = websocket_control_task(SERVER_WS_URL, ws_cmd_rx, ui_state_for_ws).await {
            eprintln!("WebSocket control task failed: {e}");
        }
    });

    let mut window = Window::new(
        "Rust Radio Desktop Client",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )?;

    let mut udp_buf = [0u8; 65536];
    let mut last_stats = Instant::now();
    let mut last_title = Instant::now();

    while window.is_open() && !window.is_key_down(Key::Escape) {
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
			&jitter,
			&waterfall_buffer,
			&spectrum_db,
			&ui_state,
			&media_stats,
			WIDTH,
			HEIGHT,
			WATERFALL_TOP,
		    );
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }

	let state_snapshot = ui_state.lock().unwrap().clone();

	{
	    let mut state_mut = ui_state.lock().unwrap();
	    crate::input::mouse::update_center_freq_widget_hover(&window, &mut state_mut);
	    crate::input::mouse::update_zoom_slider(&window, &mut state_mut);
	}

	for action in collect_keyboard_actions(&window, &state_snapshot) {
	    match action {
		UiAction::CycleLicenseForward => {
		    let mut state = ui_state.lock().unwrap();
		    state.selected_license = crate::app::om_bands::next_license(state.selected_license);
		}
		
		UiAction::CycleLicenseBackward => {
		    let mut state = ui_state.lock().unwrap();
		    state.selected_license = crate::app::om_bands::prev_license(state.selected_license);
		}

		other => {
		    let server_ip = {
			let state = ui_state.lock().unwrap();
			state.rigflow_server_ip.clone()
		    };
		    if let Some(msg) = ui_action_to_control_command(other, &server_ip) {
			let _ = ws_cmd_tx.send(msg);
		    }
		}
	    }
	}

	for action in collect_mouse_actions(&window, &state_snapshot) {
	    let server_ip = {
                let state = ui_state.lock().unwrap();
                state.rigflow_server_ip.clone()
            };
            if let Some(msg) = ui_action_to_control_command(action, &server_ip) {
                let _ = ws_cmd_tx.send(msg);
            }
	}

	for action in crate::input::mouse::collect_center_freq_widget_actions(&window, &state_snapshot) {
	    let server_ip = {
                let state = ui_state.lock().unwrap();
                state.rigflow_server_ip.clone()
            };
            if let Some(msg) = ui_action_to_control_command(action, &server_ip) {
                let _ = ws_cmd_tx.send(msg);
            }
	}

	for action in collect_waterfall_wheel_actions(&window, &state_snapshot) {
	    let server_ip = {
                let state = ui_state.lock().unwrap();
                state.rigflow_server_ip.clone()
            };
            if let Some(msg) = ui_action_to_control_command(action, &server_ip) {
                let _ = ws_cmd_tx.send(msg);
            }
	}

        {
            let buf = waterfall_buffer.lock().unwrap();
            display_buffer.copy_from_slice(&buf);
        }

	let spectrum_snapshot = spectrum_db.lock().unwrap().clone();
	let waterfall_snapshot = waterfall_buffer.lock().unwrap().clone();

	render_frame(
	    &mut display_buffer,
	    &waterfall_snapshot,
	    &spectrum_snapshot,
	    &state_snapshot,
	);

        window.update_with_buffer(&display_buffer, WIDTH, HEIGHT)?;

        if last_title.elapsed() >= Duration::from_millis(200) {
	    window.set_title(&build_window_title(&state_snapshot));
            last_title = Instant::now();
        }

	let jitter_buffer_samples = {
	    let jb = jitter.lock().unwrap();
	    jb.buffered_samples()
	};

	if let Ok(mut logger) = client_stats_logger.lock()
	    && let Ok(mut stats) = media_stats.lock() {
		logger.maybe_log(
		    &mut stats,
		    jitter_buffer_samples,
		    state_snapshot.audio_sample_rate_hz,
		);
	    }

        if last_stats.elapsed() >= Duration::from_secs(1) {
            let jb = jitter.lock().unwrap();
            log::debug!(
                "started={} buffered_samples={} rx={} inserted={} concealed={} late_drop={} overflow_drop={}",
                jb.started(),
                jb.buffered_samples(),
                jb.packets_received,
                jb.packets_inserted,
                jb.packets_missing_concealed,
                jb.packets_dropped_late,
                jb.packets_dropped_overflow,
            );
            last_stats = Instant::now();
        }
    }

    Ok(())
}

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    jitter: Arc<Mutex<JitterBuffer>>,
    client_stats_logger: Arc<Mutex<ClientStatsLogger>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let supported_configs = device.supported_output_configs()?;

    let mut selected = None;

    let jitter_for_audio = jitter.clone();
    let stats_for_audio = client_stats_logger.clone();

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

    let stream = device.build_output_stream(
        &selected_config,
        move |data: &mut [f32], _| {
            let delivered = data.len();

            if let Ok(mut jb) = jitter_for_audio.lock() {
                jb.pop_samples(data);
            } else {
                for s in data.iter_mut() {
                    *s = 0.0;
                }
            }

            if let Ok(mut stats) = stats_for_audio.lock() {
                stats.add_audio_samples(delivered);
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
