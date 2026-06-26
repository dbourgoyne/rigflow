//! Minimal **TCI** (Transceiver Control Interface) server — macOS-FT8 spike.
//!
//! TCI is Expert Electronics' WebSocket protocol that carries CAT + PTT **and**
//! RX/TX audio over one localhost connection, so a TCI-capable digital app
//! (JTDX, WSJT-X-Improved, MSHV) can do FT8 with **no virtual audio driver**
//! (no BlackHole) and **no macOS mic permission**.  Like the Hamlib `rigctld`
//! server (`rigctl_server.rs`), this runs in the *client*, reads freq/mode from
//! `UiState`, and issues control via the existing `ControlCommand` channel.
//!
//! ```text
//!   JTDX ──ws://127.0.0.1:40001──▶ TciServer (here)
//!     RX audio  ◀── binary Data_Stream frames (tap of received audio)
//!     TX audio  ──▶ binary Data_Stream frames ─▶ MicShared::push_tx ─▶ UDP mic ─▶ server
//!     freq/mode/PTT  ◀─▶ text commands ─▶ ControlCommand / UiState
//! ```
//!
//! Scope boundary: this is **only** the localhost client↔app hop.  The Rigflow
//! client↔server RF path is unchanged (UDP media + WebSocket control); TX audio
//! still reaches the server via the existing `STREAM_TYPE_MIC_AUDIO` UDP stream.
//!
//! **Spike status:** single client, single receiver (`rx0`), the decode/transmit
//! path only.  The exact handshake parameter set, the audio `channels` (mono vs
//! interleaved stereo), and the IF/offset mapping are pinned empirically against
//! JTDX; items flagged `SPIKE:` below are the first things to verify/adjust.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;

use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_protocol::radio_control::ClientRadioMessage;

use crate::mic::MicShared;
use crate::net::control::ControlCommand;
use crate::ui::freq_limits::{active_freq_limits, clamp_center};
use crate::ui::state::UiState;

/// Default TCI WebSocket port (JTDX/ExpertSDR default).
pub const DEFAULT_TCI_PORT: u16 = 40001;

/// RX audio rate the Rigflow server delivers (matches the speaker path).
const RX_SERVER_RATE_HZ: u32 = 48_000;
/// Mic-TX rate the Rigflow server expects on the UDP mic stream.
const TX_SERVER_RATE_HZ: u32 = 48_000;
/// Default TCI audio rate until the client sets one via `audio_samplerate`.
const DEFAULT_AUDIO_RATE_HZ: u32 = 48_000;
/// Audio channels we emit.  TCI audio (SunSDR, what WSJT-X is written against)
/// is 2-channel interleaved; we duplicate mono into L/R.  SPIKE: drop to 1 if a
/// client turns out to want mono.
const TCI_AUDIO_CHANNELS: u32 = 2;

/// `~0.5 s` at 48 kHz — bounds the RX tap ring; drop oldest on overrun.
const RX_RING_MAX: usize = 24_000;

/// TCI binary `Data_Stream` constants (header is 8×u32 then 32 bytes pad = 64).
const TCI_HEADER_LEN: usize = 64;
const TCI_SAMPLE_FLOAT32: u32 = 3; // TciSampleType::FLOAT32
const TCI_STREAM_RX_AUDIO: u32 = 1; // TciStreamType::RX_AUDIO_STREAM
const TCI_STREAM_TX_AUDIO: u32 = 2; // TciStreamType::TX_AUDIO_STREAM
const TCI_STREAM_TX_CHRONO: u32 = 3; // TciStreamType::TxChrono (TX clock)

/// Safety ceiling on mono samples requested per `TxChrono`.  WSJT-X allocates
/// `length*2` floats in `data[8192]` and returns `length/2` mono samples, so the
/// chrono's float `length` = mono*2 must stay ≤ 8192 → mono ≤ 2048.  The pump
/// otherwise paces to wall-clock time.
const TX_CHRONO_MAX_MONO: u32 = 2048;

/// RX-audio tap fed by the media thread (`net::udp`) and drained by the TCI
/// connection task.  No-op unless a client has requested `audio_start`.  Lives
/// in `UiState` so both the media thread and the TCI task share one instance —
/// the same pattern as `DigitalRxOutput`.
#[derive(Debug)]
pub struct TciRxAudio {
    enabled: AtomicBool,
    ring: Mutex<VecDeque<f32>>,
}

impl TciRxAudio {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            ring: Mutex::new(VecDeque::new()),
        })
    }

    /// Enable/disable buffering (set by `audio_start`/`audio_stop`).  Clears the
    /// ring on disable so a new session doesn't replay stale audio.
    fn set_enabled(&self, on: bool) {
        if self.enabled.swap(on, Ordering::Relaxed) != on && !on {
            if let Ok(mut r) = self.ring.lock() {
                r.clear();
            }
        }
    }

    /// Push a copy of received 48 kHz mono audio (media thread).  No-op unless a
    /// TCI client is streaming.  Drop oldest past the cap so RX never stalls.
    pub fn push(&self, samples: &[f32]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.ring.lock() {
            r.extend(samples.iter().copied());
            if r.len() > RX_RING_MAX {
                let drop = r.len() - RX_RING_MAX;
                r.drain(..drop);
            }
        }
    }

    fn drain(&self) -> Vec<f32> {
        match self.ring.lock() {
            Ok(mut r) => r.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Shared handles each connection uses to read state and issue commands.
struct TciShared {
    ui_state: Arc<Mutex<UiState>>,
    cmd_tx: UnboundedSender<ControlCommand>,
    /// TX-audio sink (the same ring the mic/digital TX path feeds).
    mic_shared: Arc<MicShared>,
    /// RX-audio tap (shared with the media thread via `UiState`).
    rx_audio: Arc<TciRxAudio>,
    /// Negotiated TCI audio sample rate (`audio_samplerate`), default 48 kHz.
    audio_rate_hz: AtomicU32,
    /// True while keyed (PTT on).  The connection loop sends `TxChrono` frames
    /// while this is set — WSJT-X only streams TX audio in reply to chronos.
    tx_active: AtomicBool,
}

/// The TCI WebSocket server.
pub struct TciServer {
    port: u16,
    shared: Arc<TciShared>,
}

impl TciServer {
    pub fn new(ui_state: Arc<Mutex<UiState>>, cmd_tx: UnboundedSender<ControlCommand>) -> Self {
        let (mic_shared, rx_audio) = match ui_state.lock() {
            Ok(s) => (Arc::clone(&s.mic_shared), Arc::clone(&s.tci_rx_audio)),
            Err(_) => (Arc::new(MicShared::default()), TciRxAudio::new()),
        };
        Self {
            port: DEFAULT_TCI_PORT,
            shared: Arc::new(TciShared {
                ui_state,
                cmd_tx,
                mic_shared,
                rx_audio,
                audio_rate_hz: AtomicU32::new(DEFAULT_AUDIO_RATE_HZ),
                tx_active: AtomicBool::new(false),
            }),
        }
    }

    /// Listen on `127.0.0.1:<port>` until the process exits.  Never panics.
    pub async fn run(self) {
        let listener = match TcpListener::bind(("127.0.0.1", self.port)).await {
            Ok(l) => l,
            Err(e) => {
                log::error!("[tci] failed to bind 127.0.0.1:{}: {e}", self.port);
                // Surface the bind failure to the UI (mirrors rigctl_status), so
                // the WSJT-X/FT8 Setup window can show it instead of silent dead air.
                if let Ok(mut s) = self.shared.ui_state.lock() {
                    s.tci_status = Some(format!(
                        "TCI server cannot bind 127.0.0.1:{} — {e}",
                        self.port
                    ));
                }
                return;
            }
        };
        if let Ok(mut s) = self.shared.ui_state.lock() {
            s.tci_status = None; // bound OK
        }
        log::info!("[tci] TCI server listening on ws://127.0.0.1:{}", self.port);

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let shared = Arc::clone(&self.shared);
                    tokio::spawn(async move {
                        log::info!("[tci] client connected: {peer}");
                        if let Err(e) = handle_connection(stream, &shared).await {
                            log::debug!("[tci] connection error ({peer}): {e}");
                        }
                        // Leave the rig safe if the app vanished mid-transmit.
                        end_session(&shared);
                        log::info!("[tci] client disconnected: {peer}");
                    });
                }
                Err(e) => log::warn!("[tci] accept failed: {e}"),
            }
        }
    }
}

/// Per-connection loop: WebSocket upgrade, send the handshake, then interleave
/// inbound commands/TX-audio with periodic RX-audio frames.
async fn handle_connection(
    stream: TcpStream,
    shared: &Arc<TciShared>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (mut sink, mut source) = ws.split();

    // Initialization handshake the client waits for before going ready.
    for line in handshake_lines(shared) {
        sink.send(Message::text(line)).await?;
    }

    // RX audio is flushed on a fixed cadence; ~20 ms keeps latency low and frames
    // a sensible size (~960 samples at 48 kHz).  The same tick clocks TxChrono.
    let mut flush = tokio::time::interval(Duration::from_millis(20));
    let mut rx_frames_sent = 0u64;

    // TxChrono pacing: while keyed, request TX samples at exactly real-time so the
    // FT8 waveform plays at the right speed.  `tx_start`/`tx_samples_requested`
    // lock the cumulative request count to wall-clock, self-correcting timer
    // jitter (sending too fast time-compresses the on-air signal → no decode).
    let mut tx_was_active = false;
    let mut tx_start = tokio::time::Instant::now();
    let mut tx_samples_requested: u64 = 0;
    let mut tx_chronos_sent = 0u64;

    loop {
        tokio::select! {
            msg = source.next() => {
                let Some(msg) = msg else { break };
                match msg? {
                    Message::Text(text) => {
                        // A frame may carry several `;`-terminated commands.
                        for cmd in text.as_str().split(';') {
                            let cmd = cmd.trim();
                            if cmd.is_empty() {
                                continue;
                            }
                            log::debug!("[tci] cmd: {cmd}");
                            if let Some(reply) = handle_text_command(cmd, shared) {
                                sink.send(Message::text(reply)).await?;
                            }
                        }
                    }
                    Message::Binary(buf) => handle_tx_audio(&buf, shared),
                    Message::Ping(p) => sink.send(Message::Pong(p)).await?,
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = flush.tick() => {
                if shared.rx_audio.enabled.load(Ordering::Relaxed) {
                    if let Some(frame) = next_rx_frame(shared) {
                        let samples = (frame.len() - TCI_HEADER_LEN) / 4;
                        sink.send(Message::binary(frame)).await?;
                        rx_frames_sent += 1;
                        if rx_frames_sent == 1 || rx_frames_sent % 50 == 0 {
                            log::debug!(
                                "[tci] sent {rx_frames_sent} RX audio frames ({samples} floats last)"
                            );
                        }
                    }
                }

                // TxChrono pump: while keyed, request TX audio at real-time rate.
                // WSJT-X streams TX audio ONLY in reply to these frames.
                let tx_now = shared.tx_active.load(Ordering::Relaxed);
                if tx_now {
                    let rate = shared.audio_rate_hz.load(Ordering::Relaxed).max(1);
                    if !tx_was_active {
                        tx_start = tokio::time::Instant::now();
                        tx_samples_requested = 0;
                        tx_chronos_sent = 0;
                        log::info!("[tci] TX keyed — starting TxChrono pump @ {rate} Hz");
                    }
                    // Mono samples that should have been delivered by now.
                    let target = (tx_start.elapsed().as_secs_f64() * rate as f64) as u64;
                    if target > tx_samples_requested {
                        // WSJT-X returns length/2 mono samples per chrono (it fills
                        // `length` interleaved-stereo floats), so request 2× the
                        // float length for the mono samples we still owe — else the
                        // modulator is fed at half real-time (bursty underruns).
                        let need = (target - tx_samples_requested).min(TX_CHRONO_MAX_MONO as u64);
                        sink.send(Message::binary(encode_tx_chrono(rate, (need * 2) as u32))).await?;
                        tx_samples_requested += need;
                        tx_chronos_sent += 1;
                        if tx_chronos_sent == 1 || tx_chronos_sent % 50 == 0 {
                            log::debug!(
                                "[tci] sent {tx_chronos_sent} TxChrono frames ({need} mono samples last)"
                            );
                        }
                    }
                } else if tx_was_active {
                    log::info!("[tci] TX unkeyed — TxChrono pump stopped ({tx_chronos_sent} sent)");
                }
                tx_was_active = tx_now;
            }
        }
    }
    Ok(())
}

/// The parameter burst sent on connect.  SPIKE: this is a reasonable superset of
/// what TCI clients read at startup; trim/extend against JTDX's actual reads.
fn handshake_lines(shared: &TciShared) -> Vec<String> {
    let freq = current_freq_hz(shared);
    let mode = demod_to_tci_mode(current_demod(shared));
    vec![
        "protocol:Rigflow,1.8;".to_string(),
        "device:Rigflow;".to_string(),
        "receive_only:false;".to_string(),
        "trx_count:1;".to_string(),
        "channels_count:1;".to_string(),
        "vfo_limits:0,500000000;".to_string(),
        "if_limits:-24000,24000;".to_string(),
        "modulations_list:am,sam,dsb,lsb,usb,cw,nfm,digl,digu,wfm;".to_string(),
        // Audio rates the real ExpertSDR/Thetis init burst announces.  WSJT-X
        // configures its audio engine — including the TX-audio encoder — from
        // the server-announced `audio_samplerate`.  Omitting it lets RX stream
        // (each frame carries its own rate) but leaves WSJT-X with no TX rate,
        // so it keys PTT but never streams transmit audio (zero-power symptom).
        format!("iq_samplerate:{RX_SERVER_RATE_HZ};"),
        format!("audio_samplerate:{DEFAULT_AUDIO_RATE_HZ};"),
        format!("dds:0,{freq};"),
        format!("vfo:0,0,{freq};"),
        "if:0,0,0;".to_string(),
        format!("modulation:0,{mode};"),
        "rx_enable:0,true;".to_string(),
        "trx:0,false;".to_string(),
        "start;".to_string(),
        "ready;".to_string(),
    ]
}

/// Handle one text command (`name` or `name:arg,arg`).  Returns an optional
/// reply line.  Sets carry the value param; bare-`rx` reads are answered.
fn handle_text_command(cmd: &str, shared: &TciShared) -> Option<String> {
    let (name, args) = match cmd.split_once(':') {
        Some((n, a)) => (n.trim(), a.trim()),
        None => (cmd.trim(), ""),
    };
    let params: Vec<&str> = if args.is_empty() {
        Vec::new()
    } else {
        args.split(',').map(|s| s.trim()).collect()
    };

    match name {
        // dds:rx,freq / vfo:rx,chan,freq — set (value present) or read.  ALWAYS
        // echo the resulting state: TCI clients (WSJT-X) block waiting for this
        // confirmation after setting the frequency, and time out without it.
        "dds" | "vfo" => {
            // vfo carries an extra channel index: vfo:rx,chan,freq.
            let val_idx = if name == "vfo" { 2 } else { 1 };
            if let Some(hz) = params.get(val_idx).and_then(|s| s.parse::<f64>().ok()) {
                set_frequency(shared, hz.max(0.0) as u64);
            }
            let f = current_freq_hz(shared);
            Some(if name == "vfo" {
                format!("vfo:0,0,{f};")
            } else {
                format!("dds:0,{f};")
            })
        }

        // modulation:rx,mode — set or read; always echo the current mode.
        "modulation" => {
            if let Some(m) = params.get(1) {
                set_mode_by_name(shared, m);
            }
            Some(format!(
                "modulation:0,{};",
                demod_to_tci_mode(current_demod(shared))
            ))
        }

        // trx:rx,state (PTT set) | trx:rx (read).
        "trx" => match params.get(1).copied() {
            Some(state) => {
                let on = state.eq_ignore_ascii_case("true");
                set_ptt(shared, on);
                Some(format!("trx:0,{};", if on { "true" } else { "false" }))
            }
            None => {
                let on = shared.ui_state.lock().map(|s| s.cat_ptt).unwrap_or(false);
                Some(format!("trx:0,{};", if on { "true" } else { "false" }))
            }
        },

        // WSJT-X blocks its startup waiting for the server to ECHO audio_start
        // (Cmd_AudioStart sets stream_audio_ only on the echo); without it the
        // rig init times out → "Rig Control Error".  Same for stop/samplerate.
        "audio_start" => {
            shared.rx_audio.set_enabled(true);
            log::info!("[tci] RX audio streaming started");
            Some(format!("{cmd};"))
        }
        "audio_stop" => {
            shared.rx_audio.set_enabled(false);
            log::info!("[tci] RX audio streaming stopped");
            Some(format!("{cmd};"))
        }
        "audio_samplerate" => {
            if let Some(rate) = params.first().and_then(|s| s.parse::<u32>().ok()) {
                shared.audio_rate_hz.store(rate.max(1), Ordering::Relaxed);
                log::info!("[tci] audio sample rate set to {rate} Hz");
            }
            Some(format!("{cmd};"))
        }

        // Reflect these back as the accept-confirmation the client expects (the
        // rig has no real IF offset / split, so they're otherwise no-ops).
        "if" | "rx_enable" | "split_enable" | "start" | "stop" => Some(format!("{cmd};")),

        other => {
            log::debug!("[tci] unhandled command: {other}");
            None
        }
    }
}

/// Diagnostic counters for inbound binary frames: total seen, and those passed
/// to the modulator.  Used to pinpoint "keys but zero power" (no TX audio).
static TX_FRAMES_SEEN: AtomicU64 = AtomicU64::new(0);
static TX_FRAMES_FED: AtomicU64 = AtomicU64::new(0);

/// Decode an inbound TX-audio `Data_Stream` and feed it to the mic-TX ring.
fn handle_tx_audio(buf: &[u8], shared: &TciShared) {
    if buf.len() < TCI_HEADER_LEN {
        log::debug!("[tci] TX frame too short: {} bytes", buf.len());
        return;
    }
    let u32_at = |i: usize| u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
    let sample_rate = u32_at(4);
    // field 5 = number of VALID floats (interleaved stereo); the buffer on the
    // wire is length*2 floats but only the first `length` are real audio — the
    // tail is uninitialised heap (NaN/Inf garbage), so never read past `length`.
    let length = u32_at(20) as usize;
    let data_type = u32_at(24); // field 6 (TciStreamType); field 7+ = reserved

    let seen = TX_FRAMES_SEEN.fetch_add(1, Ordering::Relaxed) + 1;
    if seen == 1 || seen % 50 == 0 {
        log::debug!(
            "[tci] TX frame #{seen}: {} bytes, data_type={data_type}, rate={sample_rate}, \
             length={length} (expect data_type={TCI_STREAM_TX_AUDIO})",
            buf.len()
        );
    }

    if data_type != TCI_STREAM_TX_AUDIO {
        // Prime suspect for "keys but zero power": WSJT-X stamps TX audio with a
        // stream-type value we drop.  Surface it loudly (rate-limited).
        if seen == 1 || seen % 50 == 0 {
            log::warn!(
                "[tci] dropping TX frame: data_type={data_type} != {TCI_STREAM_TX_AUDIO} \
                 (no audio reaches the modulator → zero RF)"
            );
        }
        return;
    }

    let mono = tx_stereo_to_mono(buf, length);
    let out = resample(&mono, sample_rate as f32, TX_SERVER_RATE_HZ as f32);

    // Log signal level so we can tell "no frames" from "silent frames" from
    // "frames not streaming" (push_tx is a no-op unless tx_streaming is set).
    let fed = TX_FRAMES_FED.fetch_add(1, Ordering::Relaxed) + 1;
    if fed == 1 || fed % 50 == 0 {
        let peak = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        let rms = if out.is_empty() {
            0.0
        } else {
            (out.iter().map(|&s| s * s).sum::<f32>() / out.len() as f32).sqrt()
        };
        log::debug!(
            "[tci] TX audio fed #{fed}: {} samples, peak={peak:.3}, rms={rms:.4}, \
             streaming={}",
            out.len(),
            shared.mic_shared.tx_streaming()
        );
    }

    shared.mic_shared.push_tx(&out);
}

/// Decode a `TxAudioStream` frame body to mono.  WSJT-X fills only the first
/// `length` floats (interleaved stereo) of a `length*2` buffer — the tail is
/// uninitialised heap — so read exactly `length`, sanitise non-finite samples
/// (NaN/Inf garbage must never reach the modulator / HL2 DAC), and downmix by 2.
fn tx_stereo_to_mono(buf: &[u8], length: usize) -> Vec<f32> {
    let avail = buf.len().saturating_sub(TCI_HEADER_LEN) / 4;
    let valid = length.min(avail);
    let mut samples = Vec::with_capacity(valid);
    for i in 0..valid {
        let o = TCI_HEADER_LEN + i * 4;
        let s = f32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
        samples.push(if s.is_finite() { s } else { 0.0 });
    }
    downmix(&samples, 2)
}

/// Build the next outbound RX-audio frame (drains the tap, resamples to the
/// negotiated rate, encodes a `Data_Stream`).  `None` when nothing is buffered.
fn next_rx_frame(shared: &TciShared) -> Option<Vec<u8>> {
    let mono48 = shared.rx_audio.drain();
    if mono48.is_empty() {
        return None;
    }
    let rate = shared.audio_rate_hz.load(Ordering::Relaxed).max(1);
    let out = resample(&mono48, RX_SERVER_RATE_HZ as f32, rate as f32);
    if out.is_empty() {
        return None;
    }
    let buf = interleave(&out, TCI_AUDIO_CHANNELS);
    Some(encode_data_stream(
        rate,
        TCI_STREAM_RX_AUDIO,
        TCI_AUDIO_CHANNELS,
        &buf,
    ))
}

/// Duplicate mono into `channels` interleaved channels (no-op for mono).
fn interleave(mono: &[f32], channels: u32) -> Vec<f32> {
    if channels <= 1 {
        return mono.to_vec();
    }
    let ch = channels as usize;
    let mut out = Vec::with_capacity(mono.len() * ch);
    for &s in mono {
        for _ in 0..ch {
            out.push(s);
        }
    }
    out
}

/// Encode a TCI `Data_Stream` packet: 8×u32 header (`<8I`) + 32 bytes pad +
/// little-endian float32 samples.  `length` is the sample count.  SPIKE: mono.
fn encode_data_stream(
    sample_rate: u32,
    stream_type: u32,
    channels: u32,
    samples: &[f32],
) -> Vec<u8> {
    let mut v = Vec::with_capacity(TCI_HEADER_LEN + samples.len() * 4);
    let header: [u32; 8] = [
        0,                    // receiver
        sample_rate,          // sample_rate
        TCI_SAMPLE_FLOAT32,   // data_format
        0,                    // codec
        0,                    // crc (unused)
        samples.len() as u32, // length (total float count)
        stream_type,          // data_type
        channels,             // channels
    ];
    for field in header {
        v.extend_from_slice(&field.to_le_bytes());
    }
    v.extend_from_slice(&[0u8; 32]); // pad header to 64 bytes
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Encode a `TxChrono` frame: a 64-byte header (no audio payload) that requests
/// `length` per-channel samples of TX audio.  WSJT-X replies with one
/// `TxAudioStream` frame of `length * 2` interleaved-stereo floats per chrono;
/// `length` (and `sample_rate`/`format`) are echoed back into that reply.
fn encode_tx_chrono(sample_rate: u32, length: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(TCI_HEADER_LEN);
    let header: [u32; 8] = [
        0,                    // receiver
        sample_rate,          // sample_rate
        TCI_SAMPLE_FLOAT32,   // data_format
        0,                    // codec
        0,                    // crc
        length,               // length (per-channel samples requested)
        TCI_STREAM_TX_CHRONO, // data_type = TxChrono
        0,                    // reserved
    ];
    for field in header {
        v.extend_from_slice(&field.to_le_bytes());
    }
    v.extend_from_slice(&[0u8; 32]); // pad header to 64 bytes (no data field)
    v
}

// ── State access (mirrors rigctl_server.rs) ──────────────────────────────────

fn current_freq_hz(shared: &TciShared) -> u64 {
    shared
        .ui_state
        .lock()
        .map(|s| s.target_freq_hz.max(0.0) as u64)
        .unwrap_or(0)
}

fn current_demod(shared: &TciShared) -> DemodMode {
    shared
        .ui_state
        .lock()
        .map(|s| s.demod_mode)
        .unwrap_or(DemodMode::Usb)
}

/// Tune to a TCI frequency — same in-band-vs-band-change logic as the CAT path
/// (`rigctl_server::set_frequency`): move the target within the visible band, or
/// recenter the LO on a band change so far jumps actually take.
fn set_frequency(shared: &TciShared, hz: u64) {
    let (center, sample_rate, limits) = match shared.ui_state.lock() {
        Ok(s) => (
            s.center_freq_hz,
            s.input_sample_rate_hz,
            active_freq_limits(&s),
        ),
        Err(_) => return,
    };
    let freq = clamp_center(hz as f32, &limits);
    let half_bw = (sample_rate / 2.0).max(0.0);
    let in_band = half_bw > 0.0 && (freq - center).abs() <= half_bw;

    if in_band {
        if let Ok(mut s) = shared.ui_state.lock() {
            s.target_freq_hz = freq;
        }
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetTargetFrequency {
                target_freq_hz: freq as u64,
            },
        ));
    } else {
        if let Ok(mut s) = shared.ui_state.lock() {
            s.center_freq_hz = freq;
            s.target_freq_hz = freq;
        }
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetCenterFrequency {
                center_freq_hz: freq as u64,
            },
        ));
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetTargetFrequency {
                target_freq_hz: freq as u64,
            },
        ));
    }
}

/// Set the demod from a TCI modulation name (maps `digu`→data-USB, etc.).
fn set_mode_by_name(shared: &TciShared, name: &str) {
    let Some(mode) = tci_mode_to_demod(name) else {
        log::warn!("[tci] unsupported modulation: {name}");
        return;
    };
    if let Ok(mut s) = shared.ui_state.lock() {
        s.demod_mode = mode;
    }
    let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
        ClientRadioMessage::SetDemodMode { mode },
    ));
    if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
        let sideband = if mode == DemodMode::Usb {
            Sideband::Usb
        } else {
            Sideband::Lsb
        };
        let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(
            ClientRadioMessage::SetSideband { sideband },
        ));
    }
}

/// Key/unkey TX.  The TCI server is itself the TX-audio producer, so it claims
/// the mic-TX ring as the external source (suppressing the always-on mic
/// capture) and enables streaming, then keys the server — exactly the contract
/// `rigctl_server::set_ptt` + `DigitalTxInput::set_active` use on Linux.
fn set_ptt(shared: &TciShared, on: bool) {
    if let Ok(mut s) = shared.ui_state.lock() {
        s.cat_ptt = on;
    }
    // Gate the TxChrono pump in the connection loop (WSJT-X only sends TX audio
    // in reply to chronos, so we must clock it for the duration of the over).
    shared.tx_active.store(on, Ordering::Relaxed);
    if on {
        shared.mic_shared.set_external_tx_source(true);
        shared.mic_shared.set_tx_streaming(true);
    } else {
        shared.mic_shared.set_tx_streaming(false);
        shared.mic_shared.set_external_tx_source(false);
    }
    let msg = if on {
        ClientRadioMessage::StartMicTx
    } else {
        ClientRadioMessage::StopMicTx
    };
    let _ = shared.cmd_tx.send(ControlCommand::RadioMessage(msg));
}

/// On disconnect: stop RX streaming and unkey if we left TX on.
fn end_session(shared: &TciShared) {
    shared.rx_audio.set_enabled(false);
    if shared.ui_state.lock().map(|s| s.cat_ptt).unwrap_or(false) {
        set_ptt(shared, false);
    }
}

// ── Helpers: mode mapping, resample, downmix ─────────────────────────────────

fn tci_mode_to_demod(m: &str) -> Option<DemodMode> {
    match m.to_ascii_lowercase().as_str() {
        "usb" => Some(DemodMode::Usb),
        "digu" | "pktusb" => Some(DemodMode::DgtU), // data-USB (FT8)
        "lsb" | "digl" | "pktlsb" => Some(DemodMode::Lsb),
        "cw" | "cwu" => Some(DemodMode::Cwu),
        "cwr" | "cwl" => Some(DemodMode::Cwl),
        "am" | "sam" | "dsb" => Some(DemodMode::Am),
        "nfm" | "fm" => Some(DemodMode::Nfm),
        "wfm" => Some(DemodMode::Wfm),
        _ => None,
    }
}

fn demod_to_tci_mode(mode: DemodMode) -> &'static str {
    match mode {
        DemodMode::Usb => "usb",
        DemodMode::DgtU => "digu",
        DemodMode::Lsb => "lsb",
        DemodMode::Cwu => "cw",
        DemodMode::Cwl => "cwr",
        DemodMode::Am => "am",
        DemodMode::Nfm => "nfm",
        DemodMode::Wfm => "wfm",
    }
}

/// Downmix interleaved frames to mono (no-op for 1 channel).
pub(crate) fn downmix(samples: &[f32], channels: u32) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks(ch)
        .map(|f| f.iter().sum::<f32>() / f.len() as f32)
        .collect()
}

/// Stateless linear resampler.  SPIKE: resets phase per call, so block-boundary
/// error is one interpolation step (~negligible for FT8); carry phase if needed.
pub(crate) fn resample(input: &[f32], from_hz: f32, to_hz: f32) -> Vec<f32> {
    if input.is_empty() || (from_hz - to_hz).abs() < 1.0 {
        return input.to_vec();
    }
    let step = from_hz / to_hz; // input advance per output sample
    let mut out = Vec::with_capacity(((input.len() as f32) * (to_hz / from_hz)) as usize + 1);
    let mut pos = 0.0f32;
    while (pos as usize) + 1 < input.len() {
        let i = pos as usize;
        let frac = pos - i as f32;
        out.push(input[i] + (input[i + 1] - input[i]) * frac);
        pos += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(buf: &[u8], byte: usize) -> u32 {
        u32::from_le_bytes([buf[byte], buf[byte + 1], buf[byte + 2], buf[byte + 3]])
    }

    /// Build a TxAudioStream frame body: 64-byte header + the given floats.  The
    /// header `length` is set independently so tests can mimic WSJT-X sending a
    /// `length*2` buffer whose tail is uninitialised.
    fn tx_frame(length: u32, floats: &[f32]) -> Vec<u8> {
        let mut v = encode_data_stream(48_000, TCI_STREAM_TX_AUDIO, 2, floats);
        v[20..24].copy_from_slice(&length.to_le_bytes()); // override length field
        v
    }

    #[test]
    fn chrono_is_header_only_with_expected_fields() {
        let frame = encode_tx_chrono(48_000, 1920);
        assert_eq!(
            frame.len(),
            TCI_HEADER_LEN,
            "chrono carries no audio payload"
        );
        assert_eq!(u32_at(&frame, 4), 48_000); // sample_rate
        assert_eq!(u32_at(&frame, 8), TCI_SAMPLE_FLOAT32); // format
        assert_eq!(u32_at(&frame, 20), 1920); // length (float budget)
        assert_eq!(u32_at(&frame, 24), TCI_STREAM_TX_CHRONO); // type = 3
    }

    #[test]
    fn data_stream_header_round_trips() {
        let frame = encode_data_stream(48_000, TCI_STREAM_RX_AUDIO, 2, &[0.5, -0.5, 0.25, -0.25]);
        assert_eq!(frame.len(), TCI_HEADER_LEN + 4 * 4);
        assert_eq!(u32_at(&frame, 4), 48_000);
        assert_eq!(u32_at(&frame, 20), 4); // length = float count
        assert_eq!(u32_at(&frame, 24), TCI_STREAM_RX_AUDIO);
    }

    #[test]
    fn downmix_averages_interleaved_stereo() {
        assert_eq!(downmix(&[1.0, 3.0, 2.0, 4.0], 2), vec![2.0, 3.0]);
        assert_eq!(downmix(&[1.0, 2.0, 3.0], 1), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn tx_decode_reads_only_valid_length_and_downmixes() {
        // length=6 valid floats (3 stereo frames, L==R) → 3 mono samples.
        let frame = tx_frame(6, &[1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
        assert_eq!(tx_stereo_to_mono(&frame, 6), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn tx_decode_ignores_uninitialised_tail() {
        // WSJT-X allocates length*2 floats but fills only `length`; the tail is
        // garbage (incl. NaN/Inf) and must never be read.
        let floats = [1.0, 1.0, 2.0, 2.0, f32::NAN, f32::INFINITY, 9.9e30, -9.9e30];
        let frame = tx_frame(4, &floats); // only first 4 floats are valid
        assert_eq!(tx_stereo_to_mono(&frame, 4), vec![1.0, 2.0]);
    }

    #[test]
    fn tx_decode_sanitises_non_finite_within_valid_region() {
        // A non-finite sample inside the valid region is zeroed, not propagated.
        let frame = tx_frame(2, &[f32::NAN, 2.0]);
        assert_eq!(tx_stereo_to_mono(&frame, 2), vec![1.0]); // (0.0 + 2.0) / 2
    }

    #[test]
    fn chrono_length_yields_half_as_many_mono_samples() {
        // Sizing contract: a chrono of float length N produces N/2 mono samples,
        // so the pump must request 2× the mono samples it owes.
        let n = 1920u32;
        let valid: Vec<f32> = (0..n).map(|i| (i % 7) as f32 * 0.1).collect();
        let frame = tx_frame(n, &valid);
        assert_eq!(
            tx_stereo_to_mono(&frame, n as usize).len(),
            (n / 2) as usize
        );
    }

    #[test]
    fn mode_mapping_round_trips_ft8_data_usb() {
        assert_eq!(tci_mode_to_demod("digu"), Some(DemodMode::DgtU));
        assert_eq!(demod_to_tci_mode(DemodMode::DgtU), "digu");
    }
}
