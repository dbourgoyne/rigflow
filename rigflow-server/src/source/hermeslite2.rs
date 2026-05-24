use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use log::{debug, info, warn};
use num_complex::Complex32;

use rigflow_core::radio::source_control::{SourceCapabilities, SourceControlState};

use crate::source::IqSource;

// Protocol 1 packet layout constants.
const P1_PACKET_LEN: usize = 1032;
const P1_OUTER_SYNC: [u8; 3] = [0xEF, 0xFE, 0x01];
const P1_ENDPOINT_H2D: u8 = 0x02; // host-to-device: C&C + TX
const P1_ENDPOINT_D2H: u8 = 0x06; // device-to-host: DDC data
const P1_SUBFRAME_SYNC: [u8; 3] = [0x7F, 0x7F, 0x7F];
const P1_SAMPLES_PER_SUBFRAME: usize = 63;
// Byte offsets within the 1032-byte packet where each 512-byte sub-frame begins.
const P1_SUBFRAME_OFFSETS: [usize; 2] = [8, 520];

const RECV_TIMEOUT: Duration = Duration::from_secs(2);

pub struct HermesLite2Source {
    socket: UdpSocket,
    sample_rate_hz: f32,
    center_freq_hz: f32,
    tx_seq: u32,
    pending: VecDeque<Complex32>,
}

impl HermesLite2Source {
    /// Open a P1 UDP connection to the discovered HL2 at `addr_str` ("ip:port").
    /// Sends the start command and an initial C&C packet (frequency + sample rate).
    pub fn open(addr_str: &str, sample_rate_hz: f32, center_freq_hz: f32) -> Result<Self, String> {
        let device_addr: SocketAddr = addr_str
            .parse()
            .map_err(|e| format!("HL2: invalid device address '{addr_str}': {e}"))?;

        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("HL2: UDP bind failed: {e}"))?;
        socket
            .connect(device_addr)
            .map_err(|e| format!("HL2: UDP connect to {device_addr} failed: {e}"))?;
        socket
            .set_read_timeout(Some(RECV_TIMEOUT))
            .map_err(|e| format!("HL2: set_read_timeout failed: {e}"))?;

        let mut src = Self {
            socket,
            sample_rate_hz,
            center_freq_hz,
            tx_seq: 0,
            pending: VecDeque::new(),
        };

        src.send_run(true)?;
        src.send_cc()?;
        info!(
            "HL2: P1 stream started from {device_addr} — \
             sample_rate={sample_rate_hz} Hz  center={center_freq_hz} Hz"
        );

        Ok(src)
    }

    /// Send the 64-byte Protocol 1 start (run=true) or stop (run=false) packet.
    fn send_run(&self, run: bool) -> Result<(), String> {
        let mut pkt = [0u8; 64];
        pkt[0] = 0xEF;
        pkt[1] = 0xFE;
        pkt[2] = 0x04;
        pkt[3] = run as u8;
        self.socket
            .send(&pkt)
            .map_err(|e| format!("HL2: send run={run} failed: {e}"))?;
        Ok(())
    }

    /// Build and send a 1032-byte C&C+TX packet with the current frequency and
    /// sample rate.
    ///
    /// Sub-frame 1: C0=0x00 (address 0) → speed code in C1[1:0]
    /// Sub-frame 2: C0=0x02 (address 1) → NCO frequency for DDC0 (C1–C4, Hz big-endian)
    fn send_cc(&mut self) -> Result<(), String> {
        let mut pkt = [0u8; P1_PACKET_LEN];

        pkt[0..3].copy_from_slice(&P1_OUTER_SYNC);
        pkt[3] = P1_ENDPOINT_H2D;
        pkt[4..8].copy_from_slice(&self.tx_seq.to_be_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);

        // Sub-frame 1: sample rate (address 0, C1[1:0] = speed code)
        write_subframe(&mut pkt[8..520], 0x00, [self.speed_code(), 0, 0, 0]);

        // Sub-frame 2: NCO frequency (address 1, C1–C4 = Hz big-endian)
        write_subframe(
            &mut pkt[520..1032],
            0x02,
            (self.center_freq_hz as u32).to_be_bytes(),
        );

        self.socket
            .send(&pkt)
            .map_err(|e| format!("HL2: send C&C failed: {e}"))?;
        Ok(())
    }

    fn speed_code(&self) -> u8 {
        match self.sample_rate_hz as u32 {
            96_000 => 0x01,
            192_000 => 0x02,
            384_000 => 0x03,
            _ => 0x00, // 48 kHz
        }
    }
}

impl Drop for HermesLite2Source {
    fn drop(&mut self) {
        if let Err(e) = self.send_run(false) {
            warn!("HL2: failed to send stop packet on drop: {e}");
        } else {
            debug!("HL2: stop packet sent");
        }
    }
}

impl IqSource for HermesLite2Source {
    fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        while self.pending.len() < max_samples {
            let mut buf = [0u8; P1_PACKET_LEN];
            match self.socket.recv(&mut buf) {
                Ok(len) if len == P1_PACKET_LEN => {
                    parse_ddc_packet(&buf, &mut self.pending);
                }
                Ok(len) => {
                    debug!("HL2: short packet ({len} bytes), discarding");
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(format!(
                        "HL2: receive timeout after {:?} — device not sending",
                        RECV_TIMEOUT
                    ));
                }
                Err(e) => return Err(format!("HL2: recv error: {e}")),
            }
        }
        let n = max_samples.min(self.pending.len());
        Ok(self.pending.drain(..n).collect())
    }

    fn set_center_frequency(&mut self, center_freq_hz: f32) -> Result<(), String> {
        self.center_freq_hz = center_freq_hz;
        info!("HL2: NCO → {} Hz", center_freq_hz as u32);
        self.send_cc()
    }

    fn keepalive(&mut self) {
        if let Err(e) = self.send_cc() {
            warn!("HL2: keepalive C&C failed: {e}");
        }
    }

    fn set_sample_rate(&mut self, sample_rate_hz: u32) -> Result<(), String> {
        self.sample_rate_hz = sample_rate_hz as f32;
        self.send_cc()
    }

    fn is_realtime(&self) -> bool {
        true
    }

    fn source_capabilities(&self) -> SourceCapabilities {
        hl2_source_capabilities()
    }

    fn source_control_state(&self) -> SourceControlState {
        SourceControlState {
            sample_rate_hz: self.sample_rate_hz as u32,
            ..SourceControlState::default()
        }
    }
}

/// Write a 512-byte sub-frame into `sf`: sync, C0, C1–C4, then zeros for TX IQ.
fn write_subframe(sf: &mut [u8], c0: u8, c1c4: [u8; 4]) {
    sf[0..3].copy_from_slice(&P1_SUBFRAME_SYNC);
    sf[3] = c0;
    sf[4..8].copy_from_slice(&c1c4);
    // sf[8..] stays zero — no TX IQ data for RX-only operation
}

/// Decode a 24-bit big-endian signed integer into i32.
fn i24_be(b: &[u8]) -> i32 {
    let raw = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
    // Sign-extend from bit 23
    if raw & 0x80_0000 != 0 {
        (raw | 0xFF00_0000) as i32
    } else {
        raw as i32
    }
}

/// Parse one 1032-byte DDC packet into Complex32 samples appended to `out`.
///
/// Each 512-byte sub-frame carries 63 × 8-byte samples:
///   bytes 0–2: I (24-bit signed big-endian)
///   bytes 3–5: Q (24-bit signed big-endian)
///   bytes 6–7: microphone (16-bit, ignored)
fn parse_ddc_packet(pkt: &[u8; P1_PACKET_LEN], out: &mut VecDeque<Complex32>) {
    if pkt[0..3] != P1_OUTER_SYNC || pkt[3] != P1_ENDPOINT_D2H {
        warn!(
            "HL2: unexpected packet header {:02x} {:02x} {:02x} {:02x}",
            pkt[0], pkt[1], pkt[2], pkt[3]
        );
        return;
    }

    for &sf_base in &P1_SUBFRAME_OFFSETS {
        let sf = &pkt[sf_base..sf_base + 512];
        if sf[0..3] != P1_SUBFRAME_SYNC {
            warn!("HL2: bad sub-frame sync at offset {sf_base}");
            continue;
        }
        for i in 0..P1_SAMPLES_PER_SUBFRAME {
            let b = 8 + i * 8;
            let i_f = i24_be(&sf[b..b + 3]) as f32 / (1u32 << 23) as f32;
            let q_f = i24_be(&sf[b + 3..b + 6]) as f32 / (1u32 << 23) as f32;
            out.push_back(Complex32::new(i_f, q_f));
        }
    }
}

/// Static source capabilities for the Hermes Lite 2.
/// Used both by the live source and by discovery (before the radio is acquired).
pub fn hl2_source_capabilities() -> SourceCapabilities {
    SourceCapabilities {
        supports_sample_rate: true,
        sample_rates_hz: vec![48_000, 96_000, 192_000, 384_000],
        tuner_freq_hz_min: 10_000,
        tuner_freq_hz_max: 30_000_000,
        ..SourceCapabilities::none()
    }
}
