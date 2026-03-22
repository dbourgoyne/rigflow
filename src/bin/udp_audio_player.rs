use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAGIC: u16 = 0x5253;
const VERSION: u8 = 1;
const STREAM_TYPE_AUDIO: u8 = 1;
const STREAM_TYPE_REGISTER_AUDIO: u8 = 10;

const LISTEN_ADDR: &str = "0.0.0.0:50000";
const SERVER_UDP_REGISTRATION_ADDR: &str = "192.168.0.225:9001";
const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;

const START_BUFFER_SAMPLES: usize = 4_800;
const MAX_BUFFER_SAMPLES: usize = 48_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let playback_started = Arc::new(Mutex::new(false));

    let queue_for_net = Arc::clone(&queue);
    let started_for_net = Arc::clone(&playback_started);

    thread::spawn(move || {
        if let Err(e) = udp_receive_loop(queue_for_net, started_for_net) {
            eprintln!("UDP receive loop error: {e}");
        }
    });

    let stream = build_output_stream(Arc::clone(&queue), Arc::clone(&playback_started))?;
    stream.play()?;

    println!("UDP audio player listening on {LISTEN_ADDR}");
    println!("Registering with server at {SERVER_UDP_REGISTRATION_ADDR}");

    loop {
        thread::sleep(Duration::from_secs(1));

        let queued = queue.lock().unwrap().len();
        let started = *playback_started.lock().unwrap();

        println!("queued_samples={queued} playback_started={started}");
    }
}

fn udp_receive_loop(
    queue: Arc<Mutex<VecDeque<f32>>>,
    playback_started: Arc<Mutex<bool>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(LISTEN_ADDR)?;

    // Send registration packet to the server
    let mut reg = Vec::with_capacity(4);
    reg.extend_from_slice(&MAGIC.to_be_bytes());
    reg.push(VERSION);
    reg.push(STREAM_TYPE_REGISTER_AUDIO);

    socket.send_to(&reg, SERVER_UDP_REGISTRATION_ADDR)?;
    println!("Sent UDP registration to {}", SERVER_UDP_REGISTRATION_ADDR);

    let mut buf = [0u8; 4096];
    let mut expected_sequence: Option<u32> = None;

    loop {
        let (len, src) = socket.recv_from(&mut buf)?;

        if len == 4 {
            let magic = u16::from_be_bytes([buf[0], buf[1]]);
            let version = buf[2];
            let stream_type = buf[3];

            if magic == MAGIC && version == VERSION && stream_type == STREAM_TYPE_REGISTER_AUDIO {
                println!("Received UDP registration ACK from {}", src);
                continue;
            }
        }

        if len < 16 {
            continue;
        }

        let packet = &buf[..len];

        let magic = u16::from_be_bytes([packet[0], packet[1]]);
        let version = packet[2];
        let stream_type = packet[3];
        let sequence = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
        let _timestamp = u64::from_be_bytes([
            packet[8], packet[9], packet[10], packet[11],
            packet[12], packet[13], packet[14], packet[15],
        ]);

        if magic != MAGIC || version != VERSION || stream_type != STREAM_TYPE_AUDIO {
            continue;
        }

        if let Some(expected) = expected_sequence {
            if sequence != expected {
                eprintln!("packet gap or reorder: expected={expected} got={sequence}");
            }
        }
        expected_sequence = Some(sequence.wrapping_add(1));

        let payload = &packet[16..];

        let mut q = queue.lock().unwrap();

        for chunk in payload.chunks_exact(2) {
            let s = i16::from_le_bytes([chunk[0], chunk[1]]);
            q.push_back(s as f32 / 32768.0);
        }

        while q.len() > MAX_BUFFER_SAMPLES {
            q.pop_front();
        }

        let mut started = playback_started.lock().unwrap();
        if !*started && q.len() >= START_BUFFER_SAMPLES {
            *started = true;
            println!("Starting playback with {} buffered samples", q.len());
        }
    }
}


fn build_output_stream(
    queue: Arc<Mutex<VecDeque<f32>>>,
    playback_started: Arc<Mutex<bool>>,
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
            let started = *playback_started.lock().unwrap();

            if !started {
                for sample in data.iter_mut() {
                    *sample = 0.0;
                }
                return;
            }

            let mut q = queue.lock().unwrap();

            for sample in data.iter_mut() {
                *sample = q.pop_front().unwrap_or(0.0);
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
