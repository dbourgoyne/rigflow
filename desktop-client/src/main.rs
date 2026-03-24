use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use radio_server::audio_client::jitter_buffer::JitterBuffer;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

const MAGIC: u16 = 0x5253;
const VERSION: u8 = 1;

const STREAM_TYPE_AUDIO: u8 = 1;
const STREAM_TYPE_WATERFALL: u8 = 2;
const STREAM_TYPE_REGISTER_AUDIO: u8 = 10;

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
const SPECTRUM_LEFT_PAD: usize = 0; //64;
const SPECTRUM_RIGHT_PAD: usize = 0; //8;
const SPECTRUM_TOP_PAD: usize = 6;
const SPECTRUM_BOTTOM_PAD: usize = 32; //16;

const SPECTRUM_PLOT_X0: usize = SPECTRUM_LEFT_PAD;
const SPECTRUM_PLOT_Y0: usize = SPECTRUM_TOP_PAD;
const SPECTRUM_PLOT_X1: usize = WIDTH - SPECTRUM_RIGHT_PAD;
const SPECTRUM_PLOT_Y1: usize = SPECTRUM_HEIGHT - SPECTRUM_BOTTOM_PAD;

const SPECTRUM_PLOT_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
const SPECTRUM_PLOT_HEIGHT: usize = SPECTRUM_PLOT_Y1 - SPECTRUM_PLOT_Y0;

const WATERFALL_TOP: usize = SPECTRUM_HEIGHT + SEPARATOR_HEIGHT;
//const WATERFALL_HEIGHT: usize = HEIGHT - WATERFALL_TOP;

const SPECTRUM_DB_MIN: f32 = -120.0;
const SPECTRUM_DB_MAX: f32 = 0.0;
const SPECTRUM_SMOOTHING_ALPHA: f32 = 0.25;

const COLOR_AXIS: u32 = 0x808080;
const COLOR_LABEL: u32 = 0xC0C0C0;
const COLOR_BLACK: u32 = 0x000000;
const COLOR_GRID: u32 = 0x202020;
const COLOR_SEPARATOR: u32 = 0x404040;
const COLOR_SPECTRUM: u32 = 0x00FF00;
//const COLOR_TUNING_MARKER: u32 = 0x00FF0000;

use radio_protocol::{ClientMessage, ServerMessage};

#[derive(Debug, Clone)]
struct UiState {
    center_freq_hz: f32,
    target_freq_hz: f32,
    sideband: String,
    demod_mode: String,
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
                    handle_media_packet(&udp_buf[..len], &jitter, &waterfall_buffer, &spectrum_db);
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

async fn websocket_control_task(
    ws_url: &str,
    mut rx: mpsc::UnboundedReceiver<ClientMessage>,
    ui_state: Arc<Mutex<UiState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    {
        let mut state = ui_state.lock().unwrap();
        state.status = "ws connected".to_string();
    }

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let text = serde_json::to_string(&cmd)?;
                        write.send(tokio_tungstenite::tungstenite::Message::Text(text.into())).await?;
                    }
                    None => break,
                }
            }

            msg = read.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
			println!("ws rx: {}", text);
                        if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                            apply_server_message(server_msg, &ui_state);
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(Box::new(e)),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

fn apply_server_message(msg: ServerMessage, ui_state: &Arc<Mutex<UiState>>) {
    let mut state = ui_state.lock().unwrap();

    match msg {
        ServerMessage::Ready => {
            state.status = "ready".to_string();
        }
        ServerMessage::Pong => {
            state.status = "pong".to_string();
        }
        ServerMessage::FrequencyChanged { target_freq_hz } => {
            state.target_freq_hz = target_freq_hz;
        }
        ServerMessage::CenterFrequencyChanged { center_freq_hz } => {
            state.center_freq_hz = center_freq_hz;
        }
        ServerMessage::SidebandChanged { sideband } => {
            state.sideband = sideband;
        }
        ServerMessage::DemodModeChanged { mode } => {
            state.demod_mode = mode;
        }
	ServerMessage::StreamConfig {
	    audio_sample_rate_hz,
	    audio_format,
	    waterfall_bins,
	    waterfall_frame_rate_hz,
	    center_freq_hz,
	    target_freq_hz,
	    input_sample_rate_hz,
	} => {
	    state.audio_sample_rate_hz = audio_sample_rate_hz;
	    state.audio_format = audio_format;
	    state.waterfall_bins = waterfall_bins;
	    state.waterfall_frame_rate_hz = waterfall_frame_rate_hz;
	    state.center_freq_hz = center_freq_hz;
	    state.target_freq_hz = target_freq_hz;
	    state.input_sample_rate_hz = input_sample_rate_hz;
	    state.status = "stream configured".to_string();
	}
        ServerMessage::UdpAudioOffer { server_udp_port } => {
            state.status = format!("udp port {}", server_udp_port);
        }
        ServerMessage::Info { message } => {
            state.status = message;
        }
        ServerMessage::Error { message } => {
            state.status = format!("error: {}", message);
        }
    }
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

fn handle_media_packet(
    packet: &[u8],
    jitter: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
) {
    if packet.len() < 16 {
        return;
    }

    let magic = u16::from_be_bytes([packet[0], packet[1]]);
    let version = packet[2];
    let stream_type = packet[3];
    let sequence = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let _timestamp = u64::from_be_bytes([
        packet[8], packet[9], packet[10], packet[11],
        packet[12], packet[13], packet[14], packet[15],
    ]);

    if magic != MAGIC || version != VERSION {
        return;
    }

    match stream_type {
        STREAM_TYPE_AUDIO => {
            let payload = &packet[16..];
            let mut samples = Vec::with_capacity(payload.len() / 2);

            for chunk in payload.chunks_exact(2) {
                let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                samples.push(s as f32 / 32768.0);
            }

            let mut jb = jitter.lock().unwrap();
            jb.push_packet(sequence, samples);
        }

        STREAM_TYPE_WATERFALL => {
            if packet.len() < 18 {
                return;
            }

            let bin_count = u16::from_be_bytes([packet[16], packet[17]]) as usize;
            let payload = &packet[18..];

            if payload.len() < bin_count {
                return;
            }

            let row = &payload[..bin_count];

            {
                let mut spectrum = spectrum_db.lock().unwrap();
                update_spectrum_db(&mut spectrum, row);
            }

            {
                let mut buffer = waterfall_buffer.lock().unwrap();
                draw_row(&mut buffer, row, WIDTH, HEIGHT);
            }
        }

        _ => {}
    }
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

fn update_spectrum_db(spectrum: &mut Vec<f32>, row: &[u8]) {
    if row.is_empty() {
        return;
    }

    if spectrum.len() != row.len() {
        spectrum.clear();
        spectrum.reserve(row.len());
        for &v in row {
            spectrum.push(byte_to_relative_db(v));
        }
        return;
    }

    for (dst, &src) in spectrum.iter_mut().zip(row.iter()) {
        let new_db = byte_to_relative_db(src);
        *dst = (1.0 - SPECTRUM_SMOOTHING_ALPHA) * *dst + SPECTRUM_SMOOTHING_ALPHA * new_db;
    }
}

fn byte_to_relative_db(v: u8) -> f32 {
    SPECTRUM_DB_MIN + (v as f32 / 255.0) * (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)
}

fn draw_row(buffer: &mut [u32], row: &[u8], width: usize, _height: usize) {
    if row.is_empty() {
        return;
    }

    for y in (WATERFALL_TOP + 1..HEIGHT).rev() {
        let dst = y * width + SPECTRUM_PLOT_X0;
        let src = (y - 1) * width + SPECTRUM_PLOT_X0;
        let len = SPECTRUM_PLOT_WIDTH;
        buffer.copy_within(src..src + len, dst);
    }

    let top = &mut buffer[WATERFALL_TOP * width..(WATERFALL_TOP + 1) * width];

    for x in 0..width {
        if x < SPECTRUM_PLOT_X0 || x >= SPECTRUM_PLOT_X1 {
            top[x] = 0x000000;
        } else {
            let plot_x = x - SPECTRUM_PLOT_X0;
            let src_x = plot_x * row.len() / SPECTRUM_PLOT_WIDTH;
            top[x] = color_map(row[src_x.min(row.len() - 1)]);
        }
    }
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


fn draw_spectrum_background(buffer: &mut [u32], width: usize, height: usize) {
    for y in 0..height {
        let row = &mut buffer[y * width..(y + 1) * width];
        row.fill(COLOR_BLACK);
    }
}

fn draw_separator(buffer: &mut [u32], width: usize, y: usize) {
    if y >= HEIGHT {
        return;
    }

    let row = &mut buffer[y * width..(y + 1) * width];
    row.fill(COLOR_SEPARATOR);
}

fn draw_spectrum_grid(
    buffer: &mut [u32],
    width: usize,
    plot_height: usize,
    db_min: f32,
    db_max: f32,
) {
    let marks = [-120.0, -100.0, -80.0, -60.0, -40.0, -20.0, 0.0];

    for &db in &marks {
        if db < db_min || db > db_max {
            continue;
        }

        let y = db_to_y(db, db_min, db_max, plot_height);
        if y >= plot_height {
            continue;
        }

        for x in 0..width {
            buffer[y * width + x] = COLOR_GRID;
        }
    }
}

fn draw_spectrum_trace(
    buffer: &mut [u32],
    width: usize,
    spectrum_db: &[f32],
) {
    if spectrum_db.len() < 2 || SPECTRUM_PLOT_WIDTH < 2 {
        return;
    }

    let mut prev_x = SPECTRUM_PLOT_X0 as i32;
    let mut prev_y = db_to_plot_y(spectrum_db[0]) as i32;

    for plot_x in 1..SPECTRUM_PLOT_WIDTH {
        let bin = plot_x * spectrum_db.len() / SPECTRUM_PLOT_WIDTH;
        let bin = bin.min(spectrum_db.len() - 1);

        let x = (SPECTRUM_PLOT_X0 + plot_x) as i32;
        let y = db_to_plot_y(spectrum_db[bin]) as i32;

        draw_line(buffer, width, prev_x, prev_y, x, y, COLOR_SPECTRUM);

        prev_x = x;
        prev_y = y;
    }
}


fn db_to_y(db: f32, db_min: f32, db_max: f32, height: usize) -> usize {
    let t = ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0);
    (height - 1).saturating_sub((t * (height as f32 - 1.0)) as usize)
}

fn draw_line(
    buffer: &mut [u32],
    fb_width: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        put_pixel(buffer, fb_width, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn put_pixel(buffer: &mut [u32], fb_width: usize, x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 {
        return;
    }

    let x = x as usize;
    let y = y as usize;

    if x >= fb_width || y >= HEIGHT {
        return;
    }

    let idx = y * fb_width + x;
    if idx < buffer.len() {
        buffer[idx] = color;
    }
}

fn color_map(v: u8) -> u32 {
    let x = v as f32 / 255.0;

    let (r, g, b) = if x < 0.25 {
        let t = x / 0.25;
        (0.0, 0.0, 255.0 * t)
    } else if x < 0.5 {
        let t = (x - 0.25) / 0.25;
        (0.0, 255.0 * t, 255.0)
    } else if x < 0.75 {
        let t = (x - 0.5) / 0.25;
        (255.0 * t, 255.0, 255.0 * (1.0 - t))
    } else {
        let t = (x - 0.75) / 0.25;
        (255.0, 255.0 * (1.0 - t), 0.0)
    };

    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn glyph_rows(c: char) -> [u8; 7] {
    match c {
        ' ' => [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b00000],
        '!' => [0b00100,0b00100,0b00100,0b00100,0b00100,0b00000,0b00100],
        '"' => [0b01010,0b01010,0b01010,0b00000,0b00000,0b00000,0b00000],
        '#' => [0b01010,0b11111,0b01010,0b01010,0b11111,0b01010,0b00000],
        '$' => [0b00100,0b01111,0b10100,0b01110,0b00101,0b11110,0b00100],
        '%' => [0b11001,0b11010,0b00100,0b01000,0b10110,0b00110,0b00000],
        '&' => [0b01100,0b10010,0b10100,0b01000,0b10101,0b10010,0b01101],
        '\'' => [0b00100,0b00100,0b01000,0b00000,0b00000,0b00000,0b00000],
        '(' => [0b00010,0b00100,0b01000,0b01000,0b01000,0b00100,0b00010],
        ')' => [0b01000,0b00100,0b00010,0b00010,0b00010,0b00100,0b01000],
        '*' => [0b00000,0b00100,0b10101,0b01110,0b10101,0b00100,0b00000],
        '+' => [0b00000,0b00100,0b00100,0b11111,0b00100,0b00100,0b00000],
        ',' => [0b00000,0b00000,0b00000,0b00000,0b00110,0b00100,0b01000],
        '-' => [0b00000,0b00000,0b00000,0b11111,0b00000,0b00000,0b00000],
        '.' => [0b00000,0b00000,0b00000,0b00000,0b00000,0b01100,0b01100],
        '/' => [0b00001,0b00010,0b00100,0b01000,0b10000,0b00000,0b00000],

        '0' => [0b01110,0b10001,0b10011,0b10101,0b11001,0b10001,0b01110],
        '1' => [0b00100,0b01100,0b00100,0b00100,0b00100,0b00100,0b01110],
        '2' => [0b01110,0b10001,0b00001,0b00010,0b00100,0b01000,0b11111],
        '3' => [0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
        '4' => [0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00010],
        '5' => [0b11111,0b10000,0b10000,0b11110,0b00001,0b00001,0b11110],
        '6' => [0b00110,0b01000,0b10000,0b11110,0b10001,0b10001,0b01110],
        '7' => [0b11111,0b00001,0b00010,0b00100,0b01000,0b01000,0b01000],
        '8' => [0b01110,0b10001,0b10001,0b01110,0b10001,0b10001,0b01110],
        '9' => [0b01110,0b10001,0b10001,0b01111,0b00001,0b00010,0b11100],

        ':' => [0b00000,0b01100,0b01100,0b00000,0b01100,0b01100,0b00000],
        ';' => [0b00000,0b01100,0b01100,0b00000,0b01100,0b00100,0b01000],
        '<' => [0b00010,0b00100,0b01000,0b10000,0b01000,0b00100,0b00010],
        '=' => [0b00000,0b11111,0b00000,0b11111,0b00000,0b00000,0b00000],
        '>' => [0b01000,0b00100,0b00010,0b00001,0b00010,0b00100,0b01000],
        '?' => [0b01110,0b10001,0b00001,0b00010,0b00100,0b00000,0b00100],
        '@' => [0b01110,0b10001,0b00001,0b01101,0b10101,0b10101,0b01110],

        'A' => [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'B' => [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110],
        'C' => [0b01110,0b10001,0b10000,0b10000,0b10000,0b10001,0b01110],
        'D' => [0b11100,0b10010,0b10001,0b10001,0b10001,0b10010,0b11100],
        'E' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111],
        'F' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b10000],
        'G' => [0b01110,0b10001,0b10000,0b10111,0b10001,0b10001,0b01110],
        'H' => [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'I' => [0b01110,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110],
        'J' => [0b00001,0b00001,0b00001,0b00001,0b10001,0b10001,0b01110],
        'K' => [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001],
        'L' => [0b10000,0b10000,0b10000,0b10000,0b10000,0b10000,0b11111],
        'M' => [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001],
        'N' => [0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b10001],
        'O' => [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'P' => [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000],
        'Q' => [0b01110,0b10001,0b10001,0b10001,0b10101,0b10010,0b01101],
        'R' => [0b11110,0b10001,0b10001,0b11110,0b10100,0b10010,0b10001],
        'S' => [0b01111,0b10000,0b10000,0b01110,0b00001,0b00001,0b11110],
        'T' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
        'U' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'V' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b01010,0b00100],
        'W' => [0b10001,0b10001,0b10001,0b10101,0b10101,0b10101,0b01010],
        'X' => [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001],
        'Y' => [0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00100],
        'Z' => [0b11111,0b00001,0b00010,0b00100,0b01000,0b10000,0b11111],

        '[' => [0b01110,0b01000,0b01000,0b01000,0b01000,0b01000,0b01110],
        '\\' => [0b10000,0b01000,0b00100,0b00010,0b00001,0b00000,0b00000],
        ']' => [0b01110,0b00010,0b00010,0b00010,0b00010,0b00010,0b01110],
        '^' => [0b00100,0b01010,0b10001,0b00000,0b00000,0b00000,0b00000],
        '_' => [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b11111],
        '`' => [0b01000,0b00100,0b00010,0b00000,0b00000,0b00000,0b00000],

        'a' => [0b00000,0b00000,0b01110,0b00001,0b01111,0b10001,0b01111],
        'b' => [0b10000,0b10000,0b10110,0b11001,0b10001,0b10001,0b11110],
        'c' => [0b00000,0b00000,0b01110,0b10001,0b10000,0b10001,0b01110],
        'd' => [0b00001,0b00001,0b01101,0b10011,0b10001,0b10001,0b01111],
        'e' => [0b00000,0b00000,0b01110,0b10001,0b11111,0b10000,0b01110],
        'f' => [0b00110,0b01001,0b01000,0b11100,0b01000,0b01000,0b01000],
        'g' => [0b00000,0b01111,0b10001,0b10001,0b01111,0b00001,0b01110],
        'h' => [0b10000,0b10000,0b10110,0b11001,0b10001,0b10001,0b10001],
        'i' => [0b00100,0b00000,0b01100,0b00100,0b00100,0b00100,0b01110],
        'j' => [0b00010,0b00000,0b00110,0b00010,0b00010,0b10010,0b01100],
        'k' => [0b10000,0b10000,0b10010,0b10100,0b11000,0b10100,0b10010],
        'l' => [0b01100,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110],
        'm' => [0b00000,0b00000,0b11010,0b10101,0b10101,0b10101,0b10101],
        'n' => [0b00000,0b00000,0b10110,0b11001,0b10001,0b10001,0b10001],
        'o' => [0b00000,0b00000,0b01110,0b10001,0b10001,0b10001,0b01110],
        'p' => [0b00000,0b11110,0b10001,0b10001,0b11110,0b10000,0b10000],
        'q' => [0b00000,0b01101,0b10011,0b10001,0b01111,0b00001,0b00001],
        'r' => [0b00000,0b00000,0b10110,0b11001,0b10000,0b10000,0b10000],
        's' => [0b00000,0b00000,0b01111,0b10000,0b01110,0b00001,0b11110],
        't' => [0b01000,0b01000,0b11100,0b01000,0b01000,0b01001,0b00110],
        'u' => [0b00000,0b00000,0b10001,0b10001,0b10001,0b10011,0b01101],
        'v' => [0b00000,0b00000,0b10001,0b10001,0b10001,0b01010,0b00100],
        'w' => [0b00000,0b00000,0b10001,0b10001,0b10101,0b10101,0b01010],
        'x' => [0b00000,0b00000,0b10001,0b01010,0b00100,0b01010,0b10001],
        'y' => [0b00000,0b10001,0b10001,0b10001,0b01111,0b00001,0b01110],
        'z' => [0b00000,0b00000,0b11111,0b00010,0b00100,0b01000,0b11111],

        '{' => [0b00010,0b00100,0b00100,0b01000,0b00100,0b00100,0b00010],
        '|' => [0b00100,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
        '}' => [0b01000,0b00100,0b00100,0b00010,0b00100,0b00100,0b01000],
        '~' => [0b00000,0b00000,0b01001,0b10110,0b00000,0b00000,0b00000],

        _ => [0b01110,0b10001,0b00010,0b00100,0b00100,0b00000,0b00100], // '?'
    }
}

fn draw_char(buffer: &mut [u32], fb_width: usize, x: usize, y: usize, c: char, color: u32) {
    let rows = glyph_rows(c);

    for (dy, row) in rows.iter().enumerate() {
        for dx in 0..5 {
            if (row & (1 << (4 - dx))) != 0 {
                let px = x + dx;
                let py = y + dy;
                if px < fb_width && py < HEIGHT {
                    buffer[py * fb_width + px] = color;
                }
            }
        }
    }
}

fn draw_text(buffer: &mut [u32], fb_width: usize, x: usize, y: usize, text: &str, color: u32) {
    let mut cx = x;
    for ch in text.chars() {
        draw_char(buffer, fb_width, cx, y, ch, color);
        cx += 6; // 5px glyph + 1px spacing
    }
}

/*
fn draw_char_2x(buffer: &mut [u32], fb_width: usize, x: usize, y: usize, c: char, color: u32) {
    let rows = glyph_rows(c);

    for (dy, row) in rows.iter().enumerate() {
        for dx in 0..5 {
            if (row & (1 << (4 - dx))) != 0 {
                let px = x + dx * 2;
                let py = y + dy * 2;

                for oy in 0..2 {
                    for ox in 0..2 {
                        let sx = px + ox;
                        let sy = py + oy;
                        if sx < fb_width && sy < HEIGHT {
                            buffer[sy * fb_width + sx] = color;
                        }
                    }
                }
            }
        }
    }
}

fn draw_text_2x(buffer: &mut [u32], fb_width: usize, x: usize, y: usize, text: &str, color: u32) {
    let mut cx = x;
    for ch in text.chars() {
        draw_char_2x(buffer, fb_width, cx, y, ch, color);
        cx += 12; // (5*2) + 2 spacing
    }
}
*/

fn format_freq_label(freq_hz: f32) -> String {
    if freq_hz.abs() >= 1_000_000.0 {
        format!("{:.3}M", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.1}k", freq_hz / 1_000.0)
    } else {
        format!("{:.0}", freq_hz)
    }
}

fn draw_spectrum_axes_and_labels(
    buffer: &mut [u32],
    width: usize,
    state: &UiState,
) {
    for y in SPECTRUM_PLOT_Y0..=SPECTRUM_PLOT_Y1 {
        buffer[y * width + SPECTRUM_PLOT_X0] = COLOR_AXIS;
    }

    for x in SPECTRUM_PLOT_X0..=SPECTRUM_PLOT_X1 {
        buffer[SPECTRUM_PLOT_Y1 * width + x] = COLOR_AXIS;
    }

    let db_ticks = [-120.0, -100.0, -80.0, -60.0, -40.0, -20.0, 0.0];
    for db in db_ticks {
        let y = db_to_plot_y(db);
        for x in SPECTRUM_PLOT_X0..SPECTRUM_PLOT_X1 {
            buffer[y * width + x] = COLOR_GRID;
        }

        let label = format!("{:.0}", db);
        let label_x = 4;
        let label_y = y.saturating_sub(3);
        draw_text(buffer, width, label_x, label_y, &label, COLOR_LABEL);
    }

    if state.input_sample_rate_hz > 0.0 {
        let left = state.center_freq_hz - 0.5 * state.input_sample_rate_hz;
        let center = state.center_freq_hz;
        let right = state.center_freq_hz + 0.5 * state.input_sample_rate_hz;

        let ticks = [
            (SPECTRUM_PLOT_X0, format_freq_label(left)),
            (SPECTRUM_PLOT_X0 + SPECTRUM_PLOT_WIDTH / 4, format_freq_label(left + 0.25 * state.input_sample_rate_hz)),
            (SPECTRUM_PLOT_X0 + SPECTRUM_PLOT_WIDTH / 2, format_freq_label(center)),
            (SPECTRUM_PLOT_X0 + 3 * SPECTRUM_PLOT_WIDTH / 4, format_freq_label(left + 0.75 * state.input_sample_rate_hz)),
            (SPECTRUM_PLOT_X1, format_freq_label(right)),
        ];

        for (x, label) in ticks {
            for y in SPECTRUM_PLOT_Y0..=SPECTRUM_PLOT_Y1 {
                if y % 4 == 0 {
                    buffer[y * width + x] = COLOR_GRID;
                }
            }

            let label_w = label.len() * 6;
            let label_x = x.saturating_sub(label_w / 2).min(width.saturating_sub(label_w));
            let label_y = SPECTRUM_PLOT_Y1 + 16;
            draw_text(buffer, width, label_x, label_y, &label, COLOR_LABEL);
        }
    }

    draw_text(buffer, width, 4, 2, "dB", COLOR_LABEL);
    draw_text(buffer, width, SPECTRUM_PLOT_X1.saturating_sub(14), 2, "Hz", COLOR_LABEL);
}

fn db_to_plot_y(db: f32) -> usize {
    let t = ((db - SPECTRUM_DB_MIN) / (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)).clamp(0.0, 1.0);
    SPECTRUM_PLOT_Y1 - (t * SPECTRUM_PLOT_HEIGHT as f32) as usize
}

fn format_freq_hz(freq_hz: f32) -> String {
    if freq_hz.abs() >= 1_000_000.0 {
        format!("{:.3} MHz", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.3} kHz", freq_hz / 1_000.0)
    } else {
        format!("{:.0} Hz", freq_hz)
    }
}

fn freq_to_plot_x(freq_hz: f32, state: &UiState) -> Option<usize> {
    if state.input_sample_rate_hz <= 0.0 || SPECTRUM_PLOT_WIDTH == 0 {
        return None;
    }

    let frac =
        ((freq_hz - state.center_freq_hz) / state.input_sample_rate_hz + 0.5).clamp(0.0, 1.0);

    Some(SPECTRUM_PLOT_X0 + (frac * SPECTRUM_PLOT_WIDTH as f32).round() as usize)
}

fn draw_frequency_overlay(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    const CF_COLOR: u32 = 0x00FFFF00;
    const TF_COLOR: u32 = 0x00FFA500;

    // 2x text metrics for the 5x7 font:
    // width = 5*2 = 10 px, spacing = 2 px, so each char advances 12 px
    const CHAR_ADVANCE_2X: usize = 12;
    const TEXT_HEIGHT_2X: usize = 14;

    // Top-left overlay for center frequency
    let cf_text = format!("CF: {}", format_freq_hz(state.center_freq_hz));
    let cf_x = SPECTRUM_PLOT_X0 + 8;
    let cf_y = SPECTRUM_PLOT_Y0 + 6;
    //draw_text_2x(buffer, fb_width, cf_x, cf_y, &cf_text, CF_COLOR);
    draw_text(buffer, fb_width, cf_x, cf_y, &cf_text, CF_COLOR);

    // Target-frequency label above the target marker location
    if let Some(tf_x_center) = freq_to_plot_x(state.target_freq_hz, state) {
        let tf_text = format!("TF: {}", format_freq_hz(state.target_freq_hz));
        let tf_width = tf_text.chars().count() * CHAR_ADVANCE_2X;

        let mut tf_x = tf_x_center.saturating_sub(tf_width / 2);
        let tf_y = SPECTRUM_PLOT_Y0 + 24;

        // Clamp label to visible spectrum area
        let min_x = SPECTRUM_PLOT_X0 + 4;
        let max_x = SPECTRUM_PLOT_X1.saturating_sub(tf_width + 4);

        if tf_x < min_x {
            tf_x = min_x;
        }
        if tf_x > max_x {
            tf_x = max_x;
        }

        draw_text(buffer, fb_width, tf_x, tf_y, &tf_text, TF_COLOR);

        // Optional small tick mark above the target x position
        let tick_top = tf_y + TEXT_HEIGHT_2X + 2;
        let tick_bottom = tick_top + 8;
        for y in tick_top..tick_bottom {
            if tf_x_center < fb_width && y < HEIGHT {
                buffer[y * fb_width + tf_x_center] = TF_COLOR;
            }
        }
    }
}
