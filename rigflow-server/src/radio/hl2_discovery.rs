use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

const HPSDR_PORT: u16 = 1024;
const PACKET_LEN: usize = 63;
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Re-send the discovery broadcast this often during the listen window, so a
/// single dropped request/reply doesn't make a present HL2 look absent (which
/// would otherwise let `rescan_radios` prune a powered-on device).
const REBROADCAST_INTERVAL: Duration = Duration::from_millis(250);
/// Idle nap between receive sweeps across the probe sockets. Short enough that
/// a reply is picked up promptly, long enough not to spin a core for a second.
const POLL_NAP: Duration = Duration::from_millis(5);

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
            self.mac[0], self.mac[1], self.mac[2], self.mac[3], self.mac[4], self.mac[5],
        )
    }
}

/// One discovery socket plus every address it should probe.
///
/// Sockets are bound to a specific local interface address rather than
/// `0.0.0.0` so that a limited broadcast (255.255.255.255) leaves via *that*
/// interface: for a limited-broadcast destination the kernel picks the output
/// device from the socket's source address. Replies are unicast back to the
/// socket that asked, so each probe is also a receive endpoint.
struct Probe {
    socket: UdpSocket,
    /// Interface name (or a placeholder) for logging.
    label: String,
    targets: Vec<SocketAddr>,
}

/// Send a Protocol 1 discovery broadcast and collect all responses that arrive
/// within the discovery timeout window.
///
/// On every non-loopback IPv4 interface we probe **both**:
/// - the subnet-directed broadcast (e.g. 192.168.1.255), and
/// - the limited broadcast 255.255.255.255.
///
/// Both are needed. The directed broadcast is what lets an HL2 on eth0 be found
/// when wlan0 holds the default route, because the kernel routes it out the
/// matching interface. But the HL2's gateware IP stack does not know its own
/// netmask, so it only accepts frames addressed to its own IP or to
/// 255.255.255.255 — on any subnet wider than /24 (a mesh router handing out a
/// /22, say) the directed broadcast is an address the device does not recognise
/// as its own and discovery silently finds nothing. Quisk and SparkSDR probe
/// both addresses for the same reason.
///
/// `extra_hosts` are additional unicast addresses to probe directly (from
/// `--hl2-ip`), for setups where neither broadcast reaches the device — a
/// routed segment, a VLAN, or an AP that drops broadcast traffic.
pub fn discover_hl2_devices(extra_hosts: &[Ipv4Addr]) -> Vec<Hl2Device> {
    let probes = build_probes(extra_hosts);
    if probes.is_empty() {
        warn!("HL2 discovery: no usable sockets — no HL2 devices will be found");
        return Vec::new();
    }

    // Protocol 1 discovery request: EF FE 02 00 followed by 59 zero bytes.
    let mut request = [0u8; PACKET_LEN];
    request[0] = 0xEF;
    request[1] = 0xFE;
    request[2] = 0x02;
    // request[3..] already 0x00 — signals discovery, not a start command.

    send_all(&probes, &request, true);

    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let mut next_broadcast = Instant::now() + REBROADCAST_INTERVAL;
    let mut devices: Vec<Hl2Device> = Vec::new();

    while Instant::now() < deadline {
        // Re-broadcast periodically through the window for robustness.
        if Instant::now() >= next_broadcast {
            send_all(&probes, &request, false);
            next_broadcast = Instant::now() + REBROADCAST_INTERVAL;
        }

        // Sweep every probe socket; nap only when the whole sweep came up empty,
        // so a burst of replies is drained without an artificial delay between
        // them.
        let mut got_any = false;
        for probe in &probes {
            while let Some(dev) = recv_device(probe) {
                got_any = true;

                // A device replies to every request in the burst, on every
                // socket that reached it — keep one entry per MAC.
                if devices.iter().any(|d| d.mac == dev.mac) {
                    continue;
                }

                info!(
                    "HL2 discovery: found {} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} fw={} {}",
                    dev.addr.ip(),
                    dev.mac[0], dev.mac[1], dev.mac[2], dev.mac[3], dev.mac[4], dev.mac[5],
                    dev.fw_version,
                    if dev.in_use { "(in use)" } else { "" },
                );

                if dev.in_use {
                    warn!(
                        "HL2 at {} reports it is already in use by another host",
                        dev.addr.ip()
                    );
                }

                devices.push(dev);
            }
        }

        if !got_any {
            thread::sleep(POLL_NAP);
        }
    }

    if devices.is_empty() {
        info!(
            "HL2 discovery: no devices found on LAN (if the radio is powered on \
             and reachable, try --hl2-ip <address> to probe it directly)"
        );
    } else {
        info!("HL2 discovery: found {} device(s)", devices.len());
    }

    devices
}

/// Every address one probe socket should send to: the interface's directed
/// broadcast (when it has one), then the limited broadcast, then any operator-
/// supplied unicast hosts. Duplicates are dropped so a burst is not sent twice
/// to the same address (e.g. a /31 whose directed broadcast is 255.255.255.255,
/// or an `--hl2-ip` that repeats a broadcast address).
fn probe_targets(iface_broadcast: Option<Ipv4Addr>, extra_hosts: &[Ipv4Addr]) -> Vec<SocketAddr> {
    let mut targets: Vec<SocketAddr> = Vec::new();
    let mut push = |ip: Ipv4Addr| {
        let addr = SocketAddr::new(IpAddr::V4(ip), HPSDR_PORT);
        if !targets.contains(&addr) {
            targets.push(addr);
        }
    };

    if let Some(bcast) = iface_broadcast {
        push(bcast);
    }
    push(Ipv4Addr::BROADCAST);
    for ip in extra_hosts {
        push(*ip);
    }

    targets
}

/// Build one probe socket per non-loopback IPv4 interface, each targeting that
/// interface's directed broadcast plus the limited broadcast. Falls back to a
/// single wildcard-bound socket if interface enumeration fails.
fn build_probes(extra_hosts: &[Ipv4Addr]) -> Vec<Probe> {
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(i) => i,
        Err(e) => {
            warn!("HL2 discovery: interface enumeration failed ({e}), falling back to 0.0.0.0");
            let targets = probe_targets(None, extra_hosts);
            return bind_probe(Ipv4Addr::UNSPECIFIED, "any".to_string(), targets)
                .into_iter()
                .collect();
        }
    };

    let mut probes = Vec::new();
    for iface in &ifaces {
        let if_addrs::IfAddr::V4(ref v4) = iface.addr else {
            continue;
        };
        if v4.ip.is_loopback() {
            continue;
        }

        let targets = probe_targets(v4.broadcast, extra_hosts);
        probes.extend(bind_probe(v4.ip, iface.name.clone(), targets));
    }

    if probes.is_empty() {
        warn!("HL2 discovery: no non-loopback IPv4 interfaces, falling back to 0.0.0.0");
        let targets = probe_targets(None, extra_hosts);
        probes.extend(bind_probe(
            Ipv4Addr::UNSPECIFIED,
            "any".to_string(),
            targets,
        ));
    }

    probes
}

fn bind_probe(local_ip: Ipv4Addr, label: String, targets: Vec<SocketAddr>) -> Option<Probe> {
    let socket = match UdpSocket::bind(SocketAddr::new(IpAddr::V4(local_ip), 0)) {
        Ok(s) => s,
        Err(e) => {
            warn!("HL2 discovery: bind on {label} ({local_ip}) failed: {e}");
            return None;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        warn!("HL2 discovery: set_broadcast on {label} failed: {e}");
        return None;
    }
    if let Err(e) = socket.set_nonblocking(true) {
        warn!("HL2 discovery: set_nonblocking on {label} failed: {e}");
        return None;
    }
    Some(Probe {
        socket,
        label,
        targets,
    })
}

/// Send `request` to every target of every probe. `announce` logs at info level
/// for the first burst; later bursts log at debug so a rescan doesn't spam.
fn send_all(probes: &[Probe], request: &[u8], announce: bool) {
    let mut sent = false;
    for probe in probes {
        for target in &probe.targets {
            match probe.socket.send_to(request, target) {
                Ok(_) => {
                    sent = true;
                    if announce {
                        info!(
                            "HL2 discovery: request sent on {} ({} → {})",
                            probe.label,
                            probe
                                .socket
                                .local_addr()
                                .map(|a| a.ip().to_string())
                                .unwrap_or_default(),
                            target.ip(),
                        );
                    } else {
                        debug!(
                            "HL2 discovery: re-sent on {} → {}",
                            probe.label,
                            target.ip()
                        );
                    }
                }
                Err(e) => {
                    // A down or unroutable interface is normal on a multi-homed
                    // host (docker0, an idle wlan0), so this is not fatal — the
                    // other probes still run. Report it once on the first burst
                    // and stay quiet for the repeats.
                    if announce {
                        warn!(
                            "HL2 discovery: send on {} → {} failed: {e}",
                            probe.label,
                            target.ip()
                        );
                    } else {
                        debug!(
                            "HL2 discovery: re-send on {} → {} failed: {e}",
                            probe.label,
                            target.ip()
                        );
                    }
                }
            }
        }
    }

    if !sent {
        warn!("HL2 discovery: every send failed — no HL2 devices will be found");
    }
}

/// Drain one datagram from `probe`, returning a device if it was a valid
/// discovery response. `None` means "nothing more to read right now".
fn recv_device(probe: &Probe) -> Option<Hl2Device> {
    let mut buf = [0u8; PACKET_LEN];
    loop {
        match probe.socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if !is_discovery_response(&buf, len) {
                    continue;
                }
                let IpAddr::V4(_) = src.ip() else { continue };

                let mut mac = [0u8; 6];
                mac.copy_from_slice(&buf[4..10]);

                // Normalise to port 1024 regardless of the source port in the
                // response — some firmware sends from an ephemeral port.
                return Some(Hl2Device {
                    addr: SocketAddr::new(src.ip(), HPSDR_PORT),
                    mac,
                    fw_version: buf[10],
                    in_use: buf[3] != 0,
                });
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                error!("HL2 discovery: recv on {} failed: {e}", probe.label);
                return None;
            }
        }
    }
}

fn is_discovery_response(buf: &[u8], len: usize) -> bool {
    // Minimum useful response: magic (2) + type (1) + status (1) + MAC (6) + fw (1) = 11
    len >= 11 && buf[0] == 0xEF && buf[1] == 0xFE && buf[2] == 0x02
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn ips(targets: &[SocketAddr]) -> Vec<String> {
        targets.iter().map(|t| t.ip().to_string()).collect()
    }

    /// The regression this module exists for (issue #43): a device on a subnet
    /// wider than /24 never sees the directed broadcast, because its gateware
    /// does not know its own netmask. The limited broadcast must always be
    /// probed alongside it.
    #[test]
    fn always_probes_limited_broadcast_alongside_directed() {
        let targets = probe_targets(Some(ip("192.168.71.255")), &[]);
        assert_eq!(ips(&targets), vec!["192.168.71.255", "255.255.255.255"]);
    }

    #[test]
    fn probes_limited_broadcast_when_interface_has_none() {
        let targets = probe_targets(None, &[]);
        assert_eq!(ips(&targets), vec!["255.255.255.255"]);
    }

    #[test]
    fn appends_operator_supplied_hosts_after_broadcasts() {
        let targets = probe_targets(
            Some(ip("192.168.1.255")),
            &[ip("192.168.68.84"), ip("10.0.0.5")],
        );
        assert_eq!(
            ips(&targets),
            vec![
                "192.168.1.255",
                "255.255.255.255",
                "192.168.68.84",
                "10.0.0.5"
            ]
        );
    }

    #[test]
    fn deduplicates_targets() {
        // A directed broadcast that is already the limited broadcast, plus an
        // --hl2-ip that repeats it, must each be sent to only once.
        let targets = probe_targets(Some(Ipv4Addr::BROADCAST), &[Ipv4Addr::BROADCAST]);
        assert_eq!(ips(&targets), vec!["255.255.255.255"]);
    }

    #[test]
    fn every_probe_target_uses_the_hpsdr_port() {
        let targets = probe_targets(Some(ip("192.168.1.255")), &[ip("192.168.1.20")]);
        assert!(targets.iter().all(|t| t.port() == HPSDR_PORT));
    }

    #[test]
    fn accepts_a_well_formed_discovery_response() {
        let mut buf = [0u8; PACKET_LEN];
        buf[0] = 0xEF;
        buf[1] = 0xFE;
        buf[2] = 0x02;
        assert!(is_discovery_response(&buf, PACKET_LEN));
        // Truncated below the MAC + firmware fields we go on to read.
        assert!(!is_discovery_response(&buf, 10));
    }

    #[test]
    fn rejects_foreign_traffic_on_the_discovery_socket() {
        let buf = [0xDEu8; PACKET_LEN];
        assert!(!is_discovery_response(&buf, PACKET_LEN));
    }
}
