use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use radio_server::audio_client::jitter_buffer::JitterBuffer;
use serde::{Deserialize, Serialize};
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

const WIDTH: usize = 512;
const HEIGHT: usize = 400;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    SetFrequency { target_freq_hz: f32 },
    SetCenterFrequency { center_freq_hz: f32 },
    SetSideband { sideband: String },
    Ping,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready,
    Pong,
    FrequencyChanged { target_freq_hz: f32 },
    CenterFrequencyChanged { center_freq_hz: f32 },
    SidebandChanged { sideband: String },
    StreamConfig {
        audio_sample_rate_hz: f32,
        audio_format: String,
        waterfall_bins: usize,
        waterfall_frame_rate_hz: f32,
        center_freq_hz: f32,
        input_sample_rate_hz: f32,
    },
    UdpAudioOffer {
        server_udp_port: u16,
    },
    Info { message: String },
    Error { message: String },
}

#[derive(Debug, Clone)]
struct UiState {
    center_freq_hz: f32,
    target_freq_hz: f32,
    sideband: String,
    input_sample_rate_hz: f32,
    waterfall_bins: usize,
    audio_sample_rate_hz: f32,
    status: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            sideband: "lsb".to_string(),
            input_sample_rate_hz: 0.0,
            waterfall_bins: WIDTH,
            audio_sample_rate_hz: OUTPUT_SAMPLE_RATE as f32,
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

                    if magic == MAGIC && version == VERSION && stream_type == STREAM_TYPE_REGISTER_AUDIO {
                        println!("Received UDP registration ACK from {}", src);
                    }
                } else if len >= 16 {
                    handle_media_packet(&udp_buf[..len], &jitter, &waterfall_buffer);
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }

        handle_keyboard(&window, &ws_cmd_tx, &ui_state);

        {
            let mut buf = waterfall_buffer.lock().unwrap();
            let state = ui_state.lock().unwrap().clone();
            draw_tuning_marker(&mut buf, WIDTH, HEIGHT, &state);
            window.update_with_buffer(&buf, WIDTH, HEIGHT)?;
        }

        if last_title.elapsed() >= Duration::from_millis(200) {
            let state = ui_state.lock().unwrap().clone();
            window.set_title(&format!(
                "Rust Radio Desktop Client | Ctr: {:.0} Hz | Tgt: {:.0} Hz | {} | {}",
                state.center_freq_hz,
                state.target_freq_hz,
                state.sideband.to_uppercase(),
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
                        write.send(tokio_tungstenite::tungstenite::Message::Text(text)).await?;
                    }
                    None => break,
                }
            }

            msg = read.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
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
        ServerMessage::StreamConfig {
            audio_sample_rate_hz,
            audio_format: _,
            waterfall_bins,
            waterfall_frame_rate_hz: _,
            center_freq_hz,
            input_sample_rate_hz,
        } => {
            state.audio_sample_rate_hz = audio_sample_rate_hz;
            state.waterfall_bins = waterfall_bins;
            state.center_freq_hz = center_freq_hz;
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

    let target_step = if shift { 100_000.0 } else { 10_000.0 };
    let center_step = if shift { 100_000.0 } else { 10_000.0 };

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

    if window.is_key_pressed(Key::P, KeyRepeat::No) {
        let _ = ws_cmd_tx.send(ClientMessage::Ping);
    }
}

fn handle_media_packet(
    packet: &[u8],
    jitter: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
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
            let mut buffer = waterfall_buffer.lock().unwrap();
            draw_row(&mut buffer, row, WIDTH, HEIGHT);
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

fn draw_row(buffer: &mut [u32], row: &[u8], width: usize, height: usize) {
    buffer.copy_within(0..width * (height - 1), width);

    let top = &mut buffer[0..width];

    for x in 0..width {
        let v = if x < row.len() { row[x] } else { 0 };
        top[x] = color_map(v);
    }
}

fn draw_tuning_marker(buffer: &mut [u32], width: usize, height: usize, state: &UiState) {
    if state.input_sample_rate_hz <= 0.0 {
        return;
    }

    let offset_hz = state.target_freq_hz - state.center_freq_hz;
    let frac = offset_hz / state.input_sample_rate_hz + 0.5;
    let x = (frac * width as f32).round() as isize;

    if x < 0 || x >= width as isize {
        return;
    }

    let x = x as usize;
    for y in 0..height {
        buffer[y * width + x] = 0x00FF0000;
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
