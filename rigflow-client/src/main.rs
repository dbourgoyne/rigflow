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

use crate::net::websocket::websocket_control_task;
use crate::net::udp::handle_media_packet;

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
};

const LISTEN_ADDR: &str = "0.0.0.0:50000";
const SERVER_UDP_REGISTRATION_ADDR: &str = "192.168.0.225:9001";
const SERVER_WS_URL: &str = "ws://192.168.0.225:9000/ws";

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;

const PACKET_SAMPLES: usize = 480;
const TARGET_BUFFER_SAMPLES: usize = 4_800;
const MAX_BUFFER_SAMPLES: usize = 24_000;

const WIDTH: usize = 1024;
const HEIGHT: usize = 512;

const SEPARATOR_HEIGHT: usize = 8; //1;

const SPECTRUM_HEIGHT: usize = 196;

//const SPECTRUM_TOP_PAD: usize = 6;
const SPECTRUM_LEFT_PAD: usize = 0; //64;
const SPECTRUM_RIGHT_PAD: usize = 0; //8;
//const SPECTRUM_BOTTOM_PAD: usize = 32; //16;

const SPECTRUM_PLOT_X0: usize = SPECTRUM_LEFT_PAD;
//const SPECTRUM_PLOT_Y0: usize = SPECTRUM_TOP_PAD;
const SPECTRUM_PLOT_X1: usize = WIDTH - SPECTRUM_RIGHT_PAD;
//const SPECTRUM_PLOT_Y1: usize = SPECTRUM_HEIGHT - SPECTRUM_BOTTOM_PAD;

const SPECTRUM_PLOT_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
//const SPECTRUM_PLOT_HEIGHT: usize = SPECTRUM_PLOT_Y1 - SPECTRUM_PLOT_Y0;

const WATERFALL_TOP: usize = SPECTRUM_HEIGHT + SEPARATOR_HEIGHT;
//const WATERFALL_HEIGHT: usize = HEIGHT - WATERFALL_TOP;

const SPECTRUM_DB_MIN: f32 = -120.0;
const SPECTRUM_DB_MAX: f32 = 0.0;
//const SPECTRUM_SMOOTHING_ALPHA: f32 = 0.25;

//const COLOR_AXIS: u32 = 0x808080;
//const COLOR_LABEL: u32 = 0xC0C0C0;
//const COLOR_BLACK: u32 = 0x000000;
//const COLOR_GRID: u32 = 0x202020;
const COLOR_SEPARATOR: u32 = 0x404040;
//const COLOR_SPECTRUM: u32 = 0x00FF00;
//const COLOR_TUNING_MARKER: u32 = 0x00FF0000;

use rigflow_protocol::ClientMessage;

#[derive(Debug, Clone)]
struct UiState {
    center_freq_hz: f32,
    target_freq_hz: f32,
    sideband: String,
    demod_mode: String,
    ssb_pitch_hz: f32,
    input_sample_rate_hz: f32,
    waterfall_bins: usize,
    audio_sample_rate_hz: f32,
    audio_format: String,
    waterfall_frame_rate_hz: f32,
    status: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            sideband: "lsb".to_string(),
            demod_mode: "wfm".to_string(),
            ssb_pitch_hz: 0.0,
            input_sample_rate_hz: 0.0,
            waterfall_bins: WIDTH,
            audio_sample_rate_hz: OUTPUT_SAMPLE_RATE as f32,
            audio_format: "unknown".to_string(),
            waterfall_frame_rate_hz: 0.0,
            status: "starting".to_string(),
        }
    }
}

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

fn draw_tuning_marker(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    y_start: usize,
    state: &UiState,
) {
    if state.input_sample_rate_hz <= 0.0 || SPECTRUM_PLOT_WIDTH == 0 {
        return;
    }

    let offset_hz = state.target_freq_hz - state.center_freq_hz;
    let frac = offset_hz / state.input_sample_rate_hz + 0.5;
    let x = SPECTRUM_PLOT_X0 as f32 + frac * SPECTRUM_PLOT_WIDTH as f32;
    let x = x.round() as isize;

    if x < 0 || x >= width as isize {
        return;
    }

    let x = x as usize;
    for y in y_start..height {
        buffer[y * width + x] = 0x00FF0000;
    }
}

fn draw_separator(buffer: &mut [u32], width: usize, y: usize) {
    if y >= HEIGHT {
        return;
    }

    let row = &mut buffer[y * width..(y + 1) * width];
    row.fill(COLOR_SEPARATOR);
}

