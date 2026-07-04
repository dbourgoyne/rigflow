use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;

use rigflow_core::net::udp_framing::{epoch_nanos, MAGIC, STREAM_TYPE_AUDIO, VERSION};

/// Sends audio samples over UDP using a simple custom packet format.
///
/// Packet layout (v2):
/// - u16 magic ("RS")
/// - u8  version (2)
/// - u8  stream_type (1 = audio)
/// - u32 sequence
/// - u64 timestamp (sample count)
/// - u64 send_wall_ns (server send time, epoch nanoseconds) — v2 only
/// - payload: i16 samples (little-endian)
pub struct UdpAudioSender {
    socket: Arc<UdpSocket>,
    sequence: u32,
    timestamp: u64,
    samples_per_packet: usize,
    pending: Vec<i16>,
}

impl UdpAudioSender {
    /// `socket` is the shared server socket bound to the registration port, so
    /// audio egresses from the same 5-tuple the client registered against.
    pub fn new(socket: Arc<UdpSocket>, samples_per_packet: usize) -> Self {
        Self {
            socket,
            sequence: 0,
            timestamp: 0,
            samples_per_packet,
            pending: Vec::new(),
        }
    }

    /// Queue audio samples and send full packets when enough data is accumulated.
    pub fn send_audio_to(&mut self, target: SocketAddr, samples: &[i16]) {
        self.pending.extend_from_slice(samples);

        while self.pending.len() >= self.samples_per_packet {
            // Drain exactly one packet worth of samples
            let chunk_len = self.samples_per_packet;

            let mut buf = Vec::with_capacity(24 + chunk_len * 2);

            // Header
            buf.extend_from_slice(&MAGIC.to_be_bytes()); // "RS"
            buf.push(VERSION); // version (2)
            buf.push(STREAM_TYPE_AUDIO); // stream_type = audio
            buf.extend_from_slice(&self.sequence.to_be_bytes());
            buf.extend_from_slice(&self.timestamp.to_be_bytes());
            // v2: server send wall-clock (epoch ns), captured as late as possible
            // before the payload so it reflects actual send time.
            buf.extend_from_slice(&epoch_nanos().to_be_bytes());

            // Payload (drain directly into buffer without intermediate Vec)
            for s in self.pending.drain(..chunk_len) {
                buf.extend_from_slice(&s.to_le_bytes());
            }

            //let _ = self.socket.send_to(&buf, target);
            match self.socket.send_to(&buf, target) {
                Ok(_) => {}
                Err(e) => {
                    log::warn!(
                        "udp audio send_to failed: seq={} samples={} err={}",
                        self.sequence,
                        chunk_len,
                        e
                    );
                }
            }

            self.sequence = self.sequence.wrapping_add(1);
            self.timestamp += chunk_len as u64;
        }
    }
}
