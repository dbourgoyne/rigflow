use std::net::UdpSocket;

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:50000")?;
    let mut buf = [0u8; 2048];

    println!("Listening on UDP 50000...");

    loop {
        let (len, _) = socket.recv_from(&mut buf)?;

        if len < 16 {
            continue;
        }

        let payload = &buf[16..len];

        let samples: Vec<i16> = payload
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        println!("received {} samples", samples.len());
    }
}
