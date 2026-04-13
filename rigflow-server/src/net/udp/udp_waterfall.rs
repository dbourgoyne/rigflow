use std::net::{SocketAddr, UdpSocket};

use rigflow_core::net::udp_framing::{MAGIC, STREAM_TYPE_WATERFALL, VERSION};

/// Sends waterfall rows over UDP using a simple custom packet format.
///
/// Packet layout:
/// - u16 magic ("RS")
/// - u8  version
/// - u8  stream_type (2 = waterfall)
/// - u32 sequence
/// - u64 timestamp (row counter)
/// - u16 payload length in bytes
/// - payload: little-endian f32 dB values, one per FFT bin
pub struct UdpWaterfallSender {
    socket: UdpSocket,
    sequence: u32,
    timestamp: u64,
}

impl UdpWaterfallSender {
    pub fn new() -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;

        Ok(Self {
            socket,
            sequence: 0,
            timestamp: 0,
        })
    }

    /// Send a single waterfall row of dB values to the target.
    pub fn send_row_db_to(&mut self, target: SocketAddr, row_db: &[f32]) {
        let payload_len = match row_db.len().checked_mul(std::mem::size_of::<f32>()) {
            Some(len) if len <= u16::MAX as usize => len as u16,
            _ => return,
        };

        let mut buf = Vec::with_capacity(16 + 2 + payload_len as usize);

        // Header
        buf.extend_from_slice(&MAGIC.to_be_bytes());
        buf.push(VERSION);
        buf.push(STREAM_TYPE_WATERFALL);
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());

        // Payload
        buf.extend_from_slice(&payload_len.to_be_bytes());
        for value in row_db {
            buf.extend_from_slice(&value.to_le_bytes());
        }

        let _ = self.socket.send_to(&buf, target);

        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(1);
    }
}
