//! `rigflow-probe` — headless diagnostic client.
//!
//! Performs the real control-plane handshake against a running `rigflow-server`,
//! receives the UDP audio stream, optionally records it to a WAV, and reports
//! transport/dropout statistics — with no GUI and no audio device.
//!
//! Run it on the server box over loopback (`--server 127.0.0.1:9000`) to remove
//! the network and check whether audio breaks are server-side. The server is
//! single-client, so disconnect the GUI client first.

mod args;
mod stats;

use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use socket2::{Domain, Socket, Type};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

use rigflow_core::audio::jitter_buffer::JitterBuffer;
use rigflow_core::dsp::modes::DemodMode;
use rigflow_core::net::udp_framing::{
    audio_samples_offset, audio_send_wall_ns, is_valid_header, parse_media_header, MAGIC,
    STREAM_TYPE_AUDIO, STREAM_TYPE_REGISTER_AUDIO, STREAM_TYPE_WATERFALL, VERSION,
};
use rigflow_core::radio::RadioId;
use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};

use args::{print_radio_list, resolve_radio, Args};
use stats::{print_report, CaptureStats};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

/// Audio packet size — must match the server (240 samples @ 48 kHz). The buffer's
/// target/max depths are configurable via `--jitter-target-ms` / `--jitter-max-ms`.
const PACKET_SAMPLES: usize = 240;

/// Live view of the tuned state, seeded from the snapshot and updated from the
/// server's `RuntimeChanged` messages so the report shows what we actually tuned to.
struct ReportState {
    center: u64,
    target: u64,
    mode: String,
    rate: u32,
    /// DSP gating state — important context: squelch/NR2 can zero the noise floor and
    /// make a capture sound silent. The probe disables squelch + NR2 for a raw capture.
    squelch: bool,
    nr2: bool,
    agc: bool,
    /// Latest S-meter reading — tells whether the band actually has RF, or is dead.
    signal_dbm: f32,
    s_units: i32,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    if let Err(e) = run(args).await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), String> {
    let (host, ws_port) = args.server_host_port();
    let udp_reg_port = args.udp_reg_port();

    // --- UDP media socket + the address we advertise to the server ----------
    // Enlarge SO_RCVBUF so a brief receive-loop stall (e.g. CPU contention when
    // co-located with the server on a Pi) doesn't overflow the kernel buffer and
    // drop packets — which would otherwise read as server-side loss. The kernel
    // caps the request at net.core.rmem_max, so we log what we actually got.
    const RCVBUF_BYTES: usize = 8 * 1024 * 1024;
    let media_sock = {
        let sock = Socket::new(Domain::IPV4, Type::DGRAM, None)
            .map_err(|e| format!("create media socket: {e}"))?;
        if let Err(e) = sock.set_recv_buffer_size(RCVBUF_BYTES) {
            warn!("could not enlarge SO_RCVBUF: {e}");
        }
        let bind_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        sock.bind(&bind_addr.into())
            .map_err(|e| format!("bind media socket: {e}"))?;
        sock.set_nonblocking(true)
            .map_err(|e| format!("set media socket nonblocking: {e}"))?;
        let got = sock.recv_buffer_size().unwrap_or(0);
        info!(
            "media socket SO_RCVBUF: requested {} KiB, got {} KiB",
            RCVBUF_BYTES / 1024,
            got / 1024
        );
        UdpSocket::from_std(sock.into()).map_err(|e| format!("media socket from_std: {e}"))?
    };
    let local_port = media_sock
        .local_addr()
        .map_err(|e| format!("media socket local_addr: {e}"))?
        .port();

    // The server streams audio to the host:port string we send in AcquireRadio,
    // not to the source of the registration datagram — so it must be reachable
    // from the server. Determine the routable local IP the way the client does.
    let local_ip = {
        let probe =
            StdUdpSocket::bind("0.0.0.0:0").map_err(|e| format!("route probe bind: {e}"))?;
        probe
            .connect((host.as_str(), ws_port))
            .map_err(|e| format!("route probe connect: {e}"))?;
        probe
            .local_addr()
            .map_err(|e| format!("route probe local_addr: {e}"))?
            .ip()
    };
    let audio_peer = format!("{local_ip}:{local_port}");
    info!("media socket bound on {audio_peer} (advertised to server)");

    // --- WebSocket control plane -------------------------------------------
    let ws_url = format!("ws://{host}:{ws_port}/ws");
    info!("connecting to {ws_url}");
    let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("websocket connect: {e}"))?;
    let (mut write, mut read) = ws.split();

    // List radios and resolve the selection.
    send_msg(&mut write, &ClientRadioMessage::ListRadios).await?;
    let radios = loop {
        match next_server_msg(&mut read, &mut write).await? {
            ServerRadioMessage::RadiosListed { radios } => break radios,
            ServerRadioMessage::RadioError { code, message } => {
                return Err(format!(
                    "server error during ListRadios [{code}]: {message}"
                ))
            }
            other => info!("ignoring pre-list message: {}", msg_kind(&other)),
        }
    };
    print_radio_list(&radios);
    let idx = resolve_radio(&radios, args.radio.as_deref())?;
    let radio = &radios[idx];
    info!(
        "acquiring radio [{idx}] {} ({})",
        radio.id.0, radio.display_name
    );

    // Require a frequency: AcquireRadio carries center/target, and the probe does
    // not inherit the GUI's tuning.
    if args.center_hz.is_none() && args.target_hz.is_none() {
        return Err(
            "provide --target-hz (and optionally --center-hz): the probe tunes itself, \
             it does not inherit the GUI's frequency"
                .to_string(),
        );
    }
    let (center_hz, target_hz) = args.initial_freqs(0);

    // Register the UDP media plane (optional for RX audio, but matches the client
    // and primes the return path). 4 bytes: MAGIC(be) | VERSION | REGISTER_AUDIO.
    let reg = [
        (MAGIC >> 8) as u8,
        (MAGIC & 0xff) as u8,
        VERSION,
        STREAM_TYPE_REGISTER_AUDIO,
    ];
    if let Err(e) = media_sock
        .send_to(&reg, (host.as_str(), udp_reg_port))
        .await
    {
        warn!("UDP registration send to {host}:{udp_reg_port} failed: {e}");
    }

    // Acquire the radio.
    send_msg(
        &mut write,
        &ClientRadioMessage::AcquireRadio {
            radio_id: RadioId(radio.id.0.clone()),
            center_freq_hz: center_hz,
            target_freq_hz: target_hz,
            audio_udp_peer: audio_peer.clone(),
            waterfall_udp_peer: audio_peer.clone(),
        },
    )
    .await?;

    // Await RadioAcquired then the immediate RuntimeSnapshot. The snapshot reflects
    // the worker's *initial* state (e.g. its default center) before our AcquireRadio
    // tuning is applied — so treat it only as a seed and drive tuning explicitly
    // below, then read the applied state back.
    let (snap_center, snap_target, snap_mode, snap_rate, snap_agc, snap_dbm, snap_s);
    loop {
        match next_server_msg(&mut read, &mut write).await? {
            ServerRadioMessage::RadioAcquired { lease_ttl_ms, .. } => {
                info!("radio acquired (lease ttl {lease_ttl_ms} ms)");
            }
            ServerRadioMessage::RuntimeSnapshot {
                center_freq_hz,
                target_freq_hz,
                input_sample_rate_hz,
                demod_mode,
                squelch_enabled,
                nr2_enabled,
                agc_enabled,
                signal_dbm,
                signal_s_units,
                ..
            } => {
                snap_center = center_freq_hz;
                snap_target = target_freq_hz;
                snap_mode = demod_mode.to_string();
                snap_rate = input_sample_rate_hz as u32;
                snap_agc = agc_enabled;
                snap_dbm = signal_dbm;
                snap_s = signal_s_units;
                info!(
                    "initial snapshot: center={center_freq_hz} target={target_freq_hz} \
                     mode={snap_mode} input_rate={input_sample_rate_hz} \
                     squelch={squelch_enabled} nr2={nr2_enabled} agc={agc_enabled}"
                );
                break;
            }
            ServerRadioMessage::RadioError { code, message } => {
                return Err(format!("acquire failed [{code}]: {message}"));
            }
            other => info!("ignoring post-acquire message: {}", msg_kind(&other)),
        }
    }

    // Start draining the media socket NOW (capturing=false → discard), before the
    // tuning + settle phase. The server began streaming audio at acquire; if we don't
    // read the socket until later, the kernel receive buffer accumulates a multi-second
    // backlog that then floods the jitter buffer the instant the capture window opens
    // (a fixed startup overflow, independent of capture duration).
    let (jitter_target, jitter_max) = args.jitter_samples(PACKET_SAMPLES);
    info!(
        "jitter buffer: target {} ms, max {} ms ({jitter_target}–{jitter_max} samples)",
        args.jitter_target_ms, args.jitter_max_ms
    );
    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        PACKET_SAMPLES,
        jitter_target,
        jitter_max,
    )));
    let stats = Arc::new(Mutex::new(CaptureStats::new()));
    let capturing = Arc::new(AtomicBool::new(false));
    let wav_path = args.wav.as_deref().map(|p| p.display().to_string());
    let (wav_tx, wav_join) = spawn_wav_writer(args.wav.as_deref())?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let media_task = tokio::spawn(media_loop(
        media_sock,
        Arc::clone(&jitter),
        Arc::clone(&stats),
        wav_tx,
        Arc::clone(&capturing),
        shutdown_rx.clone(),
    ));
    let drain_task = tokio::spawn(drain_loop(Arc::clone(&jitter), shutdown_rx.clone()));

    // Drive tuning/mode/rate explicitly — don't rely on the AcquireRadio center
    // being applied (the initial snapshot shows the worker default, not our request).
    send_msg(
        &mut write,
        &ClientRadioMessage::SetCenterFrequency {
            center_freq_hz: center_hz,
        },
    )
    .await?;
    send_msg(
        &mut write,
        &ClientRadioMessage::SetTargetFrequency {
            target_freq_hz: target_hz,
        },
    )
    .await?;
    if let Some(mode_str) = args.mode.as_deref() {
        let mode: DemodMode = mode_str.parse()?;
        send_msg(&mut write, &ClientRadioMessage::SetDemodMode { mode }).await?;
    }
    if let Some(sr) = args.sample_rate {
        send_msg(
            &mut write,
            &ClientRadioMessage::SetSourceSampleRate { sample_rate_hz: sr },
        )
        .await?;
    }

    // Disable squelch + NR2 so the capture is the raw demodulated audio (including the
    // noise floor) — otherwise gating/NR can zero the audio and read as "silence".
    send_msg(
        &mut write,
        &ClientRadioMessage::SetSquelchEnabled { enabled: false },
    )
    .await?;
    send_msg(
        &mut write,
        &ClientRadioMessage::SetNr2Enabled { enabled: false },
    )
    .await?;

    // Optionally set the waterfall frame rate (0 = off) to test its contention impact.
    if let Some(rate_hz) = args.waterfall_rate {
        send_msg(
            &mut write,
            &ClientRadioMessage::SetWaterfallFrameRate { rate_hz },
        )
        .await?;
        info!(
            "waterfall rate set to {rate_hz} Hz{}",
            if rate_hz <= 0.0 { " (disabled)" } else { "" }
        );
    }

    // Shared, live report state — seeded from the snapshot, then kept current from
    // the server's RuntimeChanged messages so the report reflects what we tuned to.
    let report = Arc::new(Mutex::new(ReportState {
        center: snap_center,
        target: snap_target,
        mode: args
            .mode
            .as_deref()
            .map(str::to_string)
            .unwrap_or(snap_mode),
        rate: args.sample_rate.unwrap_or(snap_rate),
        squelch: false,
        nr2: false,
        agc: snap_agc,
        signal_dbm: snap_dbm,
        s_units: snap_s,
    }));

    // Settle: read briefly, applying RuntimeChanged, so the server's applied/clamped
    // center/target/mode land in the report before measurement starts.
    settle(&mut read, &mut write, &report, Duration::from_secs(2)).await;
    {
        let r = report.lock().unwrap();
        info!(
            "tuned: center={} target={} mode={} rate={} Hz | squelch={} nr2={} agc={} | S-meter={:.0} dBm",
            r.center, r.target, r.mode, r.rate, r.squelch, r.nr2, r.agc, r.signal_dbm
        );
    }

    // Control task — lease renewal + ping/pong + live RuntimeChanged tracking.
    let control_task = tokio::spawn(control_loop(
        write,
        read,
        Arc::clone(&report),
        shutdown_rx.clone(),
    ));

    // --- Capture window -----------------------------------------------------
    {
        jitter.lock().unwrap().reset();
        *stats.lock().unwrap() = CaptureStats::new();
        capturing.store(true, Ordering::Release);
    }
    info!("capturing for {} s …", args.duration);
    tokio::time::sleep(Duration::from_secs(args.duration)).await;
    capturing.store(false, Ordering::Release);

    // --- Tear down ----------------------------------------------------------
    let _ = shutdown_tx.send(true);
    // Await the media task so its channel sender drops and the WAV thread finalizes.
    let _ = tokio::time::timeout(Duration::from_secs(2), media_task).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), control_task).await;
    drain_task.abort();
    if let Some(join) = wav_join {
        let _ = join.join();
    }

    // --- Report -------------------------------------------------------------
    let stats = stats.lock().unwrap();
    let jb = jitter.lock().unwrap();
    let r = report.lock().unwrap();
    print_report(
        r.rate,
        args.duration,
        &r.mode,
        r.center,
        r.target,
        r.signal_dbm,
        r.s_units,
        &stats,
        &jb,
        wav_path.as_deref(),
    );

    Ok(())
}

/// Apply a `RuntimeChanged` message to the live report state.
fn apply_runtime_changed(report: &Mutex<ReportState>, msg: &ServerRadioMessage) {
    if let ServerRadioMessage::RuntimeChanged {
        center_freq_hz,
        target_freq_hz,
        demod_mode,
        squelch_enabled,
        nr2_enabled,
        agc_enabled,
        signal_dbm,
        signal_s_units,
        ..
    } = msg
    {
        let mut r = report.lock().unwrap();
        if let Some(c) = center_freq_hz {
            r.center = *c;
        }
        if let Some(t) = target_freq_hz {
            r.target = *t;
        }
        if let Some(m) = demod_mode {
            r.mode = m.to_string();
        }
        if let Some(v) = squelch_enabled {
            r.squelch = *v;
        }
        if let Some(v) = nr2_enabled {
            r.nr2 = *v;
        }
        if let Some(v) = agc_enabled {
            r.agc = *v;
        }
        if let Some(v) = signal_dbm {
            r.signal_dbm = *v;
        }
        if let Some(v) = signal_s_units {
            r.s_units = *v;
        }
    }
}

/// Read messages for up to `dur`, applying each `RuntimeChanged` to `report`, so the
/// server's applied/clamped tuning is reflected before measurement starts.
async fn settle(
    read: &mut WsRead,
    write: &mut WsWrite,
    report: &Mutex<ReportState>,
    dur: Duration,
) {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, next_server_msg(read, write)).await {
            Ok(Ok(msg)) => apply_runtime_changed(report, &msg),
            _ => break,
        }
    }
}

/// UDP receive loop: demux on stream type, decode audio, feed the jitter buffer,
/// the WAV, and the wire-level stats — but only while `capturing`.
async fn media_loop(
    sock: UdpSocket,
    jitter: Arc<Mutex<JitterBuffer>>,
    stats: Arc<Mutex<CaptureStats>>,
    wav_tx: Option<std::sync::mpsc::Sender<Vec<i16>>>,
    capturing: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut buf = vec![0u8; 4096];
    loop {
        let (len, _src) = tokio::select! {
            res = sock.recv_from(&mut buf) => match res {
                Ok(v) => v,
                Err(e) => { warn!("media recv error: {e}"); continue; }
            },
            _ = shutdown_rx.changed() => break,
        };

        if !capturing.load(Ordering::Acquire) {
            continue; // drain socket but don't measure outside the window
        }
        if len < 4 {
            continue;
        }
        let pkt = &buf[..len];
        let stream_type = pkt[3];

        if stream_type == STREAM_TYPE_WATERFALL {
            stats.lock().unwrap().waterfall_pkts += 1;
            continue;
        }
        if stream_type != STREAM_TYPE_AUDIO {
            continue; // register ACK / time-sync / unknown
        }

        let Some(header) = parse_media_header(pkt) else {
            stats.lock().unwrap().bad_pkts += 1;
            continue;
        };
        if !is_valid_header(&header) {
            stats.lock().unwrap().bad_pkts += 1;
            continue;
        }

        let off = audio_samples_offset(header.version);
        let Some(payload) = pkt.get(off..) else {
            stats.lock().unwrap().bad_pkts += 1;
            continue;
        };

        let n = payload.len() / 2;
        let mut f32s: Vec<f32> = Vec::with_capacity(n);
        let mut i16s: Vec<i16> = if wav_tx.is_some() {
            Vec::with_capacity(n)
        } else {
            Vec::new()
        };
        for c in payload.chunks_exact(2) {
            let s = i16::from_le_bytes([c[0], c[1]]);
            f32s.push(s as f32 / i16::MAX as f32);
            if wav_tx.is_some() {
                i16s.push(s);
            }
        }

        let mut peak = 0f32;
        let mut sum_sq = 0f64;
        for &v in &f32s {
            let a = v.abs();
            if a > peak {
                peak = a;
            }
            sum_sq += v as f64 * v as f64;
        }

        {
            let mut s = stats.lock().unwrap();
            s.observe_sequence(header.sequence);
            if let Some(send_ns) = audio_send_wall_ns(&header, pkt) {
                s.observe_send_wall(send_ns);
            }
            s.observe_level(peak, sum_sq);
            s.audio_pkts += 1;
            s.samples += n as u64;
        }

        // Hand samples to the WAV thread (non-blocking) instead of writing here.
        if let Some(tx) = &wav_tx {
            let _ = tx.send(i16s);
        }

        jitter.lock().unwrap().push_packet(header.sequence, f32s);
    }
}

/// Drain the jitter buffer at 48 kHz so its concealment/late/overflow/resync counters
/// reflect a real playout consumer. Pacing is **wall-clock based**: each tick pops the
/// number of samples that *should* have been consumed by now, so a late timer tick
/// (e.g. CPU contention when co-located with the server) catches up on the next tick
/// instead of under-draining and manufacturing false overflows.
async fn drain_loop(jitter: Arc<Mutex<JitterBuffer>>, mut shutdown_rx: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(Duration::from_millis(5));
    let start = tokio::time::Instant::now();
    let mut consumed: u64 = 0;
    let mut scratch = [0f32; 4096];
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let target = (start.elapsed().as_secs_f64() * 48_000.0) as u64;
                let mut remaining = target.saturating_sub(consumed) as usize;
                while remaining > 0 {
                    let n = remaining.min(scratch.len());
                    jitter.lock().unwrap().pop_samples(&mut scratch[..n]);
                    consumed += n as u64;
                    remaining -= n;
                }
            }
            _ = shutdown_rx.changed() => break,
        }
    }
}

/// Lease renewal + ping/pong for the duration of the capture; releases the radio
/// on shutdown.
async fn control_loop(
    mut write: WsWrite,
    mut read: WsRead,
    report: Arc<Mutex<ReportState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut renew = tokio::time::interval(Duration::from_secs(10));
    renew.tick().await; // consume the immediate first tick
    loop {
        tokio::select! {
            _ = renew.tick() => {
                if send_msg(&mut write, &ClientRadioMessage::RenewLease).await.is_err() {
                    break;
                }
            }
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(m) = serde_json::from_str::<ServerRadioMessage>(t.as_str()) {
                            apply_runtime_changed(&report, &m);
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if write.send(Message::Pong(p)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => { warn!("control read error: {e}"); break; }
                    _ => {}
                }
            }
            _ = shutdown_rx.changed() => {
                let _ = send_msg(&mut write, &ClientRadioMessage::ReleaseRadio).await;
                break;
            }
        }
    }
}

/// Read the next decodable `ServerRadioMessage`, answering pings along the way.
async fn next_server_msg(
    read: &mut WsRead,
    write: &mut WsWrite,
) -> Result<ServerRadioMessage, String> {
    loop {
        match read.next().await {
            Some(Ok(Message::Text(t))) => {
                match serde_json::from_str::<ServerRadioMessage>(t.as_str()) {
                    Ok(m) => return Ok(m),
                    Err(e) => warn!("ignoring undecodable server message: {e}"),
                }
            }
            Some(Ok(Message::Ping(p))) => {
                write
                    .send(Message::Pong(p))
                    .await
                    .map_err(|e| format!("pong: {e}"))?;
            }
            Some(Ok(Message::Close(_))) => return Err("server closed the connection".to_string()),
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(format!("websocket read: {e}")),
            None => return Err("websocket stream ended".to_string()),
        }
    }
}

async fn send_msg(write: &mut WsWrite, msg: &ClientRadioMessage) -> Result<(), String> {
    let text = serde_json::to_string(msg).map_err(|e| format!("serialize: {e}"))?;
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| format!("websocket send: {e}"))
}

/// Spawn the WAV writer thread, returning a channel sender for sample chunks and the
/// thread's join handle. Returns `(None, None)` when no `--wav` path was given. Keeping
/// the file I/O off the receive path is what prevents disk stalls from dropping UDP
/// packets and being misread as server-side loss.
fn spawn_wav_writer(
    path: Option<&std::path::Path>,
) -> Result<
    (
        Option<std::sync::mpsc::Sender<Vec<i16>>>,
        Option<std::thread::JoinHandle<()>>,
    ),
    String,
> {
    let Some(path) = path else {
        return Ok((None, None));
    };
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("create WAV {}: {e}", path.display()))?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let join = std::thread::spawn(move || {
        while let Ok(chunk) = rx.recv() {
            for s in chunk {
                let _ = writer.write_sample(s);
            }
        }
        if let Err(e) = writer.finalize() {
            error!("finalize WAV: {e}");
        }
    });
    Ok((Some(tx), Some(join)))
}

/// Short label for an unexpected server message (for logging).
fn msg_kind(msg: &ServerRadioMessage) -> &'static str {
    match msg {
        ServerRadioMessage::RadiosListed { .. } => "radios_listed",
        ServerRadioMessage::RadioAcquired { .. } => "radio_acquired",
        ServerRadioMessage::RadioReleased { .. } => "radio_released",
        ServerRadioMessage::LeaseRenewed { .. } => "lease_renewed",
        ServerRadioMessage::RuntimeSnapshot { .. } => "runtime_snapshot",
        ServerRadioMessage::RuntimeChanged { .. } => "runtime_changed",
        ServerRadioMessage::RadioError { .. } => "radio_error",
    }
}
