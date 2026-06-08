use std::collections::{BTreeMap, VecDeque};

/// Packet-oriented audio jitter buffer.
///
/// Goals:
/// - tolerate modest out-of-order delivery
/// - tolerate packet loss without permanently stalling
/// - start playback only after enough buffered audio exists
/// - keep latency bounded
///
/// Design:
/// - packets are stored by sequence number in `pending`
/// - playout consumes from a sample FIFO
/// - once started, the buffer expects `next_sequence`
/// - if `next_sequence` is missing but later packets exist, the gap is
///   concealed with silence and playout advances
/// - if the buffer gets badly out of sync, it can resynchronize to the
///   earliest available packet
#[derive(Debug)]
pub struct JitterBuffer {
    packet_samples: usize,
    target_buffer_samples: usize,
    max_buffer_samples: usize,
    max_observed_buffered_samples: usize,
    
    started: bool,
    start_sequence: Option<u32>,
    next_sequence: Option<u32>,

    /// Packets waiting to be played, indexed by sequence number.
    pending: BTreeMap<u32, Vec<f32>>,

    /// Sample-level FIFO consumed by the audio callback.
    playout: VecDeque<f32>,

    /// Number of consecutive concealed packets since the last real packet.
    consecutive_conceals: usize,

    pub packets_received: u64,
    pub packets_inserted: u64,
    pub packets_missing_concealed: u64,
    pub packets_dropped_late: u64,
    pub packets_dropped_overflow: u64,
    pub packets_dropped_invalid_size: u64,
    pub resync_count: u64,
}

impl JitterBuffer {
    pub fn new(
        packet_samples: usize,
        target_buffer_samples: usize,
        max_buffer_samples: usize,
    ) -> Self {
        assert!(packet_samples > 0, "packet_samples must be > 0");
        assert!(
            target_buffer_samples >= packet_samples,
            "target_buffer_samples must be >= packet_samples"
        );
        assert!(
            max_buffer_samples >= target_buffer_samples,
            "max_buffer_samples must be >= target_buffer_samples"
        );

        Self {
            packet_samples,
            target_buffer_samples,
            max_buffer_samples,
            max_observed_buffered_samples: 0,

	    started: false,
            start_sequence: None,
            next_sequence: None,

            pending: BTreeMap::new(),
            playout: VecDeque::new(),

            consecutive_conceals: 0,

            packets_received: 0,
            packets_inserted: 0,
            packets_missing_concealed: 0,
            packets_dropped_late: 0,
            packets_dropped_overflow: 0,
            packets_dropped_invalid_size: 0,
            resync_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.started = false;
        self.start_sequence = None;
        self.next_sequence = None;
        self.pending.clear();
        self.playout.clear();
        self.consecutive_conceals = 0;

        self.packets_received = 0;
        self.packets_inserted = 0;
        self.packets_missing_concealed = 0;
        self.packets_dropped_late = 0;
        self.packets_dropped_overflow = 0;
        self.packets_dropped_invalid_size = 0;
        self.resync_count = 0;
    }

    /// Total buffered audio, counting both queued playout samples and pending packets.
    pub fn buffered_samples(&self) -> usize {
        self.playout.len() + self.pending.len() * self.packet_samples
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn max_buffered_samples(&self) -> usize {
        self.max_observed_buffered_samples
    }

    pub fn max_buffered_ms(&self, sample_rate_hz: f32) -> f32 {
        if sample_rate_hz <= 0.0 {
            0.0
        } else {
            self.max_observed_buffered_samples as f32 / sample_rate_hz * 1000.0
        }
    }

    /// Insert one packet into the jitter buffer.
    ///
    /// Behavior:
    /// - empty packets are ignored
    /// - wrong-sized packets are dropped
    /// - late packets older than `next_sequence` are dropped once playout
    ///   has advanced past them
    /// - packets are dropped on overflow when total buffered audio is too large
    pub fn push_packet(&mut self, sequence: u32, samples: Vec<f32>) {
        self.packets_received += 1;

        if samples.is_empty() {
            return;
        }

        if samples.len() != self.packet_samples {
            self.packets_dropped_invalid_size += 1;
            return;
        }

	if let Some(next) = self.next_sequence {
            if sequence < next {
                self.packets_dropped_late += 1;
                return;
            }
        }

        if self.buffered_samples() >= self.max_buffer_samples {
            self.packets_dropped_overflow += 1;
            return;
        }

        let inserted = self.pending.insert(sequence, samples).is_none();
        if inserted {
            self.packets_inserted += 1;
        }

        if self.start_sequence.is_none() {
            self.start_sequence = Some(sequence);
        }

	// Start playback once we have enough buffered audio. We do not require
        // a large perfect contiguous run forever; once playback has started,
        // later gaps are handled by concealment.
        if !self.started && self.can_start_playout() {
            let first = self
                .pending
                .first_key_value()
                .map(|(seq, _)| *seq)
                .unwrap_or(sequence);

            self.start_sequence = Some(first);
            self.next_sequence = Some(first);
            self.started = true;
        }

        self.fill_playout();

        let buffered = self.buffered_samples();
        self.max_observed_buffered_samples =
            self.max_observed_buffered_samples.max(buffered);
    }

    /// Pop audio into the output slice.
    ///
    /// If playback has not started yet, output silence.
    /// If playout underflows after startup, missing samples are concealed with silence.
    pub fn pop_samples(&mut self, out: &mut [f32]) {
        if !self.started {
            out.fill(0.0);
            return;
        }

        self.fill_playout();
        self.apply_drift_correction();

        for sample in out.iter_mut() {
            *sample = self.playout.pop_front().unwrap_or(0.0);
        }
    }

    /// Start once we have enough total buffered audio to avoid instant starvation.
    ///
    /// We prefer robustness over requiring perfect startup contiguity forever.
    fn can_start_playout(&self) -> bool {
        self.pending.len() * self.packet_samples >= self.target_buffer_samples
    }

    /// Move packets into the playout FIFO, concealing gaps when necessary.
    fn fill_playout(&mut self) {
        if !self.started {
            return;
        }

        while self.playout.len() < self.target_buffer_samples {
            let seq = match self.next_sequence {
                Some(sequence) => sequence,
                None => break,
            };

            if let Some(packet) = self.pending.remove(&seq) {
                self.playout.extend(packet);
                self.next_sequence = Some(seq.wrapping_add(1));
                self.consecutive_conceals = 0;
                continue;
            }
	    // If the exact expected packet is missing:
            // - if no later packets are available, wait
            // - if later packets are available, conceal one packet and advance
            let earliest_pending = self.pending.first_key_value().map(|(k, _)| *k);

	    match earliest_pending {
                None => break,

                Some(earliest) if earliest > seq => {
                    // We have moved ahead of the missing packet. Conceal it so
                    // playout can continue.
                    self.playout
                        .extend(std::iter::repeat_n(0.0, self.packet_samples));
                    self.packets_missing_concealed += 1;
                    self.consecutive_conceals += 1;
                    self.next_sequence = Some(seq.wrapping_add(1));

		    // If we conceal repeatedly, we are probably badly out of sync.
                    // Re-anchor to the earliest packet we actually have.
                    if self.consecutive_conceals >= 3 {
                        self.resync_to_earliest_pending();
                    }
                }

                Some(_) => {
                    // earliest_pending < seq should normally not happen because
                    // older packets are dropped in push_packet, but just wait.
                    break;
                }
            }

            if self.playout.len() >= self.max_buffer_samples {
                break;
            }
        }
    }

    fn resync_to_earliest_pending(&mut self) {
        if let Some((&earliest, _)) = self.pending.first_key_value() {
            self.next_sequence = Some(earliest);
            self.consecutive_conceals = 0;
            self.resync_count += 1;
        }
    }

    /// Trim excess playout audio when latency grows above target.
    ///
    /// Policy:
    /// - slightly above target: trim a tiny amount
    /// - moderately above target: trim more
    /// - far above target or near max: pull latency down quickly
    fn apply_drift_correction(&mut self) {
        let buffered_len = self.playout.len();
        let target = self.target_buffer_samples;
        let packet = self.packet_samples;
        let max = self.max_buffer_samples;

        if buffered_len <= target {
            return;
        }

        let excess = buffered_len - target;

        // Mildly above target: trim a tiny amount.
        if excess >= packet / 4 && excess < packet {
            let drop_count = 16.min(self.playout.len());
            for _ in 0..drop_count {
                self.playout.pop_front();
            }
            return;
        }

	// Clearly above target: trim more aggressively.
        if excess >= packet && excess < packet * 4 {
            let drop_count = (packet / 4).max(16).min(self.playout.len());
            for _ in 0..drop_count {
                self.playout.pop_front();
            }
            return;
        }

        // Far above target: pull latency down quickly.
        if excess >= packet * 4 || buffered_len >= max.saturating_sub(packet) {
            let desired = target + packet / 2;
            let drop_count = buffered_len
                .saturating_sub(desired)
                .min(self.playout.len());

            for _ in 0..drop_count {
                self.playout.pop_front();
            }
        }
    }
}
