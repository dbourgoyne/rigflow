use std::net::{SocketAddr, UdpSocket};

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

    pub fn send_audio_to(&mut self, target: SocketAddr, samples: &[i16]) {
        self.pending.extend_from_slice(samples);

        while self.pending.len() >= self.samples_per_packet {
            let chunk: Vec<i16> = self.pending.drain(..self.samples_per_packet).collect();

            let mut buf = Vec::with_capacity(16 + chunk.len() * 2);

            // Header
            buf.extend_from_slice(&0x5253u16.to_be_bytes()); // "RS"
            buf.push(1); // version
            buf.push(1); // stream_type = audio
            buf.extend_from_slice(&self.sequence.to_be_bytes());
            buf.extend_from_slice(&self.timestamp.to_be_bytes());

            // Payload
            for &s in &chunk {
                buf.extend_from_slice(&s.to_le_bytes());
            }
/*
	    println!(
		"UDP AUDIO SEND: target={} seq={} samples={}",
		target,
		self.sequence,
		chunk.len()
        	);
*/
            let _ = self.socket.send_to(&buf, target);

            self.sequence = self.sequence.wrapping_add(1);
            self.timestamp += chunk.len() as u64;
        }
    }
}
