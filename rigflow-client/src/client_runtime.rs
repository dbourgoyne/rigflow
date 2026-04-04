use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

#[derive(Debug, Clone)]
pub enum MediaCommand {
    RegisterUdp {
        server_ip: String,
        server_udp_port: u16,
    },
}

#[derive(Clone)]
pub struct MediaRuntimeHandles {
    pub media_cmd_tx: mpsc::UnboundedSender<MediaCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
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
    })
}
