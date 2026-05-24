use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use log::{error, info, warn};

const HPSDR_PORT: u16 = 1024;
const PACKET_LEN: usize = 63;
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone)]
pub struct Hl2Device {
    /// Address of the device (its IP, port 1024).
    pub addr: SocketAddr,
    pub mac: [u8; 6],
    pub fw_version: u8,
    /// True if the device reports it is already in use by another host.
    pub in_use: bool,
}

impl Hl2Device {
    /// MAC as a compact lowercase hex string, used to build stable RadioIds.
    pub fn mac_hex(&self) -> String {
        format!(
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.mac[0], self.mac[1], self.mac[2],
            self.mac[3], self.mac[4], self.mac[5],
        )
    }
}

/// Send a Protocol 1 discovery broadcast and collect all responses that arrive
/// within the discovery timeout window.
///
/// We send on every non-loopback IPv4 interface's subnet broadcast address
/// rather than to 255.255.255.255, because limited broadcast is routed via the
/// default gateway (typically wlan0 on a Pi). Subnet-directed broadcasts are
/// routed by the kernel to the correct interface based on the routing table, so
/// an HL2 connected directly on eth0 will be found even when wlan0 is the
/// default route.
pub fn discover_hl2_devices() -> Vec<Hl2Device> {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            error!("HL2 discovery: bind failed: {e}");
            return Vec::new();
        }
    };

    if let Err(e) = socket.set_broadcast(true) {
        error!("HL2 discovery: set_broadcast failed: {e}");
        return Vec::new();
    }

    // Protocol 1 discovery request: EF FE 02 00 followed by 59 zero bytes.
    let mut request = [0u8; PACKET_LEN];
    request[0] = 0xEF;
    request[1] = 0xFE;
    request[2] = 0x02;
    // request[3..] already 0x00 — signals discovery, not a start command.

    send_on_all_interfaces(&socket, &request);

    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let mut devices = Vec::new();
    let mut buf = [0u8; PACKET_LEN];

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        socket.set_read_timeout(Some(remaining)).ok();

        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if !is_discovery_response(&buf, len) {
                    continue;
                }

                let in_use = buf[3] != 0;
                let mut mac = [0u8; 6];
                mac.copy_from_slice(&buf[4..10]);
                let fw_version = buf[10];

                info!(
                    "HL2 discovery: found {} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} fw={} {}",
                    src.ip(),
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                    fw_version,
                    if in_use { "(in use)" } else { "" },
                );

                if in_use {
                    warn!("HL2 at {} reports it is already in use by another host", src.ip());
                }

                // Normalise to port 1024 regardless of the source port in the
                // response — some firmware sends from an ephemeral port.
                devices.push(Hl2Device {
                    addr: SocketAddr::new(src.ip(), HPSDR_PORT),
                    mac,
                    fw_version,
                    in_use,
                });
            }

            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }

            Err(e) => {
                error!("HL2 discovery: recv_from error: {e}");
                break;
            }
        }
    }

    if devices.is_empty() {
        info!("HL2 discovery: no devices found on LAN");
    } else {
        info!("HL2 discovery: found {} device(s)", devices.len());
    }

    devices
}

/// Send `request` to the subnet broadcast address of every non-loopback IPv4
/// interface. Falls back to 255.255.255.255 if interface enumeration fails.
fn send_on_all_interfaces(socket: &UdpSocket, request: &[u8]) {
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(i) => i,
        Err(e) => {
            warn!("HL2 discovery: interface enumeration failed ({e}), falling back to 255.255.255.255");
            let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), HPSDR_PORT);
            socket.send_to(request, fallback).ok();
            return;
        }
    };

    let mut sent = false;
    for iface in &ifaces {
        if let if_addrs::IfAddr::V4(ref v4) = iface.addr {
            if v4.ip.is_loopback() {
                continue;
            }
            let bcast = v4.broadcast.unwrap_or(Ipv4Addr::BROADCAST);
            let target = SocketAddr::new(IpAddr::V4(bcast), HPSDR_PORT);
            match socket.send_to(request, target) {
                Ok(_) => {
                    info!("HL2 discovery: broadcast sent on {} ({} → {})", iface.name, v4.ip, bcast);
                    sent = true;
                }
                Err(e) => {
                    warn!("HL2 discovery: send on {} failed: {e}", iface.name);
                }
            }
        }
    }

    if !sent {
        warn!("HL2 discovery: no interfaces sent — no HL2 devices will be found");
    }
}

fn is_discovery_response(buf: &[u8], len: usize) -> bool {
    // Minimum useful response: magic (2) + type (1) + status (1) + MAC (6) + fw (1) = 11
    len >= 11 && buf[0] == 0xEF && buf[1] == 0xFE && buf[2] == 0x02
}
