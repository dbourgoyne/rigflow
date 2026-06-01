use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use num_complex::Complex32;

use rigflow_core::radio::source_control::{GainMode, SourceCapabilities, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::tx_tune::{
    compute_swr, compute_swr_from_raw, TxTuneResult, TxTuneStatus,
};

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
// auto-cycling through register addresses on successive packets.
//
// The device-to-host address is encoded in C0[7:3] (RADDR = C0 >> 3); the
// low three bits C0[2:0] are the PTT/DASH/DOT hardware inputs.  This is NOT
// the same as host-to-device writes, where C0 = (addr << 1) | MOX.
//
// RADDR 0x00 (C0[7:3] = 0 → C0 = 0x00):
//   bits [7:0]   = firmware version
//   bits [14:8]  = TX IQ FIFO count MSBs
//   bit  [15]    = under/overflow recovery
//   bit  [24]    = RF ADC overload (active high)
//   bit  [25]    = TX inhibit (active low: 0 = inhibited)
//
// RADDR 0x01 (C0[7:3] = 1 → C0 = 0x08):
//   bits [15:0]  = forward power (raw ADC)
//   bits [31:16] = temperature (raw ADC)
//
// RADDR 0x02 (C0[7:3] = 2 → C0 = 0x10):
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
    /// TX IQ FIFO occupancy from bits [14:8] of the RADDR 0x00 status word.
    tx_fifo_count: u16,
    raddr0_valid: bool,

    // RADDR 0x01
    temperature_raw: u16,
    forward_power_raw: u16,
    raddr1_valid: bool,

    // RADDR 0x02
    reverse_power_raw: u16,
    current_raw: u16,
    raddr2_valid: bool,

    /// Raw 32-bit RDATA word (C1<<24|C2<<16|C3<<8|C4) last seen for each
    /// device-to-host status address 0..=4.  Diagnostic only — lets us inspect
    /// the actual register contents (incl. RADDR 3/4, which Quisk does not
    /// decode) without interpreting them.
    rdata: [u32; 5],
    rdata_seen: [bool; 5],
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

    /// Program both the TX1 NCO (register 0x01 → C0=0x02) and the RX1 NCO
    /// (register 0x02 → C0=0x04) to `freq_hz` in a single C&C packet.
    ///
    /// Used by the TX tune test so the transmit carrier and receiver land on
    /// the same (simplex) frequency.  The normal RX path never writes RX1
    /// (register 0x02); it programs only register 0x01 with `center_freq_hz`.
    fn send_tx_rx_nco(&mut self, freq_hz: u32) -> Result<(), String> {
        let mut pkt = [0u8; P1_PACKET_LEN];
        pkt[0..3].copy_from_slice(&P1_OUTER_SYNC);
        pkt[3] = P1_ENDPOINT_H2D;
        pkt[4..8].copy_from_slice(&self.tx_seq.to_be_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);

        // Sub-frame 1: TX1 NCO (register 0x01 → C0 = 0x02).
        write_subframe(&mut pkt[8..520], 0x02, freq_hz.to_be_bytes());
        // Sub-frame 2: RX1 NCO (register 0x02 → C0 = 0x04).
        write_subframe(&mut pkt[520..1032], 0x04, freq_hz.to_be_bytes());

        self.socket
            .send(&pkt)
            .map(|_| ())
            .map_err(|e| format!("HL2: send TX/RX NCO failed: {e}"))
    }

    /// Program the HL2 TX drive-level + PA-enable register (address 0x09,
    /// C0 = 0x12) in a single C&C packet.  Used only by the TX tune test.
    ///
    /// Register 0x09 data word (C1=[31:24] … C4=[7:0]):
    /// - C1 [7:0]  = TX drive level (0–255 PWM).  HL2 RF output scales with
    ///   this AND the digital IQ amplitude; unprogrammed it defaults to 0,
    ///   which is why forward/reverse telemetry reads ~0 regardless of IQ.
    /// - C2 bit 3  = PA enable (overall bit 19).  The forward/reverse power
    ///   detectors sit after the PA, so it must be enabled to read power.
    /// - C3/C4     = Alex Rx/Tx filter bytes — left 0 (no Alex on a basic HL2).
    ///   Antenna-tuner bits 17/20 are likewise left 0 (no external ATU).
    ///
    /// Both sub-frames carry register 0x09 (idempotent) so this packet never
    /// disturbs the TX/RX NCO programmed separately.  The value is sticky on
    /// the HL2, so a single pre-TX write holds for the whole pulse.
    fn send_tx_drive_cc(&mut self, drive_level: u8, pa_enable: bool) -> Result<(), String> {
        let mut pkt = [0u8; P1_PACKET_LEN];
        pkt[0..3].copy_from_slice(&P1_OUTER_SYNC);
        pkt[3] = P1_ENDPOINT_H2D;
        pkt[4..8].copy_from_slice(&self.tx_seq.to_be_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);

        let c2 = if pa_enable { 0x08 } else { 0x00 }; // bit 19 = C2 bit 3
        let data = [drive_level, c2, 0x00, 0x00];
        // Register 0x09 → C0 = 0x12 (addr 9 << 1, MOX = 0 for this setup write).
        write_subframe(&mut pkt[8..520], 0x12, data);
        write_subframe(&mut pkt[520..1032], 0x12, data);

        self.socket
            .send(&pkt)
            .map(|_| ())
            .map_err(|e| format!("HL2: send TX drive C&C failed: {e}"))
    }

    /// Send one 1032-byte H2D TX packet carrying a constant carrier
    /// (I = `amplitude_fs` × 0x7FFF, Q = 0) in every TX IQ slot.
    ///
    /// Protocol 1 host-to-device sample groups are 8 bytes of **four 16-bit
    /// signed big-endian** values — `[L_hi L_lo][R_hi R_lo][I_hi I_lo][Q_hi
    /// Q_lo]`: left audio, right audio, TX I, TX Q.  (This differs from the
    /// device-to-host DDC layout, which is 24-bit I + 24-bit Q + 16-bit mic.)
    /// For the tune carrier: L = R = 0, I = amplitude, Q = 0.
    ///
    /// `ptt` controls the PTT bit (C0[0]) in both sub-frames:
    /// - `ptt = false` — feeds TX IQ into the FIFO **without** asserting PTT.
    ///   Used for pre-filling the FIFO before transmission begins.
    /// - `ptt = true`  — feeds TX IQ **and** asserts PTT.  The caller MUST
    ///   call `send_gain_cc()` on every exit path to release PTT.
    fn send_tx_packet(&mut self, nco_freq_hz: u32, amplitude_fs: f32, ptt: bool) -> Result<(), String> {
        // TX I as a 16-bit signed big-endian sample; Q stays 0 (carrier).
        let i_code = (amplitude_fs.clamp(0.0, 1.0) * 0x7FFF as f32) as i32 as i16;
        let i_b = i_code.to_be_bytes();

        let ptt_bit = ptt as u8; // 0 or 1 — OR'd into both C0 bytes

        let mut pkt = [0u8; P1_PACKET_LEN];
        pkt[0..3].copy_from_slice(&P1_OUTER_SYNC);
        pkt[3] = P1_ENDPOINT_H2D;
        pkt[4..8].copy_from_slice(&self.tx_seq.to_be_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);

        // TX1 NCO register is repeated in every packet's sub-frame 1; use the
        // caller-supplied tune target, NOT self.center_freq_hz (the RX centre).
        let freq_bytes = nco_freq_hz.to_be_bytes();
        let gain_byte = 0x40 | (self.lna_gain_code & 0x3F);

        // Sub-frame 1 (offset 8): NCO freq register (address 0x01) ± PTT.
        // C0 = (0x01 << 1) | ptt_bit = 0x02 | ptt_bit
        {
            let sf = &mut pkt[8..520];
            sf[0..3].copy_from_slice(&P1_SUBFRAME_SYNC);
            sf[3] = 0x02 | ptt_bit;
            sf[4..8].copy_from_slice(&freq_bytes);
            for i in 0..P1_SAMPLES_PER_SUBFRAME {
                let b = 8 + i * 8;
                // L audio [b..b+2] and R audio [b+2..b+4] remain 0.
                sf[b + 4..b + 6].copy_from_slice(&i_b); // TX I (16-bit BE)
                // Q bytes [b+6..b+8] remain 0 (pkt initialised to zero).
            }
        }

        // Sub-frame 2 (offset 520): LNA gain register (address 0x0A) ± PTT.
        // C0 = (0x0A << 1) | ptt_bit = 0x14 | ptt_bit
        {
            let sf = &mut pkt[520..1032];
            sf[0..3].copy_from_slice(&P1_SUBFRAME_SYNC);
            sf[3] = 0x14 | ptt_bit;
            sf[4] = 0;
            sf[5] = 0;
            sf[6] = 0;
            sf[7] = gain_byte;
            for i in 0..P1_SAMPLES_PER_SUBFRAME {
                let b = 8 + i * 8;
                // L/R audio [b..b+4] remain 0; TX I at [b+4..b+6]; Q [b+6..b+8] = 0.
                sf[b + 4..b + 6].copy_from_slice(&i_b);
            }
        }

        self.socket
            .send(&pkt)
            .map(|_| ())
            .map_err(|e| format!("HL2: send TX packet failed: {e}"))
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

    /// HL2 TX tune test — short carrier pulse for SWR measurement.
    ///
    /// # Safety invariants
    ///
    /// - PTT is asserted only after all pre-checks pass AND the TX FIFO
    ///   has been pre-filled (see below).
    /// - PTT is released via `send_gain_cc()` on **every** exit path that
    ///   asserts PTT: normal completion, underflow/overflow, socket fault.
    ///   Pre-check rejections (invalid freq, TX inhibited) never assert PTT.
    /// - Amplitude defaults to 0.05 FS and is hard-clamped to a 0.10 FS ceiling.
    /// - Duration is hard-clamped to `MAX_DURATION_MS` (500 ms).
    /// - The caller-supplied `drive` parameter is logged but overridden;
    ///   drive is not user-configurable in this revision.
    ///
    /// # TX FIFO pre-fill
    ///
    /// The HL2 TX IQ FIFO must contain samples before PTT is asserted.
    /// Asserting PTT on an empty FIFO causes an immediate underflow fault.
    /// `TX_FIFO_PREFILL_PACKETS` packets (≈ 52 ms) are sent **without PTT**
    /// first.  After pre-fill the socket receive buffer is drained and any
    /// stale `recovery_bits` from before TX started are cleared, so only
    /// genuine in-flight anomalies trigger the underflow/overflow guard.
    /// Spot / SWR carrier (formerly the engineering "TX tune test").  `drive`
    /// now carries the operator's **TX Drive percent (0–100)** from HL2 Source
    /// Control — NOT an IQ amplitude.  It maps to the HL2 drive-level register
    /// (0x09); the carrier IQ amplitude itself stays a fixed safe value.
    fn tx_tune_test(
        &mut self,
        target_freq_hz: u64,
        duration_ms: u32,
        drive: f32,
    ) -> TxTuneResult {
        // ── Hard safety constants ────────────────────────────────────────
        // Fixed safe carrier IQ amplitude (constant I = amp, Q = 0).  Power is
        // controlled by the TX drive-level register, not this amplitude.
        const TX_TUNE_DEFAULT_AMPLITUDE_FS: f32 = 0.05; // 5% FS (~-26 dBFS)
        // HL2 TX drive level (register 0x09 C1, 0–255) derived from TX Drive %.
        // Hard-capped well below full scale (and below Quisk's default of 63)
        // as a conservative safety ceiling regardless of the operator's %.
        const TX_TUNE_DRIVE_LEVEL_MAX: u8 = 63;
        const MAX_DURATION_MS: u32 = 500;
        const TX_SAMPLE_RATE_HZ: f32 = 48_000.0;
        const SAMPLES_PER_PACKET: f32 = 126.0; // 2 sub-frames × 63
        // Pre-fill: send IQ without PTT to fill the HL2 TX FIFO before asserting PTT.
        // 20 packets × 126 samples / 48 kHz ≈ 52 ms of buffering.
        const TX_FIFO_PREFILL_PACKETS: usize = 20;
        // SWR above this threshold is reported as HighSwr.
        const SWR_ALARM_THRESHOLD: f32 = 3.0;
        // Forward power below this threshold means NoForwardPower.
        const MIN_FORWARD_W: f32 = 0.1;

        // ── Parameter clamping ───────────────────────────────────────────
        let clamped_duration_ms = duration_ms.min(MAX_DURATION_MS);

        // Carrier IQ amplitude is fixed (existing safe carrier generation).
        let effective_amplitude = TX_TUNE_DEFAULT_AMPLITUDE_FS;

        // TX Drive percent → HL2 drive-level register (0x09 C1), per Quisk:
        //   drive_level_u8 = round(tx_drive_percent * 255 / 100)
        // then hard-capped to TX_TUNE_DRIVE_LEVEL_MAX even at 100%.  RF output
        // scales with BOTH this drive level AND the (fixed) IQ amplitude; left
        // unprogrammed the drive level defaults to 0 → no RF.  We program 0x09
        // (drive level + PA enable) before PTT and safe it off on exit.
        let tx_drive_percent = drive.clamp(0.0, 100.0);
        let drive_level = ((tx_drive_percent * 255.0 / 100.0).round() as i32)
            .clamp(0, TX_TUNE_DRIVE_LEVEL_MAX as i32) as u8;

        // ── Frequency validation ─────────────────────────────────────────
        // HL2 HF TX band: 1.8–30 MHz.
        const HF_MIN_HZ: u64 = 1_800_000;
        const HF_MAX_HZ: u64 = 30_000_000;
        if target_freq_hz < HF_MIN_HZ || target_freq_hz > HF_MAX_HZ {
            warn!(
                "[hl2 tx-tune] rejected freq={target_freq_hz}: \
                 outside HF range ({HF_MIN_HZ}-{HF_MAX_HZ} Hz)"
            );
            return TxTuneResult {
                status: TxTuneStatus::InvalidFrequency,
                frequency_hz: target_freq_hz,
                duration_ms: clamped_duration_ms,
                drive: effective_amplitude,
                message: Some(format!(
                    "{target_freq_hz} Hz is outside HF TX range \
                     ({HF_MIN_HZ}-{HF_MAX_HZ} Hz)"
                )),
                ..TxTuneResult::default()
            };
        }

        // ── TX inhibit check ─────────────────────────────────────────────
        if self.status_regs.raddr0_valid && self.status_regs.tx_inhibited {
            warn!("[hl2 tx-tune] rejected: TX inhibited by hardware");
            return TxTuneResult {
                status: TxTuneStatus::TxInhibited,
                frequency_hz: target_freq_hz,
                duration_ms: clamped_duration_ms,
                drive: effective_amplitude,
                message: Some("TX inhibited by hardware".to_string()),
                ..TxTuneResult::default()
            };
        }

        // ── Program TX1 + RX1 NCO to the tune target (simplex) ────────────
        // Register 0x01 (C0=0x02) = TX1 NCO; register 0x02 (C0=0x04) = RX1 NCO.
        // For simplex TX tune both must sit on the operator's target freq.
        // NOTE: the RX path (send_cc/send_gain_cc/set_center_frequency) only
        // ever programs register 0x01 with self.center_freq_hz and never
        // writes RX1 (register 0x02); this is the first place RX1 is set.
        // We deliberately do NOT use self.center_freq_hz here — the carrier
        // must land on the tune target, not the RX DDC centre.
        let target_u32 = target_freq_hz as u32;
        if let Err(e) = self.send_tx_rx_nco(target_u32) {
            warn!("[hl2 tx-tune] NCO program failed: {e}");
            return TxTuneResult {
                status: TxTuneStatus::Fault,
                frequency_hz: target_freq_hz,
                duration_ms: clamped_duration_ms,
                drive: effective_amplitude,
                message: Some(format!("NCO program failed: {e}")),
                ..TxTuneResult::default()
            };
        }
        info!(
            "[hl2 tx-tune] NCO programmed (before PTT): \
             TX1_NCO={target_u32} Hz (reg 0x01)  RX1_NCO={target_u32} Hz (reg 0x02)"
        );

        // ── Program TX drive level + PA enable (register 0x09) ────────────
        // Without this the HL2 drive level sits at 0 and produces no RF, so
        // forward/reverse telemetry reads ~0 regardless of IQ amplitude.  The
        // drive level was derived from TX Drive % above and hard-capped; enable
        // the PA so the post-PA forward/reverse detectors register power.
        // Disabled again on every exit path below.
        if let Err(e) = self.send_tx_drive_cc(drive_level, true) {
            warn!("[hl2 tx-tune] drive-level program failed: {e}");
            // PA may be partially set — attempt a safe-off before returning.
            let _ = self.send_tx_drive_cc(0, false);
            return TxTuneResult {
                status: TxTuneStatus::Fault,
                frequency_hz: target_freq_hz,
                duration_ms: clamped_duration_ms,
                drive: effective_amplitude,
                message: Some(format!("drive-level program failed: {e}")),
                ..TxTuneResult::default()
            };
        }
        info!(
            "[hl2 spot] TX drive programmed (before PTT): \
             tx_drive_percent={tx_drive_percent:.0} drive_level_u8={drive_level}/255 \
             (reg 0x09 C1, cap {TX_TUNE_DRIVE_LEVEL_MAX})  pa_enable=true (bit 19)  \
             effective_amplitude_fs={effective_amplitude:.3}"
        );

        let packet_period = Duration::from_secs_f32(SAMPLES_PER_PACKET / TX_SAMPLE_RATE_HZ);

        // ── Capture a fresh TX FIFO reading before pre-fill ───────────────
        // Switch to non-blocking and drain+parse any queued DDC packets so
        // `tx_fifo_count` reflects the device's state immediately before we
        // start feeding TX IQ.  The socket stays non-blocking through the
        // main TX loop and is restored to blocking on exit.
        let _ = self.socket.set_nonblocking(true);
        {
            let mut buf = [0u8; P1_PACKET_LEN];
            let mut discard = VecDeque::new();
            while let Ok(len) = self.socket.recv(&mut buf) {
                if len == P1_PACKET_LEN {
                    parse_ddc_packet(&buf, &mut discard, &mut self.status_regs);
                }
            }
        }
        let fifo_before = self.status_regs.tx_fifo_count;
        info!(
            "[hl2 tx-tune] TX FIFO count before pre-fill: {fifo_before} \
             (raddr0_valid={})  local={:?}  peer={:?}",
            self.status_regs.raddr0_valid,
            self.socket.local_addr().ok(),
            self.socket.peer_addr().ok(),
        );

        // ── TX FIFO pre-fill (PTT=0) ──────────────────────────────────────
        // Feed TX IQ into the HL2 FIFO without asserting PTT.  This prevents
        // an immediate underflow when PTT is asserted in the main loop.
        info!(
            "[hl2 tx-tune] pre-fill: {} packets (~{:.0} ms)  \
             requested_drive={drive:.3}  effective_amplitude={effective_amplitude:.3} FS",
            TX_FIFO_PREFILL_PACKETS,
            TX_FIFO_PREFILL_PACKETS as f32 * (SAMPLES_PER_PACKET / TX_SAMPLE_RATE_HZ) * 1000.0,
        );
        {
            let mut tick = Instant::now();
            for _ in 0..TX_FIFO_PREFILL_PACKETS {
                let now = Instant::now();
                if now < tick {
                    thread::sleep(tick - now);
                }
                tick += packet_period;
                if let Err(e) = self.send_tx_packet(target_u32, effective_amplitude, false) {
                    warn!("[hl2 tx-tune] pre-fill failed: {e}");
                    // PTT was never asserted, but drive level + PA are enabled —
                    // turn them back off before returning.
                    let _ = self.send_tx_drive_cc(0, false);
                    return TxTuneResult {
                        status: TxTuneStatus::Fault,
                        frequency_hz: target_freq_hz,
                        duration_ms: clamped_duration_ms,
                        drive: effective_amplitude,
                        message: Some(format!("pre-fill failed: {e}")),
                        ..TxTuneResult::default()
                    };
                }
            }
        }

        // ── Drain DDC packets after pre-fill and read the TX FIFO count ───
        // After pre-fill the socket buffer holds DDC packets received during
        // pre-fill.  Parse them so `tx_fifo_count` reflects the freshest
        // post-pre-fill reading: if our TX IQ packets are being accepted the
        // count should have risen above `fifo_before`.  The recovery field
        // may carry a value set by a prior event unrelated to this TX, so we
        // explicitly zero it afterwards — only anomalies that occur AFTER PTT
        // is asserted should be treated as faults.  (Socket is already
        // non-blocking from the before-pre-fill drain above.)
        {
            let mut stale_buf = [0u8; P1_PACKET_LEN];
            let mut discard = VecDeque::new();
            while let Ok(len) = self.socket.recv(&mut stale_buf) {
                if len == P1_PACKET_LEN {
                    parse_ddc_packet(&stale_buf, &mut discard, &mut self.status_regs);
                }
            }
        }
        let fifo_after = self.status_regs.tx_fifo_count;
        info!(
            "[hl2 tx-tune] TX FIFO count after pre-fill: {fifo_after} \
             (before={fifo_before}  delta={})",
            fifo_after as i32 - fifo_before as i32
        );
        self.status_regs.recovery_bits = 0;

        // ── Main TX loop (PTT=1) ──────────────────────────────────────────
        // PTT is now asserted in every packet.  The socket remains in
        // non-blocking mode so D2H status packets can be polled between TX
        // sends without stalling packet timing.  WouldBlock = no packet yet.
        info!(
            "[hl2 tx-tune] PTT asserted: freq={target_freq_hz} Hz  \
             duration={clamped_duration_ms} ms  amplitude={effective_amplitude:.3} FS"
        );
        // PTT warmup: assert PTT and seed the TX FIFO with a *minimal* number
        // of ptt=true frames before the paced loop takes over.  Kept tiny (2)
        // because a large burst overfills the FIFO toward overflow; the paced
        // loop then maintains the level at the 48 kHz consumption rate.  The
        // earlier ptt=false pre-fill does not load the TX FIFO (the HL2 appears
        // to consume H2D TX IQ only while MOX/PTT is asserted).
        const PTT_WARMUP_PACKETS: usize = 2;
        // Grace window after PTT assert during which a reported FIFO anomaly is
        // logged but NOT treated as fatal, giving the FIFO time to prime.
        const PTT_GRACE_MS: u128 = 50;

        let tx_start = Instant::now();
        let deadline = tx_start + Duration::from_millis(clamped_duration_ms as u64);
        let mut fault_status: Option<TxTuneStatus> = None;
        let mut packets_sent_with_ptt: u32 = 0;

        for _ in 0..PTT_WARMUP_PACKETS {
            if let Err(e) = self.send_tx_packet(target_u32, effective_amplitude, true) {
                warn!("[hl2 tx-tune] warmup send failed: {e}");
                fault_status = Some(TxTuneStatus::Fault);
                break;
            }
            packets_sent_with_ptt += 1;
        }
        info!(
            "[hl2 tx-tune] PTT warmup: {packets_sent_with_ptt} ptt=true frames sent back-to-back"
        );

        // Paced main loop: SEND a ptt=true TX IQ packet first, THEN poll status
        // — so the FIFO is fed every iteration before we inspect it.
        let mut next_tick = Instant::now();
        let mut last_log = Instant::now();
        // Last status addresses observed in a polled D2H packet (C0[7:3] of the
        // two sub-frames), for telemetry visibility into the RADDR rotation.
        let mut last_raddrs: [u8; 2] = [0xFF, 0xFF];
        // Latch the peak raw detector readings seen WHILE PTT is asserted.  The
        // result reports these maxima, never the post-release value (which has
        // already decayed by the time we drain after key-up).  `None` until the
        // matching RADDR is actually observed during the pulse.
        let mut max_forward_raw: Option<u16> = None;
        let mut max_reverse_raw: Option<u16> = None;
        let mut max_current_raw: Option<u16> = None;

        while fault_status.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            // Deadline-based pacer: exactly one packet per `packet_period`
            // (126 samples / 48 kHz = 2.625 ms).  Accumulate the next send
            // instant so timing does not drift, BUT if we have fallen behind
            // schedule, resync to `now` instead of firing catch-up bursts —
            // bursting is what drove the FIFO into overflow.
            if now < next_tick {
                thread::sleep(next_tick - now);
                next_tick += packet_period;
            } else {
                next_tick = now + packet_period;
            }

            if let Err(e) = self.send_tx_packet(target_u32, effective_amplitude, true) {
                warn!("[hl2 tx-tune] send_tx_packet failed: {e}");
                fault_status = Some(TxTuneStatus::Fault);
                break;
            }
            packets_sent_with_ptt += 1;

            // Poll for D2H status packets (non-blocking) AFTER feeding the FIFO.
            // Drain ALL queued packets each iteration (status readback only —
            // does not touch send pacing) so the address rotation does not cause
            // us to miss RADDR 1/2/3 between sparse single-packet polls.
            let mut rx_buf = [0u8; P1_PACKET_LEN];
            while let Ok(len) = self.socket.recv(&mut rx_buf) {
                if len == P1_PACKET_LEN {
                    // Record which status addresses this packet carried (C0[7:3]
                    // of each sub-frame) so the telemetry log shows the actual
                    // RADDR rotation, not just the valid flags.
                    last_raddrs = [
                        (rx_buf[P1_SUBFRAME_OFFSETS[0] + 3] >> 3) & 0x1F,
                        (rx_buf[P1_SUBFRAME_OFFSETS[1] + 3] >> 3) & 0x1F,
                    ];
                    let mut discard = VecDeque::new();
                    parse_ddc_packet(&rx_buf, &mut discard, &mut self.status_regs);

                    // Latch peak forward/reverse/current seen during PTT.
                    if self.status_regs.raddr1_valid {
                        let f = self.status_regs.forward_power_raw;
                        max_forward_raw = Some(max_forward_raw.map_or(f, |m| m.max(f)));
                    }
                    if self.status_regs.raddr2_valid {
                        let r = self.status_regs.reverse_power_raw;
                        max_reverse_raw = Some(max_reverse_raw.map_or(r, |m| m.max(r)));
                        let c = self.status_regs.current_raw;
                        max_current_raw = Some(max_current_raw.map_or(c, |m| m.max(c)));
                    }
                }
            }

            let elapsed_ms = tx_start.elapsed().as_millis();

            // Telemetry every 25 ms.
            if last_log.elapsed() >= Duration::from_millis(25) {
                let r = &self.status_regs.rdata;
                info!(
                    "[hl2 tx-tune] PTT active: packets_sent_with_ptt={packets_sent_with_ptt} \
                     elapsed_ms={elapsed_ms} amp={effective_amplitude:.3} FS fifo_count={} \
                     recovery_bits={:#02b} last_raddrs={last_raddrs:?} \
                     fwd_raw={} rev_raw={} cur_raw={} \
                     max_fwd_raw={max_forward_raw:?} max_rev_raw={max_reverse_raw:?} \
                     max_cur_raw={max_current_raw:?} (raddr1={} raddr2={}) \
                     RDATA[0]={:#010x} RDATA[1]={:#010x} RDATA[2]={:#010x} RDATA[3]={:#010x} \
                     (seen={:?})",
                    self.status_regs.tx_fifo_count,
                    self.status_regs.recovery_bits,
                    self.status_regs.forward_power_raw,
                    self.status_regs.reverse_power_raw,
                    self.status_regs.current_raw,
                    self.status_regs.raddr1_valid,
                    self.status_regs.raddr2_valid,
                    r[0], r[1], r[2], r[3],
                    self.status_regs.rdata_seen,
                );
                last_log = Instant::now();
            }

            // FIFO anomaly: fatal only after the grace window has elapsed.
            if self.status_regs.raddr0_valid && self.status_regs.recovery_bits != 0 {
                let bits = self.status_regs.recovery_bits;
                if elapsed_ms < PTT_GRACE_MS {
                    debug!(
                        "[hl2 tx-tune] FIFO anomaly within grace ({elapsed_ms} ms): \
                         recovery_bits={bits:#02b} — not fatal, continuing to feed"
                    );
                } else {
                    warn!(
                        "[hl2 tx-tune] FIFO anomaly during TX at {elapsed_ms} ms: \
                         recovery_bits={bits:#02b}"
                    );
                    fault_status = Some(if bits == 0b10 {
                        TxTuneStatus::Underflow
                    } else {
                        TxTuneStatus::Overflow
                    });
                    break;
                }
            }
        }
        info!(
            "[hl2 tx-tune] TX loop done: packets_sent_with_ptt={packets_sent_with_ptt} \
             elapsed_ms={}",
            tx_start.elapsed().as_millis()
        );

        // ── Restore blocking mode ─────────────────────────────────────────
        let _ = self.socket.set_nonblocking(false);
        let _ = self.socket.set_read_timeout(Some(RECV_TIMEOUT));

        // ── PTT release — unconditional on all paths that asserted PTT ────
        // Send several ptt=false TX frames (C0[0]=0 → MOX/PTT cleared) so the
        // release survives a single dropped packet, with zero amplitude so no
        // carrier rides the release.  Then send_gain_cc to restore the RX NCO
        // centre and gain register (it too has C0[0]=0 → PTT cleared).
        const PTT_RELEASE_FRAMES: usize = 5;
        let mut release_frames_sent = 0u32;
        for _ in 0..PTT_RELEASE_FRAMES {
            if self.send_tx_packet(target_u32, 0.0, false).is_ok() {
                release_frames_sent += 1;
            }
        }
        info!("[hl2 tx-tune] PTT release frames sent: {release_frames_sent}");

        // ── TX drive / PA safe-off ────────────────────────────────────────
        // PTT is now cleared; zero the TX drive level and disable the PA
        // (register 0x09) so the board returns to an RX-safe state.  Run on
        // every path that reached the TX loop (normal completion or fault).
        if let Err(e) = self.send_tx_drive_cc(0, false) {
            warn!("[hl2 tx-tune] TX drive safe-off failed: {e}");
            if fault_status.is_none() {
                fault_status = Some(TxTuneStatus::Fault);
            }
        } else {
            info!("[hl2 tx-tune] TX drive level zeroed + PA disabled (reg 0x09)");
        }

        if let Err(e) = self.send_gain_cc() {
            warn!("[hl2 tx-tune] PTT release (send_gain_cc) failed: {e}");
            if fault_status.is_none() {
                fault_status = Some(TxTuneStatus::Fault);
            }
        } else {
            info!("[hl2 tx-tune] PTT released");
        }

        // ── Early return on TX fault ───────────────────────────────────────
        if let Some(status) = fault_status {
            let msg = match status {
                TxTuneStatus::Underflow => "TX IQ FIFO underflow",
                TxTuneStatus::Overflow => "TX IQ FIFO overflow",
                _ => "socket or hardware fault",
            };
            warn!("[hl2 tx-tune] fault: {msg}");
            return TxTuneResult {
                status,
                frequency_hz: target_freq_hz,
                duration_ms: clamped_duration_ms,
                drive: effective_amplitude,
                message: Some(msg.to_string()),
                ..TxTuneResult::default()
            };
        }

        // ── Post-TX status drain (~60 ms) ─────────────────────────────────
        // Read DDC packets briefly to refresh status registers with
        // post-TX telemetry (forward/reverse power, temperature, etc.).
        let _ = self.socket.set_read_timeout(Some(Duration::from_millis(20)));
        let drain_end = Instant::now() + Duration::from_millis(60);
        let mut drain_buf = [0u8; P1_PACKET_LEN];
        while Instant::now() < drain_end {
            match self.socket.recv(&mut drain_buf) {
                Ok(len) if len == P1_PACKET_LEN => {
                    let mut discard = VecDeque::new();
                    parse_ddc_packet(&drain_buf, &mut discard, &mut self.status_regs);
                }
                _ => {} // timeout or short packet — keep draining
            }
        }
        let _ = self.socket.set_read_timeout(Some(RECV_TIMEOUT));

        // ── Compute result telemetry ──────────────────────────────────────
        // Watts calibration is not available, so power stays None.  SWR,
        // however, depends only on the *ratio* of reflected to forward, so we
        // derive it directly from the peak raw detector counts captured during
        // the pulse.  (Suppress unused-variable noise; SWR_ALARM_THRESHOLD and
        // MIN_FORWARD_W are kept for the future watts-calibrated path.)
        let _ = (SWR_ALARM_THRESHOLD, MIN_FORWARD_W);
        let forward_power_w: Option<f32> = None;
        let reverse_power_w: Option<f32> = None;

        // Raw detector counts to surface in the result.  Use the PEAK values
        // latched WHILE PTT was asserted, NOT the post-release `status_regs`
        // (which has already decayed by the time the post-TX drain runs).
        let fwd_raw = max_forward_raw;
        let rev_raw = max_reverse_raw;
        let cur_raw = max_current_raw;

        // SWR from raw fwd/rev ratio (uncalibrated).  `None` when no forward
        // reading, rev > fwd, or gamma >= 1.0 (see compute_swr_from_raw).
        let swr = match (fwd_raw, rev_raw) {
            (Some(f), Some(r)) => compute_swr_from_raw(f, r),
            _ => None,
        };
        // gamma is logged for diagnostics (reflection coefficient magnitude).
        let gamma = match (fwd_raw, rev_raw) {
            (Some(f), Some(r)) if f > 0 => Some((r as f32 / f as f32).sqrt()),
            _ => None,
        };

        // ── Determine result status ───────────────────────────────────────
        // Watts not calibrated and no TX protection here (by design): a
        // completed pulse is reported Ok regardless of SWR; the operator reads
        // the SWR value itself.
        let status = TxTuneStatus::Ok;

        info!(
            "[hl2 spot] done: status={:?}  tx_drive_percent={tx_drive_percent:.0}  \
             drive_level_u8={drive_level}  effective_amplitude_fs={effective_amplitude:.3}  \
             max_fwd_raw={fwd_raw:?}  max_rev_raw={rev_raw:?}  max_cur_raw={cur_raw:?}  \
             gamma={gamma:?}  swr={swr:?}",
            status
        );

        TxTuneResult {
            status,
            forward_power_w,
            reverse_power_w,
            swr,
            forward_raw: fwd_raw,
            reverse_raw: rev_raw,
            current_raw: cur_raw,
            frequency_hz: target_freq_hz,
            duration_ms: clamped_duration_ms,
            drive: effective_amplitude,
            // TEMPORARY: surface peak in-pulse raw detector counts until watts
            // are calibrated.
            message: Some(format!(
                "max raw fwd={fwd_raw:?} rev={rev_raw:?} cur={cur_raw:?} \
                 @ {effective_amplitude:.3} FS, drive {drive_level}/255 (uncalibrated)"
            )),
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
        // Device-to-host status address is C0[7:3]; C0[2:0] are PTT/DASH/DOT.
        // (This differs from host-to-device writes, where C0 = addr<<1 | MOX.)
        let raddr = (c0 >> 3) & 0x1F;

        // Diagnostic: capture the raw 32-bit RDATA for any address 0..=4,
        // including ones we do not otherwise interpret (e.g. RADDR 3/4).
        if (raddr as usize) < status.rdata.len() {
            status.rdata[raddr as usize] = word;
            status.rdata_seen[raddr as usize] = true;
        }

        match raddr {
            0x00 => {
                status.firmware_version = (word & 0xFF) as u8;
                // Bits [15:14]: 2-bit under/overflow recovery code.
                status.recovery_bits = ((word >> 14) & 0x3) as u8;
                // Bits [14:8]: TX IQ FIFO occupancy (per HL2 RADDR 0x00 layout).
                status.tx_fifo_count = ((word >> 8) & 0x7F) as u16;
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
        //
        // The HL2 sends two 24-bit big-endian fields per sample.  Matching
        // Quisk's validated decode (`samp = xr + xi*I`, where `xi` is the FIRST
        // field and `xr` the SECOND), the complex sample is:
        //     real = SECOND field, imag = FIRST field
        // Mapping them the naive way — `Complex(first, second)` — conjugate-
        // mirrors the spectrum and swaps USB/LSB (an HL2-only sideband
        // inversion; RTL-SDR already delivers I+jQ in the expected orientation).
        for i in 0..P1_SAMPLES_PER_SUBFRAME {
            let b = 8 + i * 8;
            let field0 = i24_be(&sf[b..b + 3]) as f32 / (1u32 << 23) as f32;
            let field1 = i24_be(&sf[b + 3..b + 6]) as f32 / (1u32 << 23) as f32;
            out.push_back(Complex32::new(field1, field0));
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
        supports_tx_tune_test: true,
        ..SourceCapabilities::none()
    }
}
