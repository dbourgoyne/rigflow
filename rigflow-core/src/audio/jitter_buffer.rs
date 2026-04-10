use std::collections::{BTreeMap, VecDeque};

#[derive(Debug)]
pub struct JitterBuffer {
    packet_samples: usize,
    target_buffer_samples: usize,
    max_buffer_samples: usize,
    max_observed_buffered_samples: usize,

    started: bool,
    start_sequence: Option<u32>,
    next_sequence: Option<u32>,

    pending: BTreeMap<u32, Vec<f32>>,
    playout: VecDeque<f32>,

    pub packets_received: u64,
    pub packets_inserted: u64,
    pub packets_missing_concealed: u64,
    pub packets_dropped_late: u64,
    pub packets_dropped_overflow: u64,
}

impl JitterBuffer {
    pub fn new(
        packet_samples: usize,
        target_buffer_samples: usize,
        max_buffer_samples: usize,
    ) -> Self {
        assert!(packet_samples > 0);
        assert!(target_buffer_samples >= packet_samples);
        assert!(max_buffer_samples >= target_buffer_samples);

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
            packets_received: 0,
            packets_inserted: 0,
            packets_missing_concealed: 0,
            packets_dropped_late: 0,
            packets_dropped_overflow: 0,
        }
    }

    pub fn reset(&mut self) {
        self.started = false;
        self.start_sequence = None;
        self.next_sequence = None;
        self.pending.clear();
        self.playout.clear();
        self.packets_received = 0;
        self.packets_inserted = 0;
        self.packets_missing_concealed = 0;
        self.packets_dropped_late = 0;
        self.packets_dropped_overflow = 0;
    }

    pub fn buffered_samples(&self) -> usize {
        self.playout.len() + self.pending.len() * self.packet_samples
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn push_packet(&mut self, sequence: u32, samples: Vec<f32>) {
        self.packets_received += 1;

        if samples.is_empty() {
            return;
        }

	if let Some(next) = self.next_sequence && sequence < next {
            self.packets_dropped_late += 1;
            return;
        }

        if self.buffered_samples() >= self.max_buffer_samples {
            self.packets_dropped_overflow += 1;
            return;
        }

        self.pending.entry(sequence).or_insert(samples);
        self.packets_inserted += 1;

        if self.start_sequence.is_none() {
            self.start_sequence = Some(sequence);
        }

        if !self.started && self.has_enough_contiguous_packets() {
            let start = self.start_sequence.unwrap();
            self.next_sequence = Some(start);
            self.started = true;
        }

        self.fill_playout();

	let buffered = self.buffered_samples();
	self.max_observed_buffered_samples =
	    self.max_observed_buffered_samples.max(buffered);
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

    fn has_enough_contiguous_packets(&self) -> bool {
        let mut count = 0usize;
        let mut seq = match self.start_sequence {
            Some(s) => s,
            None => return false,
        };

        while self.pending.contains_key(&seq) {
            count += self.packet_samples;
            if count >= self.target_buffer_samples {
                return true;
            }
            seq = seq.wrapping_add(1);
        }

        false
    }

    fn fill_playout(&mut self) {
        if !self.started {
            return;
        }

        while self.playout.len() < self.target_buffer_samples {
            let seq = match self.next_sequence {
                Some(s) => s,
                None => break,
            };

            match self.pending.remove(&seq) {
                Some(packet) => {
                    for s in packet {
                        self.playout.push_back(s);
                    }
                    self.next_sequence = Some(seq.wrapping_add(1));
                }
                None => {
                    break;
                }
            }

            if self.playout.len() >= self.max_buffer_samples {
                break;
            }
        }
    }

    fn apply_drift_correction(&mut self) {
	let len = self.playout.len();
	let target = self.target_buffer_samples;
	let packet = self.packet_samples;
	let max = self.max_buffer_samples;

	if len <= target {
            return;
	}

	let excess = len - target;

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
	if excess >= packet * 4 || len >= max.saturating_sub(packet) {
            let desired = target + packet / 2;
            let drop_count = len.saturating_sub(desired).min(self.playout.len());

            for _ in 0..drop_count {
		self.playout.pop_front();
            }
	}
    }

    /*
     * Old function
     *
    fn apply_drift_correction(&mut self) {
        let len = self.playout.len();

        if len > self.target_buffer_samples + self.packet_samples {
            let drop_count = 16.min(self.playout.len());
            for _ in 0..drop_count {
                self.playout.pop_front();
            }
        }
    }
    */
}
