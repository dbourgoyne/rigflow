use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use minifb::{Key, Window, WindowOptions};
use rigflow_core::audio::jitter_buffer::JitterBuffer;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

mod net;
mod render;
mod app;
mod input;

use crate::net::websocket::websocket_control_task;
use crate::net::udp::handle_media_packet;
use crate::app::state::UiState;
use crate::input::keyboard::{collect_keyboard_actions, UiAction};
use crate::input::mouse::collect_mouse_actions;

use rigflow_core::net::udp_framing::{
    MAGIC, VERSION,
//    STREAM_TYPE_AUDIO,
//    STREAM_TYPE_WATERFALL,
    STREAM_TYPE_REGISTER_AUDIO,
};

use crate::render::spectrum::{
    draw_spectrum_background,
    draw_spectrum_grid,
    draw_spectrum_trace,
    draw_spectrum_axes_and_labels,
    draw_frequency_overlay,
    draw_separator,
    draw_tuning_marker,
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
    SPECTRUM_HEIGHT, WATERFALL_TOP,
    SPECTRUM_DB_MIN, SPECTRUM_DB_MAX,
};

use rigflow_protocol::ClientMessage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        PACKET_SAMPLES,
        TARGET_BUFFER_SAMPLES,
        MAX_BUFFER_SAMPLES,
    )));

    let waterfall_buffer = Arc::new(Mutex::new(vec![0u32; WIDTH * HEIGHT]));
    let spectrum_db = Arc::new(Mutex::new(vec![SPECTRUM_DB_MIN; WIDTH]));
    let mut display_buffer = vec![0u32; WIDTH * HEIGHT];
    let ui_state = Arc::new(Mutex::new(UiState::default()));

    let stream = build_output_stream(Arc::clone(&jitter))?;
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
    let (ws_cmd_tx, ws_cmd_rx) = mpsc::unbounded_channel::<ClientMessage>();
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

	for action in collect_keyboard_actions(&window, &state_snapshot) {
	    let msg = ui_action_to_client_message(action);
	    let _ = ws_cmd_tx.send(msg);
	}

	for action in collect_mouse_actions(&window, &state_snapshot) {
	    let msg = ui_action_to_client_message(action);
	    let _ = ws_cmd_tx.send(msg);
	}

        {
            let buf = waterfall_buffer.lock().unwrap();
            display_buffer.copy_from_slice(&buf);
        }

        {
            let spectrum = spectrum_db.lock().unwrap().clone();
            draw_spectrum_background(&mut display_buffer, WIDTH, SPECTRUM_HEIGHT);
            draw_spectrum_grid(
                &mut display_buffer,
                WIDTH,
                SPECTRUM_HEIGHT,
                SPECTRUM_DB_MIN,
                SPECTRUM_DB_MAX,
            );
	    {
		let state = ui_state.lock().unwrap().clone();
		draw_spectrum_background(&mut display_buffer, WIDTH, SPECTRUM_HEIGHT);
		draw_spectrum_axes_and_labels(&mut display_buffer, WIDTH, &state);
	    }
	    draw_spectrum_trace(
		&mut display_buffer,
		WIDTH,
		&spectrum,
	    );
            draw_separator(&mut display_buffer, WIDTH, SPECTRUM_HEIGHT);
        }

        {
            let state = ui_state.lock().unwrap().clone();
            draw_tuning_marker(
                &mut display_buffer,
                WIDTH,
                HEIGHT,
                WATERFALL_TOP,
                &state,
            );
        }

	{
	    let state = ui_state.lock().unwrap().clone();

	    draw_spectrum_background(&mut display_buffer, WIDTH, SPECTRUM_HEIGHT);
	    draw_spectrum_axes_and_labels(&mut display_buffer, WIDTH, &state);

	    let spectrum = spectrum_db.lock().unwrap().clone();
	    draw_spectrum_trace(&mut display_buffer, WIDTH, &spectrum);

	    draw_frequency_overlay(
		&mut display_buffer,
		WIDTH,
		&state,
	    );

	}

        window.update_with_buffer(&display_buffer, WIDTH, HEIGHT)?;

        if last_title.elapsed() >= Duration::from_millis(200) {
            let state = ui_state.lock().unwrap().clone();
            window.set_title(&format!(
                "Rust Radio | Mode: {} | Ctr: {:.0} Hz | Tgt: {:.0} Hz | SB: {} | {} | {} Hz | {:.1} fps | {}",
                state.demod_mode.to_uppercase(),
                state.center_freq_hz,
                state.target_freq_hz,
                state.sideband.to_uppercase(),
                state.audio_format,
                state.audio_sample_rate_hz,
                state.waterfall_frame_rate_hz,
                state.status
            ));
            last_title = Instant::now();
        }

        if last_stats.elapsed() >= Duration::from_secs(1) {
            let jb = jitter.lock().unwrap();
            println!(
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
    jitter: Arc<Mutex<JitterBuffer>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .ok_or("No default output device available")?;

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

    let config = if let Some(cfg) = selected {
        cfg.config()
    } else {
        let default_cfg = device.default_output_config()?;
        let mut cfg: cpal::StreamConfig = default_cfg.config();
        cfg.channels = CHANNELS;
        cfg.sample_rate = cpal::SampleRate(OUTPUT_SAMPLE_RATE);
        cfg
    };

    println!(
        "Using output device: {} @ {} Hz",
        device.name().unwrap_or_else(|_| "<unknown>".to_string()),
        config.sample_rate.0
    );

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let mut jb = jitter.lock().unwrap();
            jb.pop_samples(data);
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn ui_action_to_client_message(action: UiAction) -> ClientMessage {
    match action {
        UiAction::SetTargetFrequency(target_freq_hz) => {
            ClientMessage::SetFrequency { target_freq_hz }
        }
        UiAction::SetCenterFrequency(center_freq_hz) => {
            ClientMessage::SetCenterFrequency { center_freq_hz }
        }
        UiAction::SetDemodMode(mode) => {
            ClientMessage::SetDemodMode {
                mode: mode.to_string(),
            }
        }
        UiAction::SetSideband(sideband) => {
            ClientMessage::SetSideband {
                sideband: sideband.to_string(),
            }
        }
        UiAction::SetSsbPitch(pitch_hz) => {
            ClientMessage::SetSsbPitch { pitch_hz }
        }
        UiAction::Ping => ClientMessage::Ping,
    }
}
