use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use log::info;

use rigflow_core::net::udp_framing::{
    MAGIC,
    VERSION,
    STREAM_TYPE_REGISTER_AUDIO,
};

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
    let mut buf = [0u8; 256];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;

        // Minimum header size
        if len < 4 {
            continue;
        }

        let header = &buf[..4];

        let magic = u16::from_be_bytes([header[0], header[1]]);
        let version = header[2];
        let stream_type = header[3];

        // Validate registration packet
        if magic != MAGIC
            || version != VERSION
            || stream_type != STREAM_TYPE_REGISTER_AUDIO
        {
            continue;
        }

        // Update shared target
        {
            let mut target = udp_audio_target.write().await;
            *target = Some(src);
        }

        // Send ACK (echo header)
        let _ = socket.send_to(header, src).await;

        info!("Registered UDP audio client: {}", src);
    }
}
