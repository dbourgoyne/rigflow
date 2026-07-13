use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tokio::net::UdpSocket;

use log::info;

use rigflow_core::net::udp_framing::{
    build_time_sync_response, epoch_nanos, parse_time_sync_request, MAGIC, STREAM_TYPE_MIC_AUDIO,
    STREAM_TYPE_REGISTER_AUDIO, STREAM_TYPE_TIME_SYNC_REQUEST, VERSION,
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
    socket: UdpSocket,
    udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Large enough for a mic-audio packet (~240 mono f32 samples + header).
    let mut buf = [0u8; 8192];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;
        // Capture the server receive time as early as possible (T2 for TIME_SYNC).
        let recv_ns = epoch_nanos();

        // Minimum header size
        if len < 4 {
            continue;
        }

        let header = [buf[0], buf[1], buf[2], buf[3]];

        let magic = u16::from_be_bytes([header[0], header[1]]);
        let version = header[2];
        let stream_type = header[3];

        // Accept any protocol version we understand (1..=VERSION).
        if magic != MAGIC || version < 1 || version > VERSION {
            continue;
        }

        match stream_type {
            STREAM_TYPE_REGISTER_AUDIO => {
                // Record the reflexive source (the address, post-NAT, that the
                // client's packets actually arrive from) as the media target, and
                // ACK (echo header).
                *udp_audio_target.write().unwrap_or_else(|e| e.into_inner()) = Some(src);
                let _ = socket.send_to(&header, src).await;
                info!("Registered UDP audio client: {}", src);
            }
            STREAM_TYPE_MIC_AUDIO => {
                // Mono f32 LE samples after the 4-byte header.  Loss-tolerant:
                // push into the global queue the worker drains while keying SSB.
                let payload = &buf[4..len];
                let mut samples = Vec::with_capacity(payload.len() / 4);
                for chunk in payload.chunks_exact(4) {
                    samples.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
                if !samples.is_empty() {
                    push_mic_samples(&samples);
                }
            }
            STREAM_TYPE_TIME_SYNC_REQUEST => {
                // The 1 Hz time-sync probe doubles as a media-target refresh: it
                // re-observes the reflexive source, so a lost initial registration
                // (or a NAT rebind) self-heals within ~1 s.
                *udp_audio_target.write().unwrap_or_else(|e| e.into_inner()) = Some(src);
                // Clock-offset probe: echo T1 plus the server receive (T2) and
                // send (T3) wall-clocks so the client can compute offset + RTT.
                if let Some((probe_id, t1)) = parse_time_sync_request(&buf[..len]) {
                    let t3 = epoch_nanos();
                    let resp = build_time_sync_response(probe_id, t1, recv_ns, t3);
                    let _ = socket.send_to(&resp, src).await;
                }
            }
            _ => {}
        }
    }
}
