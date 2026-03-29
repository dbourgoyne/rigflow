use std::time::{Duration, Instant};

use log::info;

use crate::net::udp::MediaPacketStats;

pub struct ClientStatsLogger {
    last_log: Instant,
    audio_samples_since_last: u64,
}

impl ClientStatsLogger {
    pub fn new() -> Self {
        Self {
            last_log: Instant::now(),
            audio_samples_since_last: 0,
        }
    }

    pub fn add_audio_samples(&mut self, count: usize) {
        self.audio_samples_since_last += count as u64;
    }

    pub fn maybe_log(
        &mut self,
        media_stats: &mut MediaPacketStats,
        jitter_buffer_samples: usize,
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

        let jitter_ms = if audio_sample_rate_hz > 0.0 {
            jitter_buffer_samples as f64 / audio_sample_rate_hz as f64 * 1000.0
        } else {
            0.0
        };

        info!(
            "client stats: audio={:.1} ksps packets={:.0}/s wf={:.0}/s jitter={:.1} ms late_a={} drop_a={} late_w={} drop_w={}",
            audio_rate / 1_000.0,
            packet_rate,
            waterfall_rate,
            jitter_ms,
            media_stats.late_audio_packets,
            media_stats.dropped_audio_packets,
            media_stats.late_waterfall_packets,
            media_stats.dropped_waterfall_packets,
        );

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
