use rigflow_core::audio::jitter_buffer::JitterBuffer;

/// Wire-level packet statistics, derived purely from UDP arrival + header
/// sequence numbers (independent of the jitter buffer's concealment logic).
#[derive(Debug, Default)]
pub struct CaptureStats {
    /// Total audio packets received.
    pub audio_pkts: u64,
    /// Audio samples decoded (and written to the WAV, if enabled).
    pub samples: u64,
    /// Packets dropped before decode (bad/short header).
    pub bad_pkts: u64,
    /// Waterfall packets seen (ignored, counted for context).
    pub waterfall_pkts: u64,

    /// First sequence number observed in the window (the span baseline).
    first_seq: Option<u32>,
    /// Highest in-order sequence number observed so far.
    highest_seq: Option<u32>,
    /// Number of times a forward gap appeared in the sequence.
    pub gap_events: u64,
    /// Total packets missing on the wire across all gaps.
    pub missing_pkts: u64,
    /// Packets that arrived older than the highest seen (reorder/duplicate).
    pub reorder_pkts: u64,

    /// Server send wall-clock (epoch ns) of the first and last received v2 audio
    /// packet — used to measure the server's true production pacing.
    first_send_ns: Option<u64>,
    last_send_ns: Option<u64>,

    /// Peak absolute sample magnitude (normalized 0..1) and running sum of squares,
    /// for an audio level read-out (is the capture signal, or silence?).
    peak: f32,
    sum_sq: f64,
}

impl CaptureStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the server send timestamp (v2 audio packets only).
    pub fn observe_send_wall(&mut self, ns: u64) {
        if self.first_send_ns.is_none() {
            self.first_send_ns = Some(ns);
        }
        self.last_send_ns = Some(ns);
    }

    /// Accumulate the audio level of one packet's samples (normalized -1..1).
    pub fn observe_level(&mut self, peak: f32, sum_sq: f64) {
        if peak > self.peak {
            self.peak = peak;
        }
        self.sum_sq += sum_sq;
    }

    /// Account for one audio packet's sequence number. The first packet
    /// establishes the baseline without counting a gap.
    pub fn observe_sequence(&mut self, seq: u32) {
        match self.highest_seq {
            None => {
                self.first_seq = Some(seq);
                self.highest_seq = Some(seq);
            }
            Some(highest) => {
                let expected = highest.wrapping_add(1);
                // Forward distance from the expected next sequence.
                let ahead = seq.wrapping_sub(expected);
                if seq == expected {
                    self.highest_seq = Some(seq);
                } else if ahead < u32::MAX / 2 {
                    // seq is ahead of expected → `ahead` packets went missing.
                    self.gap_events += 1;
                    self.missing_pkts += ahead as u64;
                    self.highest_seq = Some(seq);
                } else {
                    // seq is behind the highest → reordered or duplicate.
                    self.reorder_pkts += 1;
                }
            }
        }
    }
}

/// Print the per-capture report. `jb` counters are absolute since the buffer is
/// reset at the start of the window, so they read directly as window totals.
#[allow(clippy::too_many_arguments)]
pub fn print_report(
    rate_hz: u32,
    duration_s: u64,
    mode: &str,
    center_hz: u64,
    target_hz: u64,
    signal_dbm: f32,
    s_units: i32,
    stats: &CaptureStats,
    jb: &JitterBuffer,
    wav_path: Option<&str>,
) {
    let expected_pkts = stats.audio_pkts + stats.missing_pkts;
    let loss_pct = if expected_pkts > 0 {
        stats.missing_pkts as f64 / expected_pkts as f64 * 100.0
    } else {
        0.0
    };

    // Server production pacing, measured from the v2 send timestamps. Uses the
    // *sequence* span (received + missing) over the server-clock span, so it is
    // immune to receiver-side drops: it answers "did the server emit audio faster
    // than real time?" — 100% = nominal, >100% = server over-producing.
    let span_pkts = match (stats.first_seq, stats.highest_seq) {
        (Some(a), Some(b)) => b.wrapping_sub(a) as u64 + 1,
        _ => 0,
    };
    let sent_audio_secs = span_pkts as f64 * 240.0 / 48_000.0;
    let send_span_secs = match (stats.first_send_ns, stats.last_send_ns) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 / 1e9,
        _ => 0.0,
    };
    let pacing_pct = if send_span_secs > 0.0 {
        sent_audio_secs / send_span_secs * 100.0
    } else {
        0.0
    };

    println!();
    println!(
        "─── capture: {rate_hz} Hz, {duration_s}s, {mode}, center {center_hz} Hz, target {target_hz} Hz ───"
    );
    println!("  audio packets recv : {}", stats.audio_pkts);
    println!("  audio samples      : {}", stats.samples);

    // Audio level — distinguishes a real capture from silence/a dead band.
    let dbfs = |x: f64| {
        if x > 0.0 {
            20.0 * x.log10()
        } else {
            f64::NEG_INFINITY
        }
    };
    let rms = if stats.samples > 0 {
        (stats.sum_sq / stats.samples as f64).sqrt()
    } else {
        0.0
    };
    let rms_db = dbfs(rms);
    let note = if rms_db < -60.0 {
        "  (≈ silence)"
    } else {
        ""
    };
    println!(
        "  audio level        : peak {:.1} dBFS, rms {:.1} dBFS{note}",
        dbfs(stats.peak as f64),
        rms_db
    );
    println!("  rx signal (S-meter): {signal_dbm:.0} dBm (S{s_units})");

    if send_span_secs > 0.0 {
        println!(
            "  server pacing      : {span_pkts} pkts in {send_span_secs:.2}s server-clock = {pacing_pct:.0}% of real-time  (100% = nominal)"
        );
    }
    println!(
        "  wire seq gaps      : {} ({} pkts missing, {:.3}% loss)",
        stats.gap_events, stats.missing_pkts, loss_pct
    );
    println!("  reordered/dup pkts : {}", stats.reorder_pkts);
    println!("  bad/short pkts     : {}", stats.bad_pkts);
    println!("  waterfall pkts     : {}", stats.waterfall_pkts);
    // Simulated playout — mirrors the client's jitter buffer. These count against a
    // 0.5 s buffer drained at 48 kHz, so bursty (e.g. WiFi) delivery shows overflow
    // here even with zero wire loss. They do NOT affect the WAV (written pre-buffer).
    println!("  -- jitter buffer (simulated playout; does NOT affect the WAV) --");
    println!("  concealed (silence): {}", jb.packets_missing_concealed);
    println!("  dropped late       : {}", jb.packets_dropped_late);
    println!("  dropped overflow   : {}", jb.packets_dropped_overflow);
    println!("  dropped bad size   : {}", jb.packets_dropped_invalid_size);
    println!("  resyncs            : {}", jb.resync_count);
    if let Some(path) = wav_path {
        println!("  wav                : {path}");
    }
    println!();
}
