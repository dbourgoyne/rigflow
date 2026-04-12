use std::net::{SocketAddr, UdpSocket};

/// Sends audio samples over UDP using a simple custom packet format.
///
/// Packet layout:
/// - u16 magic ("RS")
/// - u8  version
/// - u8  stream_type (1 = audio)
/// - u32 sequence
/// - u64 timestamp (sample count)
/// - payload: i16 samples (little-endian)
pub struct UdpAudioSender {
    socket: UdpSocket,
    sequence: u32,
    timestamp: u64,
    samples_per_packet: usize,
    pending: Vec<i16>,
}

impl UdpAudioSender {
    pub fn new(samples_per_packet: usize) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;

        Ok(Self {
            socket,
            sequence: 0,
            timestamp: 0,
            samples_per_packet,
            pending: Vec::new(),
        })
    }

    /// Queue audio samples and send full packets when enough data is accumulated.
    pub fn send_audio_to(&mut self, target: SocketAddr, samples: &[i16]) {
        self.pending.extend_from_slice(samples);

        while self.pending.len() >= self.samples_per_packet {
            // Drain exactly one packet worth of samples
            let chunk_len = self.samples_per_packet;

            let mut buf = Vec::with_capacity(16 + chunk_len * 2);

            // Header
            buf.extend_from_slice(&0x5253u16.to_be_bytes()); // "RS"
            buf.push(1); // version
            buf.push(1); // stream_type = audio
            buf.extend_from_slice(&self.sequence.to_be_bytes());
            buf.extend_from_slice(&self.timestamp.to_be_bytes());

            // Payload (drain directly into buffer without intermediate Vec)
            for s in self.pending.drain(..chunk_len) {
                buf.extend_from_slice(&s.to_le_bytes());
            }

            let _ = self.socket.send_to(&buf, target);

            self.sequence = self.sequence.wrapping_add(1);
            self.timestamp += chunk_len as u64;
        }
    }
}
