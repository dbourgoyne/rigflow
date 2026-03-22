use std::collections::{BTreeMap, VecDeque};

#[derive(Debug)]
pub struct JitterBuffer {
    packet_samples: usize,
    target_buffer_samples: usize,
    max_buffer_samples: usize,

    started: bool,
    next_sequence: Option<u32>,

    // Packets waiting to be played, keyed by sequence number.
    pending: BTreeMap<u32, Vec<f32>>,

    // Flattened playout FIFO consumed by audio callback.
    playout: VecDeque<f32>,

    // Stats
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
            started: false,
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

        // If we have already advanced past this sequence, it's late.
        if let Some(next) = self.next_sequence {
            if sequence < next {
                self.packets_dropped_late += 1;
                return;
            }
        }

        // Avoid unbounded growth.
        if self.buffered_samples() >= self.max_buffer_samples {
            self.packets_dropped_overflow += 1;
            return;
        }

        self.pending.entry(sequence).or_insert(samples);
        self.packets_inserted += 1;

        if !self.started && self.buffered_samples() >= self.target_buffer_samples {
            // Start playout at the earliest packet we have.
            if let Some((&first_seq, _)) = self.pending.iter().next() {
                self.next_sequence = Some(first_seq);
                self.started = true;
            }
        }

        self.fill_playout();
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

    fn fill_playout(&mut self) {
        if !self.started {
            return;
        }

        while self.playout.len() < self.target_buffer_samples {
            let seq = match self.next_sequence {
                Some(s) => s,
                None => break,
            };

            if let Some(packet) = self.pending.remove(&seq) {
                for s in packet {
                    self.playout.push_back(s);
                }
            } else {
                // Missing packet: conceal with silence.
                for _ in 0..self.packet_samples {
                    self.playout.push_back(0.0);
                }
                self.packets_missing_concealed += 1;
            }

            self.next_sequence = Some(seq.wrapping_add(1));

            if self.playout.len() >= self.max_buffer_samples {
                break;
            }
        }
    }

    fn apply_drift_correction(&mut self) {
        // Keep buffer near target. This is intentionally gentle.
        let len = self.playout.len();

        // Too much buffered audio: drop a tiny amount to reduce latency growth.
        if len > self.target_buffer_samples + self.packet_samples {
            let drop_count = 16.min(self.playout.len());
            for _ in 0..drop_count {
                self.playout.pop_front();
            }
        }

        // Too little buffered audio: duplicate a tiny amount to avoid underruns.
        else if len < self.target_buffer_samples.saturating_sub(self.packet_samples / 2) {
            if let Some(&last) = self.playout.back() {
                let dup_count = 16;
                for _ in 0..dup_count {
                    self.playout.push_back(last);
                }
            }
        }
    }
}
