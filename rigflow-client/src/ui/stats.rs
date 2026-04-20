use std::time::{Duration, Instant};

use log::{trace, warn};
use rigflow_core::audio::jitter_buffer::JitterBuffer;

use crate::net::udp::MediaPacketStats;

/// Periodic client-side media statistics logger.
///
/// Tracks:
/// - audio sample throughput
/// - media packet rates
/// - jitter buffer depth
/// - packet loss / lateness counters
/// - jitter buffer conceal / resync behavior
pub struct ClientStatsLogger {
    last_log: Instant,
    audio_samples_since_last: u64,

    // Last observed cumulative jitter-buffer counters.
    last_jb_concealed: u64,
    last_jb_late: u64,
    last_jb_overflow: u64,
    last_jb_invalid: u64,
    last_jb_resync: u64,
}

impl ClientStatsLogger {
    pub fn new() -> Self {
        Self {
            last_log: Instant::now(),
            audio_samples_since_last: 0,
            last_jb_concealed: 0,
            last_jb_late: 0,
            last_jb_overflow: 0,
            last_jb_invalid: 0,
            last_jb_resync: 0,
        }
    }

    /// Record audio samples produced during the current logging interval.
    pub fn add_audio_samples(&mut self, count: usize) {
        self.audio_samples_since_last += count as u64;
    }

    /// Log one client stats line once per second.
    pub fn maybe_log(
        &mut self,
        media_stats: &mut MediaPacketStats,
        jb: &JitterBuffer,
        audio_sample_rate_hz: f32,
    ) {
        let elapsed = self.last_log.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }

        let secs = elapsed.as_secs_f64();

        let audio_rate = self.audio_samples_since_last as f64 / secs;
        let packet_rate = media_stats.audio_packets as f64 / secs;
        let waterfall_rate = media_stats.waterfall_packets as f64 / secs;

        let jb_samples = jb.buffered_samples();
        let jb_ms = if audio_sample_rate_hz > 0.0 {
            jb_samples as f64 / audio_sample_rate_hz as f64 * 1000.0
        } else {
            0.0
        };

        let conceal_delta =
            jb.packets_missing_concealed.saturating_sub(self.last_jb_concealed);
        let jb_late_delta =
            jb.packets_dropped_late.saturating_sub(self.last_jb_late);
        let jb_overflow_delta =
            jb.packets_dropped_overflow.saturating_sub(self.last_jb_overflow);
        let jb_invalid_delta = jb
            .packets_dropped_invalid_size
            .saturating_sub(self.last_jb_invalid);
        let jb_resync_delta = jb.resync_count.saturating_sub(self.last_jb_resync);

        trace!(
            "client stats: audio={:.1} ksps packets={:.0}/s wf={:.0}/s \
jb={:.1} ms started={} \
conceal={} jb_late={} jb_overflow={} jb_invalid={} jb_resync={} \
late_a={} drop_a={} late_w={} drop_w={}",
            audio_rate / 1_000.0,
            packet_rate,
            waterfall_rate,
            jb_ms,
            jb.started(),
            jb.packets_missing_concealed,
            jb.packets_dropped_late,
            jb.packets_dropped_overflow,
            jb.packets_dropped_invalid_size,
            jb.resync_count,
            media_stats.late_audio_packets,
            media_stats.dropped_audio_packets,
            media_stats.late_waterfall_packets,
            media_stats.dropped_waterfall_packets,
        );

        if conceal_delta > 0
            || jb_late_delta > 0
            || jb_overflow_delta > 0
            || jb_invalid_delta > 0
            || jb_resync_delta > 0
        {
            warn!(
                "jitter buffer event: conceal_delta={} jb_late_delta={} \
jb_overflow_delta={} jb_invalid_delta={} jb_resync_delta={} jb={:.1} ms started={}",
                conceal_delta,
                jb_late_delta,
                jb_overflow_delta,
                jb_invalid_delta,
                jb_resync_delta,
                jb_ms,
                jb.started(),
            );
        }

        self.last_jb_concealed = jb.packets_missing_concealed;
        self.last_jb_late = jb.packets_dropped_late;
        self.last_jb_overflow = jb.packets_dropped_overflow;
        self.last_jb_invalid = jb.packets_dropped_invalid_size;
        self.last_jb_resync = jb.resync_count;

        // Reset interval-local network counters after each log emission.
        self.audio_samples_since_last = 0;
        media_stats.incoming_packets = 0;
        media_stats.audio_packets = 0;
        media_stats.waterfall_packets = 0;
        media_stats.dropped_audio_packets = 0;
        media_stats.dropped_waterfall_packets = 0;
        media_stats.late_audio_packets = 0;
        media_stats.late_waterfall_packets = 0;
        self.last_log = Instant::now();
    }
}
