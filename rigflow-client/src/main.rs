use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use rigflow_core::audio::jitter_buffer::JitterBuffer;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

mod net;
mod render;
mod app;

use crate::net::websocket::websocket_control_task;
use crate::net::udp::handle_media_packet;
use crate::app::state::UiState;

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
    SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1, SPECTRUM_PLOT_WIDTH,
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
    let mut mouse_was_down = false;

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

        handle_keyboard(&window, &ws_cmd_tx, &ui_state);
        handle_mouse_click_tune(&window, &ws_cmd_tx, &ui_state, &mut mouse_was_down);

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

fn handle_keyboard(
    window: &Window,
    ws_cmd_tx: &mpsc::UnboundedSender<ClientMessage>,
    ui_state: &Arc<Mutex<UiState>>,
) {
    let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);

    let target_step = if shift { 1_000.0 } else { 100.0 };
    let center_step = if shift { 10_000.0 } else { 1_000.0 };

    let state_snapshot = { ui_state.lock().unwrap().clone() };

    if window.is_key_pressed(Key::Left, KeyRepeat::Yes) {
        let new_freq = state_snapshot.target_freq_hz - target_step;
        let _ = ws_cmd_tx.send(ClientMessage::SetFrequency {
            target_freq_hz: new_freq,
        });
    }

    if window.is_key_pressed(Key::Right, KeyRepeat::Yes) {
        let new_freq = state_snapshot.target_freq_hz + target_step;
        let _ = ws_cmd_tx.send(ClientMessage::SetFrequency {
            target_freq_hz: new_freq,
        });
    }

    if window.is_key_pressed(Key::Up, KeyRepeat::Yes) {
        let new_center = state_snapshot.center_freq_hz + center_step;
        let _ = ws_cmd_tx.send(ClientMessage::SetCenterFrequency {
            center_freq_hz: new_center,
        });
    }

    if window.is_key_pressed(Key::Down, KeyRepeat::Yes) {
        let new_center = state_snapshot.center_freq_hz - center_step;
        let _ = ws_cmd_tx.send(ClientMessage::SetCenterFrequency {
            center_freq_hz: new_center,
        });
    }

    if window.is_key_pressed(Key::RightBracket, KeyRepeat::Yes) {
        let _ = ws_cmd_tx.send(ClientMessage::SetSsbPitch {
            pitch_hz: 500.0,
        });
    }

    if window.is_key_pressed(Key::LeftBracket, KeyRepeat::Yes) {
        let _ = ws_cmd_tx.send(ClientMessage::SetSsbPitch {
            pitch_hz: -500.0,
        });
    }

    if window.is_key_pressed(Key::L, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::SetSideband {
            sideband: "lsb".to_string(),
        });
    }

    if window.is_key_pressed(Key::U, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::SetSideband {
            sideband: "usb".to_string(),
        });
    }

    if window.is_key_pressed(Key::Key1, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::SetDemodMode {
            mode: "wfm".to_string(),
        });
    }

    if window.is_key_pressed(Key::Key2, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::SetDemodMode {
            mode: "usb".to_string(),
        });
    }

    if window.is_key_pressed(Key::Key3, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::SetDemodMode {
            mode: "lsb".to_string(),
        });
    }

    if window.is_key_pressed(Key::P, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::Ping);
    }
}

fn handle_mouse_click_tune(
    window: &Window,
    ws_cmd_tx: &mpsc::UnboundedSender<ClientMessage>,
    ui_state: &Arc<Mutex<UiState>>,
    mouse_was_down: &mut bool,
) {
    let mouse_down = window.get_mouse_down(MouseButton::Left);

    if mouse_down && !*mouse_was_down {
        if let Some((mx, _my)) = window.get_mouse_pos(MouseMode::Discard) {
            let state_snapshot = { ui_state.lock().unwrap().clone() };

	    if let Some(target_freq_hz) = x_to_frequency(mx, &state_snapshot) {
                let rounded = target_freq_hz.round();

                let _ = ws_cmd_tx.send(ClientMessage::SetFrequency {
                    target_freq_hz: rounded,
                });

                println!("click tune: x={:.1} -> target_freq_hz={:.0}", mx, rounded);
            }
        }
    }

    *mouse_was_down = mouse_down;
}

fn x_to_frequency(x: f32, state: &UiState) -> Option<f32> {
    if state.input_sample_rate_hz <= 0.0 || SPECTRUM_PLOT_WIDTH == 0 {
        return None;
    }

    let x0 = SPECTRUM_PLOT_X0 as f32;
    let x1 = SPECTRUM_PLOT_X1 as f32;

    if x < x0 || x > x1 {
        return None;
    }

    let frac = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    let offset_hz = (frac - 0.5) * state.input_sample_rate_hz;

    Some(state.center_freq_hz + offset_hz)
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

