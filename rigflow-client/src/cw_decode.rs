//! Client-side assistive CW-to-text decoder.
//!
//! Runs in the media (UDP) thread on the *already-received* audio (so it never
//! touches the receive audio path — no gain/volume/AGC/squelch/NR2 changes, and
//! nothing is transmitted).  A Goertzel detector at the CW pitch turns the audio
//! into tone on/off, a ratio-based classifier turns on/off durations into
//! dits/dahs and element/character/word gaps, and a reverse Morse table turns
//! symbols into text appended to a shared buffer the UI displays.
//!
//! V1 assumptions: one signal at a time, tuned correctly, reasonably strong.
//! It is assistive — not expected to decode crowded/weak/noisy bands.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Cap on the decoded-text buffer (oldest characters are dropped past this).
const MAX_TEXT_CHARS: usize = 4000;

/// Control + decoded-text output shared between the UI thread (writer of the
/// controls / reader of the text) and the media thread (the decoder).
#[derive(Debug)]
pub struct CwDecodeShared {
    enabled: AtomicBool,
    /// Target tone frequency in Hz (= CW pitch).  f32 bits.
    pitch_hz_bits: AtomicU32,
    /// Manual WPM seed (the CW Speed setting); bootstraps the dit estimate.
    wpm: AtomicU32,
    /// Current auto-estimated WPM, for display.
    est_wpm: AtomicU32,
    /// Decoded text (UI reads + Clear; decoder appends).
    text: Mutex<String>,
}

impl Default for CwDecodeShared {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            pitch_hz_bits: AtomicU32::new(600.0f32.to_bits()),
            wpm: AtomicU32::new(20),
            est_wpm: AtomicU32::new(20),
            text: Mutex::new(String::new()),
        }
    }
}

impl CwDecodeShared {
    pub fn set_enabled(&self, v: bool) {
        self.enabled.store(v, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn set_pitch_hz(&self, hz: f32) {
        self.pitch_hz_bits
            .store(hz.max(0.0).to_bits(), Ordering::Relaxed);
    }
    pub fn pitch_hz(&self) -> f32 {
        f32::from_bits(self.pitch_hz_bits.load(Ordering::Relaxed))
    }
    pub fn set_wpm(&self, w: u32) {
        self.wpm.store(w.clamp(5, 50), Ordering::Relaxed);
    }
    pub fn wpm(&self) -> u32 {
        self.wpm.load(Ordering::Relaxed)
    }
    pub fn set_est_wpm(&self, w: u32) {
        self.est_wpm.store(w, Ordering::Relaxed);
    }
    pub fn est_wpm(&self) -> u32 {
        self.est_wpm.load(Ordering::Relaxed)
    }
    pub fn text(&self) -> String {
        self.text.lock().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn clear(&self) {
        if let Ok(mut s) = self.text.lock() {
            s.clear();
        }
    }
    fn push(&self, c: char) {
        if let Ok(mut s) = self.text.lock() {
            s.push(c);
            if s.chars().count() > MAX_TEXT_CHARS {
                // Drop oldest characters, keeping the tail.
                let keep: String = s.chars().skip(s.chars().count() - MAX_TEXT_CHARS).collect();
                *s = keep;
            }
        }
    }
}

/// Reverse Morse lookup (dits/dahs → character).  Mirror of the encode table in
/// `cw_text`; unknown patterns return `None` (decoded as `?`).
fn decode_morse(s: &str) -> Option<char> {
    Some(match s {
        ".-" => 'A',
        "-..." => 'B',
        "-.-." => 'C',
        "-.." => 'D',
        "." => 'E',
        "..-." => 'F',
        "--." => 'G',
        "...." => 'H',
        ".." => 'I',
        ".---" => 'J',
        "-.-" => 'K',
        ".-.." => 'L',
        "--" => 'M',
        "-." => 'N',
        "---" => 'O',
        ".--." => 'P',
        "--.-" => 'Q',
        ".-." => 'R',
        "..." => 'S',
        "-" => 'T',
        "..-" => 'U',
        "...-" => 'V',
        ".--" => 'W',
        "-..-" => 'X',
        "-.--" => 'Y',
        "--.." => 'Z',
        "-----" => '0',
        ".----" => '1',
        "..---" => '2',
        "...--" => '3',
        "....-" => '4',
        "....." => '5',
        "-...." => '6',
        "--..." => '7',
        "---.." => '8',
        "----." => '9',
        ".-.-.-" => '.',
        "--..--" => ',',
        "..--.." => '?',
        "-..-." => '/',
        "-...-" => '=',
        ".-.-." => '+',
        _ => return None,
    })
}

/// The decoder state machine, owned by the media thread.
pub struct CwDecoder {
    shared: Arc<CwDecodeShared>,
    sample_rate: f32,
    block_len: usize,
    block_ms: f32,
    buf: Vec<f32>,

    prev_enabled: bool,

    // Envelope tracking (Goertzel magnitude) for an adaptive on/off threshold.
    floor: f32,
    peak: f32,
    tone_on: bool,
    state_ms: f32, // time spent in the current on/off state

    // Morse accumulation.
    symbol: String,
    char_flushed: bool, // current off period already produced a character
    word_flushed: bool, // current off period already produced a space

    // Estimated dit length (ms); drives dit/dah and gap thresholds.
    dit_ms: f32,
}

impl CwDecoder {
    pub fn new(shared: Arc<CwDecodeShared>, sample_rate: f32) -> Self {
        // ~6 ms detection blocks — fine enough for 5–50 WPM (dit 24–240 ms).
        let block_len = ((sample_rate * 0.006) as usize).max(64);
        let block_ms = block_len as f32 / sample_rate * 1000.0;
        Self {
            shared,
            sample_rate,
            block_len,
            block_ms,
            buf: Vec::with_capacity(block_len),
            prev_enabled: false,
            floor: 0.0,
            peak: 0.0,
            tone_on: false,
            state_ms: 0.0,
            symbol: String::new(),
            char_flushed: true,
            word_flushed: true,
            dit_ms: 60.0,
        }
    }

    /// Feed received audio (48 kHz mono) — only processes when decode is enabled.
    pub fn process(&mut self, samples: &[f32]) {
        let enabled = self.shared.enabled();
        if enabled != self.prev_enabled {
            self.prev_enabled = enabled;
            if enabled {
                self.reset_runtime();
                self.dit_ms = (1200.0 / self.shared.wpm() as f32).clamp(24.0, 240.0);
            }
        }
        if !enabled {
            return;
        }

        let pitch = self.shared.pitch_hz();
        for &s in samples {
            self.buf.push(s);
            if self.buf.len() >= self.block_len {
                self.process_block(pitch);
                self.buf.clear();
            }
        }
    }

    fn reset_runtime(&mut self) {
        self.buf.clear();
        self.floor = 0.0;
        self.peak = 0.0;
        self.tone_on = false;
        self.state_ms = 0.0;
        self.symbol.clear();
        self.char_flushed = true;
        self.word_flushed = true;
    }

    fn process_block(&mut self, pitch_hz: f32) {
        let mag = self.goertzel(pitch_hz);

        // Fast-attack / slow-decay peak, fast-drop / slow-rise floor → an
        // adaptive threshold with hysteresis.
        if mag > self.peak {
            self.peak = mag;
        } else {
            self.peak = self.peak * 0.99 + mag * 0.01;
        }
        if mag < self.floor {
            self.floor = mag;
        } else {
            self.floor = self.floor * 0.99 + mag * 0.01;
        }

        let span = self.peak - self.floor;
        // Require the tone to sit clearly above the noise floor before decoding.
        let present = self.peak > self.floor * 2.0 + 1e-4 && span > 1e-4;
        let on_thr = self.floor + 0.6 * span;
        let off_thr = self.floor + 0.4 * span;
        let on = present
            && if self.tone_on {
                mag > off_thr
            } else {
                mag > on_thr
            };

        if on != self.tone_on {
            if self.tone_on {
                // Tone just ended: classify the key-down we just finished.
                self.classify_keydown(self.state_ms);
            }
            self.tone_on = on;
            self.state_ms = 0.0;
            if on {
                // Entering a tone: reset the off-period flush flags.
                self.char_flushed = false;
                self.word_flushed = false;
            }
        }

        self.state_ms += self.block_ms;

        if !self.tone_on {
            self.check_off_gaps(self.state_ms);
        }
    }

    /// Goertzel magnitude at `pitch_hz` over the current block, normalised.
    fn goertzel(&self, pitch_hz: f32) -> f32 {
        let n = self.buf.len();
        if n == 0 {
            return 0.0;
        }
        let w = std::f32::consts::TAU * pitch_hz / self.sample_rate;
        let coeff = 2.0 * w.cos();
        let (mut s_prev, mut s_prev2) = (0.0f32, 0.0f32);
        for &x in &self.buf {
            let s = x + coeff * s_prev - s_prev2;
            s_prev2 = s_prev;
            s_prev = s;
        }
        let power = s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2;
        power.max(0.0).sqrt() / n as f32
    }

    fn classify_keydown(&mut self, dur_ms: f32) {
        // dit < 2 units < dah; adapt the dit estimate toward the implied dit.
        if dur_ms < 2.0 * self.dit_ms {
            self.symbol.push('.');
            self.adapt_dit(dur_ms);
        } else {
            self.symbol.push('-');
            self.adapt_dit(dur_ms / 3.0);
        }
        log::debug!(
            "[cw-decode] key-down {dur_ms:.0} ms → symbol {} (dit≈{:.0} ms)",
            self.symbol,
            self.dit_ms
        );
    }

    fn adapt_dit(&mut self, implied_dit_ms: f32) {
        const ALPHA: f32 = 0.2;
        self.dit_ms = ((1.0 - ALPHA) * self.dit_ms + ALPHA * implied_dit_ms).clamp(24.0, 240.0);
        self.shared
            .set_est_wpm((1200.0 / self.dit_ms).round().clamp(5.0, 50.0) as u32);
    }

    fn check_off_gaps(&mut self, off_ms: f32) {
        // Character gap ≈ 3 units (boundary ~2 units): flush the current symbol.
        if !self.char_flushed && !self.symbol.is_empty() && off_ms >= 2.0 * self.dit_ms {
            let c = decode_morse(&self.symbol).unwrap_or('?');
            log::debug!("[cw-decode] symbol {} → '{c}'", self.symbol);
            self.shared.push(c);
            self.symbol.clear();
            self.char_flushed = true;
        }
        // Word gap ≈ 7 units (boundary ~5 units): append a space.
        if !self.word_flushed && off_ms >= 5.0 * self.dit_ms {
            self.shared.push(' ');
            self.word_flushed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_patterns() {
        assert_eq!(decode_morse("-"), Some('T'));
        assert_eq!(decode_morse(".-"), Some('A'));
        assert_eq!(decode_morse("-"), Some('T'));
        assert_eq!(decode_morse("...-"), Some('V'));
        assert_eq!(decode_morse("...--"), Some('3'));
        assert_eq!(decode_morse("-..-."), Some('/'));
        assert_eq!(decode_morse(".-.-."), Some('+'));
        // TEST = - . ... -
        for (sym, ch) in [("-", 'T'), (".", 'E'), ("...", 'S'), ("-", 'T')] {
            assert_eq!(decode_morse(sym), Some(ch));
        }
        // Unknown pattern.
        assert_eq!(decode_morse("........"), None);
    }
}
