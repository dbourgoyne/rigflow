use std::net::{SocketAddr, UdpSocket};

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

    pub fn send_row_to(&mut self, target: SocketAddr, row: &[u8]) {
        if row.len() > u16::MAX as usize {
            return;
        }

        let mut buf = Vec::with_capacity(16 + 2 + row.len());

        // Common header
        buf.extend_from_slice(&0x5253u16.to_be_bytes()); // "RS"
        buf.push(1); // version
        buf.push(2); // stream_type = waterfall
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());

        // Waterfall payload
        buf.extend_from_slice(&(row.len() as u16).to_be_bytes());
        buf.extend_from_slice(row);

        let _ = self.socket.send_to(&buf, target);

        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(1);
    }
}
