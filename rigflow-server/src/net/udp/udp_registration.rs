use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use log::info;

use rigflow_core::net::udp_framing::{
    MAGIC,
    STREAM_TYPE_MIC_AUDIO,
    STREAM_TYPE_REGISTER_AUDIO,
    VERSION,
};

use crate::net::udp::mic_audio::push_mic_samples;

/// Listens for UDP registration packets from clients.
///
/// Expected packet format:
/// - u16 magic
/// - u8  version
/// - u8  stream_type (REGISTER_AUDIO)
///
/// On valid packet:
/// - stores sender as current UDP audio target
/// - sends 4-byte ACK (echo header)
pub async fn run_udp_registration_listener(
    bind_addr: &str,
    udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = UdpSocket::bind(bind_addr).await?;
    // Large enough for a mic-audio packet (~240 mono f32 samples + header).
    let mut buf = [0u8; 8192];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;

        // Minimum header size
        if len < 4 {
            continue;
        }

        let header = [buf[0], buf[1], buf[2], buf[3]];

        let magic = u16::from_be_bytes([header[0], header[1]]);
        let version = header[2];
        let stream_type = header[3];

        if magic != MAGIC || version != VERSION {
            continue;
        }

        match stream_type {
            STREAM_TYPE_REGISTER_AUDIO => {
                // Update shared target + ACK (echo header).
                {
                    let mut target = udp_audio_target.write().await;
                    *target = Some(src);
                }
                let _ = socket.send_to(&header, src).await;
                info!("Registered UDP audio client: {}", src);
            }
            STREAM_TYPE_MIC_AUDIO => {
                // Mono f32 LE samples after the 4-byte header.  Loss-tolerant:
                // push into the global queue the worker drains while keying SSB.
                let payload = &buf[4..len];
                let mut samples = Vec::with_capacity(payload.len() / 4);
                for chunk in payload.chunks_exact(4) {
                    samples.push(f32::from_le_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3],
                    ]));
                }
                if !samples.is_empty() {
                    push_mic_samples(&samples);
                }
            }
            _ => {}
        }
    }
}
