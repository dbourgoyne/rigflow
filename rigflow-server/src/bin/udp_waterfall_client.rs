use minifb::{Key, Window, WindowOptions};
use std::net::UdpSocket;
use std::time::Duration;
use rigflow_core::net::udp_framing::STREAM_TYPE_WATERFALL;
use rigflow_core::net::udp_framing::{
    MAGIC,
    VERSION,
    STREAM_TYPE_REGISTER_AUDIO,
};

const LISTEN_ADDR: &str = "0.0.0.0:50000";
const SERVER_UDP_REGISTRATION_ADDR: &str = "192.168.0.225:9001";

const WIDTH: usize = 512;
const HEIGHT: usize = 400;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(LISTEN_ADDR)?;
    socket.set_read_timeout(Some(Duration::from_millis(10)))?;

    // Reuse the same UDP registration mechanism.
    let mut reg = Vec::with_capacity(4);
    reg.extend_from_slice(&MAGIC.to_be_bytes());
    reg.push(VERSION);
    reg.push(STREAM_TYPE_REGISTER_AUDIO);
    socket.send_to(&reg, SERVER_UDP_REGISTRATION_ADDR)?;

    println!("Sent UDP registration to {}", SERVER_UDP_REGISTRATION_ADDR);
    println!("Listening on {}", LISTEN_ADDR);

    let mut window = Window::new(
        "UDP Waterfall Client",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )?;

    let mut buffer = vec![0u32; WIDTH * HEIGHT];
    let mut udp_buf = [0u8; 2048];

    while window.is_open() && !window.is_key_down(Key::Escape) {
        match socket.recv_from(&mut udp_buf) {
            Ok((len, src)) => {
                if len == 4 {
                    let magic = u16::from_be_bytes([udp_buf[0], udp_buf[1]]);
                    let version = udp_buf[2];
                    let stream_type = udp_buf[3];

                    if magic == MAGIC && version == VERSION && stream_type == STREAM_TYPE_REGISTER_AUDIO {
                        println!("Received UDP registration ACK from {}", src);
                        continue;
                    }
                }

                if len < 18 {
                    continue;
                }

                let packet = &udp_buf[..len];

                let magic = u16::from_be_bytes([packet[0], packet[1]]);
                let version = packet[2];
                let stream_type = packet[3];

                if magic != MAGIC || version != VERSION || stream_type != STREAM_TYPE_WATERFALL {
                    continue;
                }

                let bin_count = u16::from_be_bytes([packet[16], packet[17]]) as usize;
                let payload = &packet[18..];

                if payload.len() < bin_count {
                    continue;
                }

                let row = &payload[..bin_count];
                draw_row(&mut buffer, row, WIDTH, HEIGHT);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }

        window.update_with_buffer(&buffer, WIDTH, HEIGHT)?;
    }

    Ok(())
}

fn draw_row(buffer: &mut [u32], row: &[u8], width: usize, height: usize) {
    // Scroll existing rows down by one row
    buffer.copy_within(0..width * (height - 1), width);

    // Draw new row at top
    let top = &mut buffer[0..width];

    for x in 0..width {
        let v = if x < row.len() { row[x] } else { 0 };
        top[x] = color_map(v);
    }
}

fn color_map(v: u8) -> u32 {
    // Simple SDR-ish gradient: black -> blue -> cyan -> yellow -> red
    let x = v as f32 / 255.0;

    let (r, g, b) = if x < 0.25 {
        let t = x / 0.25;
        (0.0, 0.0, 255.0 * t)
    } else if x < 0.5 {
        let t = (x - 0.25) / 0.25;
        (0.0, 255.0 * t, 255.0)
    } else if x < 0.75 {
        let t = (x - 0.5) / 0.25;
        (255.0 * t, 255.0, 255.0 * (1.0 - t))
    } else {
        let t = (x - 0.75) / 0.25;
        (255.0, 255.0 * (1.0 - t), 0.0)
    };

    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
