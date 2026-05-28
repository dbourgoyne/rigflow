use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use log::{debug, info, warn};
use num_complex::Complex32;

use rigflow_core::radio::source_control::{GainMode, SourceCapabilities, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;

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

// HL2 P1 D2H status register accumulator.
//
// The HL2 sends status data in the C0/C1-C4 bytes of every DDC sub-frame,
// cycling through register addresses on successive packets.
//
// RADDR 0x00 (C0 = 0x00):
//   bits [7:0]   = firmware version
//   bits [14:8]  = TX IQ FIFO count MSBs
//   bit  [15]    = under/overflow recovery
//   bit  [24]    = RF ADC overload (active high)
//   bit  [25]    = TX inhibit (active low: 0 = inhibited)
//
// RADDR 0x01 (C0 = 0x02):
//   bits [15:0]  = forward power (raw ADC)
//   bits [31:16] = temperature (raw ADC)
//
// RADDR 0x02 (C0 = 0x04):
//   bits [15:0]  = current (raw ADC)
//   bits [31:16] = reverse power (raw ADC)
#[derive(Debug, Clone, Default)]
struct Hl2StatusRegs {
    // RADDR 0x00
    firmware_version: u8,
    adc_overload: bool,
    tx_inhibited: bool,
    /// 2-bit under/overflow recovery field from bits [15:14] of the status word.
    recovery_bits: u8,
    raddr0_valid: bool,

    // RADDR 0x01
    temperature_raw: u16,
    forward_power_raw: u16,
    raddr1_valid: bool,

    // RADDR 0x02
    reverse_power_raw: u16,
    current_raw: u16,
    raddr2_valid: bool,
}

// HL2 LNA gain at P1 address 0x0A (C0=0x14), extended-range mode (C1[6]=1).
// gain_db = code - 12 (code 0 = -12 dB, code 60 = +48 dB, 1 dB/step).
// Set C1 = 0x40 | code to enable extended range; without 0x40 the HL2 uses
// a backward-compat attenuator-only mode capped at +19 dB.
const DEFAULT_LNA_GAIN_CODE: u8 = 32; // 20 dB

pub struct HermesLite2Source {
    socket: UdpSocket,
    sample_rate_hz: f32,
    center_freq_hz: f32,
    lna_gain_code: u8,
    tx_seq: u32,
    pending: VecDeque<Complex32>,
    /// Accumulated status registers decoded from incoming DDC packet headers.
    status_regs: Hl2StatusRegs,
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
            lna_gain_code: DEFAULT_LNA_GAIN_CODE,
            tx_seq: 0,
            pending: VecDeque::new(),
            status_regs: Hl2StatusRegs::default(),
        };

        src.send_run(true)?;
        src.send_cc()?;
        src.send_gain_cc()?;
        info!(
            "HL2: P1 stream started from {device_addr} — \
             sample_rate={sample_rate_hz} Hz  center={center_freq_hz} Hz  \
             lna_gain={:.1} dB",
            DEFAULT_LNA_GAIN_CODE as f32 - 12.0
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

    /// Send a C&C packet carrying the current LNA gain code (address 9, C0=0x12).
    /// Repeats the NCO frequency in subframe 1 as an implicit keepalive.
    fn send_gain_cc(&mut self) -> Result<(), String> {
        let mut pkt = [0u8; P1_PACKET_LEN];
        pkt[0..3].copy_from_slice(&P1_OUTER_SYNC);
        pkt[3] = P1_ENDPOINT_H2D;
        pkt[4..8].copy_from_slice(&self.tx_seq.to_be_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);

        // Subframe 1: repeat NCO freq so hardware LO stays current
        write_subframe(&mut pkt[8..520], 0x02, (self.center_freq_hz as u32).to_be_bytes());
        // Subframe 2: RX LNA gain — address 0x0A (C0=0x14).
        // The 32-bit data word is big-endian: C1=MSB, C4=LSB.
        // Bit 6 and bits 5:0 of the DATA word live in C4 (the LSB byte).
        // C4[6]=1 enables extended range; C4[5:0] = gain code.
        write_subframe(&mut pkt[520..1032], 0x14, [0, 0, 0, 0x40 | (self.lna_gain_code & 0x3F)]);

        self.socket
            .send(&pkt)
            .map_err(|e| format!("HL2: send gain C&C failed: {e}"))?;
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
                    parse_ddc_packet(&buf, &mut self.pending, &mut self.status_regs);
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
        // send_gain_cc carries both the NCO freq and the current gain code so
        // the gain register is always in sync after every tune.
        self.send_gain_cc()
    }

    fn keepalive(&mut self) {
        // Use send_gain_cc so the gain register is refreshed on every keepalive,
        // not just when the user explicitly changes it.
        if let Err(e) = self.send_gain_cc() {
            warn!("HL2: keepalive C&C failed: {e}");
        }
    }

    fn set_sample_rate(&mut self, sample_rate_hz: u32) -> Result<(), String> {
        self.sample_rate_hz = sample_rate_hz as f32;
        // send_cc sets the speed code; follow with send_gain_cc to also apply
        // the current gain to the updated configuration.
        self.send_cc()?;
        self.send_gain_cc()
    }

    fn set_gain_db(&mut self, gain_db: f32) -> Result<(), String> {
        // code = gain_db + 12: code 0 = -12 dB, code 60 = +48 dB
        self.lna_gain_code = (gain_db + 12.0).round().clamp(0.0, 60.0) as u8;
        info!("HL2: LNA gain → {:.1} dB (code {})", gain_db, self.lna_gain_code);
        self.send_gain_cc()
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
            gain_mode: GainMode::Manual,
            gain_db: self.lna_gain_code as f32 - 12.0,
            ..SourceControlState::default()
        }
    }

    fn source_status(&self) -> SourceStatus {
        hl2_status_regs_to_source_status(&self.status_regs)
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

/// Parse one 1032-byte DDC packet into Complex32 samples appended to `out`,
/// and update `status` with any status register data found in sub-frame headers.
///
/// Each 512-byte sub-frame layout:
///   bytes 0–2:  sub-frame sync 0x7F 0x7F 0x7F
///   byte  3:    C0 — D2H register address encoding: RADDR = C0 >> 1
///   bytes 4–7:  C1..C4 — status word (C1=MSB, C4=LSB)
///   bytes 8–511: 63 × 8-byte DDC samples
///     [0..3]: I sample (24-bit signed big-endian)
///     [3..6]: Q sample (24-bit signed big-endian)
///     [6..8]: microphone (16-bit, ignored)
fn parse_ddc_packet(
    pkt: &[u8; P1_PACKET_LEN],
    out: &mut VecDeque<Complex32>,
    status: &mut Hl2StatusRegs,
) {
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

        // Decode status header (C0 / C1–C4).
        let c0 = sf[3];
        let word = ((sf[4] as u32) << 24)
            | ((sf[5] as u32) << 16)
            | ((sf[6] as u32) << 8)
            | (sf[7] as u32);
        let raddr = c0 >> 1; // bits [7:1] encode the register address

        match raddr {
            0x00 => {
                status.firmware_version = (word & 0xFF) as u8;
                // Bits [15:14]: 2-bit under/overflow recovery code.
                status.recovery_bits = ((word >> 14) & 0x3) as u8;
                status.adc_overload = (word >> 24) & 1 == 1;
                // Tx inhibit is active-low: bit=0 means TX is inhibited.
                status.tx_inhibited = (word >> 25) & 1 == 0;
                status.raddr0_valid = true;
            }
            0x01 => {
                status.forward_power_raw = (word & 0xFFFF) as u16;
                status.temperature_raw = ((word >> 16) & 0xFFFF) as u16;
                status.raddr1_valid = true;
            }
            0x02 => {
                status.current_raw = (word & 0xFFFF) as u16;
                status.reverse_power_raw = ((word >> 16) & 0xFFFF) as u16;
                status.raddr2_valid = true;
            }
            _ => {}
        }

        // Decode IQ samples.
        for i in 0..P1_SAMPLES_PER_SUBFRAME {
            let b = 8 + i * 8;
            let i_f = i24_be(&sf[b..b + 3]) as f32 / (1u32 << 23) as f32;
            let q_f = i24_be(&sf[b + 3..b + 6]) as f32 / (1u32 << 23) as f32;
            out.push_back(Complex32::new(i_f, q_f));
        }
    }
}

/// Convert accumulated HL2 status register snapshot into a generic `SourceStatus`.
///
/// Calibration notes:
/// - `firmware_version`: exact binary version reported by HL2 firmware.
/// - `temperature_c`: approximated from the raw 16-bit ADC reading using
///   a linear formula typical for HPSDR-class hardware.  The exact slope
///   depends on your HL2 board revision — treat this as indicative (±5 °C).
///   TODO: verify formula against your hardware and adjust if needed.
/// - `forward_power_w` / `reverse_power_w` / `current_a`: hardware-specific
///   calibration constants are not yet known.  These fields are left as `None`
///   and the raw ADC values are logged at TRACE level for future calibration.
///   TODO: add calibration once forward-power detector and current-shunt
///   specs are confirmed for your HL2 board revision.
fn hl2_status_regs_to_source_status(r: &Hl2StatusRegs) -> SourceStatus {
    let firmware_version = if r.raddr0_valid {
        let major = r.firmware_version / 10;
        let minor = r.firmware_version % 10;
        Some(format!("{major}.{minor}"))
    } else {
        None
    };

    let adc_overload = r.raddr0_valid.then_some(r.adc_overload);
    let tx_inhibited = r.raddr0_valid.then_some(r.tx_inhibited);
    let recovery_status = if r.raddr0_valid {
        let label = match r.recovery_bits {
            0b00 => "OK",
            0b10 => "UNDERFLOW",
            0b11 => "OVERFLOW",
            _    => "UNKNOWN",
        };
        Some(label.to_string())
    } else {
        None
    };

    // Temperature: approximate formula for HPSDR-class hardware.
    // raw is a 16-bit value; we use an affine mapping calibrated roughly
    // around typical AD9866 / LM19 analog paths.
    // TODO: verify with your HL2 board revision.
    let temperature_c = if r.raddr1_valid {
        let raw = r.temperature_raw as f32;
        // Approximate: maps ~20 °C at raw≈2800 and ~70 °C at raw≈3500
        Some((raw / 65536.0) * 3.3 / 0.01 - 50.0)
    } else {
        None
    };

    // Forward / reverse power and current require board-specific calibration.
    // Log raw values at TRACE for future calibration work.
    if r.raddr1_valid {
        log::trace!(
            "HL2 status: fwd_raw={} temp_raw={}",
            r.forward_power_raw,
            r.temperature_raw
        );
    }
    if r.raddr2_valid {
        log::trace!(
            "HL2 status: rev_raw={} cur_raw={}",
            r.reverse_power_raw,
            r.current_raw
        );
    }

    // TODO: add board-specific calibration for forward_power_w, reverse_power_w,
    // current_a once hardware constants are confirmed.
    let forward_power_w: Option<f32> = None;
    let reverse_power_w: Option<f32> = None;
    let current_a: Option<f32> = None;

    // SWR is only valid when forward power is above a minimal threshold.
    let swr = compute_swr(forward_power_w, reverse_power_w);

    SourceStatus {
        firmware_version,
        adc_overload,
        temperature_c,
        current_a,
        forward_power_w,
        reverse_power_w,
        swr,
        tx_inhibited,
        recovery_status,
    }
}

/// Compute SWR from forward and reverse power (both in watts).
///
/// Returns `None` when:
/// - either power value is `None`
/// - forward power is below 0.1 W (not transmitting / noise floor)
/// - reverse power is negative or >= forward power (invalid reading)
///
/// Formula: SWR = (1 + √(Pr/Pf)) / (1 − √(Pr/Pf))
fn compute_swr(forward_w: Option<f32>, reverse_w: Option<f32>) -> Option<f32> {
    let fwd = forward_w?;
    let rev = reverse_w?;

    const MIN_FORWARD_W: f32 = 0.1;
    if fwd < MIN_FORWARD_W || rev < 0.0 || rev >= fwd {
        return None;
    }

    let gamma = (rev / fwd).sqrt();
    let denominator = 1.0 - gamma;
    if denominator.abs() < f32::EPSILON {
        return None;
    }

    Some(((1.0 + gamma) / denominator).clamp(1.0, 999.0))
}

/// Static source capabilities for the Hermes Lite 2.
/// Used both by the live source and by discovery (before the radio is acquired).
pub fn hl2_source_capabilities() -> SourceCapabilities {
    SourceCapabilities {
        supports_sample_rate: true,
        sample_rates_hz: vec![48_000, 96_000, 192_000, 384_000],
        supports_gain: true,
        // HL2 AD9866 extended range: -12 dB to +48 dB in 1 dB steps
        gain_values_db: (-12..=48).map(|i| i as f32).collect(),
        tuner_freq_hz_min: 10_000,
        tuner_freq_hz_max: 30_000_000,
        ..SourceCapabilities::none()
    }
}
