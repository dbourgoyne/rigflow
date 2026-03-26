use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use std::sync::Arc;

use rigflow_core::net::udp_framing::{
    MAGIC,
    VERSION,
    STREAM_TYPE_REGISTER_AUDIO,
};

pub async fn run_udp_registration_listener(
    bind_addr: &str,
    udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = UdpSocket::bind(bind_addr).await?;
    let mut buf = [0u8; 256];

    //println!("UDP registration listener on {}", bind_addr);

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;

        if len < 4 {
            continue;
        }

        let magic = u16::from_be_bytes([buf[0], buf[1]]);
        let version = buf[2];
        let stream_type = buf[3];

        if magic != MAGIC || version != VERSION || stream_type != STREAM_TYPE_REGISTER_AUDIO {
            continue;
        }

        {
            let mut target = udp_audio_target.write().await;
            *target = Some(src);
        }

        // Optional ACK: same 4-byte header echoed back
        let ack = [buf[0], buf[1], buf[2], buf[3]];
        let _ = socket.send_to(&ack, src).await;

        println!("Registered UDP audio client: {}", src);
    }
}
