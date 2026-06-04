//! Client-side Text-to-CW: text → Morse → existing `StartCwKey`/`StopCwKey`
//! events, with local sidetone.  All Morse encoding and timing live here in the
//! client; the server only ever sees key-down/key-up events (it cannot tell
//! Space-bar keying from Text-to-CW) and keeps owning PTT/sequencing/safety.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::net::control::ControlCommand;
use crate::sidetone::SidetoneShared;
use rigflow_protocol::radio_control::ClientRadioMessage;

/// Default CW memory macros `(label, text)` for the 4 slots (F1–F4).  Safe
/// generic content — the operator edits in their own callsign; we never infer
/// it.  Shared by UI state defaults and persistence defaults.
pub const CW_MACRO_DEFAULTS: [(&str, &str); 4] = [
    ("CQ", "CQ CQ CQ DE YOURCALL YOURCALL K"),
    ("Call", "THEIRCALL DE YOURCALL K"),
    ("RST", "599 599"),
    ("TU", "TU 73 DE YOURCALL SK"),
];

/// Morse code for the supported characters (A–Z, 0–9, common punctuation).
/// Case-insensitive (caller uppercases); unknown characters return `None` and
/// are skipped.  `.` = dit, `-` = dah.
fn morse_for(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_uppercase() {
        'A' => ".-",
        'B' => "-...",
        'C' => "-.-.",
        'D' => "-..",
        'E' => ".",
        'F' => "..-.",
        'G' => "--.",
        'H' => "....",
        'I' => "..",
        'J' => ".---",
        'K' => "-.-",
        'L' => ".-..",
        'M' => "--",
        'N' => "-.",
        'O' => "---",
        'P' => ".--.",
        'Q' => "--.-",
        'R' => ".-.",
        'S' => "...",
        'T' => "-",
        'U' => "..-",
        'V' => "...-",
        'W' => ".--",
        'X' => "-..-",
        'Y' => "-.--",
        'Z' => "--..",
        '0' => "-----",
        '1' => ".----",
        '2' => "..---",
        '3' => "...--",
        '4' => "....-",
        '5' => ".....",
        '6' => "-....",
        '7' => "--...",
        '8' => "---..",
        '9' => "----.",
        '.' => ".-.-.-",
        ',' => "--..--",
        '?' => "..--..",
        '/' => "-..-.",
        '=' => "-...-", // BT
        '+' => ".-.-.", // AR
        _ => return None,
    })
}

/// Build the keying schedule for `text` at `wpm`: one `(key_down, key_up_after)`
/// pair per Morse element (dit/dah).  Standard timing with no double-counted
/// gaps: dit = 1 unit, dah = 3 units; intra-character gap = 1 unit; character
/// gap = 3 units; word gap = 7 units (each gap is the *total*, not additive).
/// `unit_ms = 1200 / wpm`.
pub fn encode_schedule(text: &str, wpm: u32) -> Vec<(Duration, Duration)> {
    let wpm = wpm.clamp(5, 50);
    let unit_ms = 1200.0 / wpm as f32;
    let units = |u: f32| Duration::from_secs_f32(u * unit_ms / 1000.0);

    // Words (whitespace-separated), each a list of known-character Morse strings.
    // Unknown characters are filtered out; empty words are dropped.
    let words: Vec<Vec<&'static str>> = text
        .split_whitespace()
        .map(|w| w.chars().filter_map(morse_for).collect::<Vec<_>>())
        .filter(|w| !w.is_empty())
        .collect();

    let mut sched = Vec::new();
    for (wi, word) in words.iter().enumerate() {
        for (li, morse) in word.iter().enumerate() {
            let elems: Vec<char> = morse.chars().collect();
            for (ei, sym) in elems.iter().enumerate() {
                let on = units(if *sym == '-' { 3.0 } else { 1.0 });
                let off = if ei + 1 < elems.len() {
                    1.0 // gap between elements of the same character
                } else if li + 1 < word.len() {
                    3.0 // gap between characters in a word
                } else if wi + 1 < words.len() {
                    7.0 // gap between words
                } else {
                    0.0 // end of message
                };
                sched.push((on, units(off)));
            }
        }
    }
    sched
}

/// Spawn the Text-to-CW sender on a dedicated thread (so the UI stays
/// responsive and timing is not tied to the egui frame rate).  It walks the
/// schedule sending `StartCwKey`/`StopCwKey` at element boundaries and drives
/// the local sidetone with the *same* timing.  `abort` aborts promptly (a
/// key-down in progress is followed by an immediate key-up); `sending` tracks
/// the active state for the UI.  The server's semi break-in / hang timer then
/// releases PTT safely.
pub fn spawn_send(
    text: String,
    wpm: u32,
    ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    sidetone: Arc<SidetoneShared>,
    abort: Arc<AtomicBool>,
    sending: Arc<AtomicBool>,
) {
    abort.store(false, Ordering::Relaxed);
    sending.store(true, Ordering::Relaxed);

    thread::spawn(move || {
        let sched = encode_schedule(&text, wpm);
        let send = |msg| {
            let _ = ws_cmd_tx.send(ControlCommand::RadioMessage(msg));
        };

        // Track our own key state so we never send duplicate down/up events.
        let mut keyed = false;
        for (on, off) in sched {
            if abort.load(Ordering::Relaxed) {
                break;
            }
            // Key down.
            keyed = true;
            sidetone.set_keyed(true);
            send(ClientRadioMessage::StartCwKey);
            if interruptible_sleep(on, &abort) {
                break; // aborted mid key-down → safety key-up below
            }
            // Key up.
            keyed = false;
            sidetone.set_keyed(false);
            send(ClientRadioMessage::StopCwKey);
            if !off.is_zero() && interruptible_sleep(off, &abort) {
                break;
            }
        }

        // If we aborted during a key-down, ensure the key is released.
        if keyed {
            sidetone.set_keyed(false);
            send(ClientRadioMessage::StopCwKey);
        }

        sending.store(false, Ordering::Relaxed);
    });
}

/// Sleep `dur`, but wake every few ms to check `abort`.  Returns `true` if
/// aborted (so the caller can stop and release the key promptly).
fn interruptible_sleep(dur: Duration, abort: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(5);
    let mut remaining = dur;
    while remaining > Duration::ZERO {
        if abort.load(Ordering::Relaxed) {
            return true;
        }
        let s = remaining.min(SLICE);
        thread::sleep(s);
        remaining = remaining.saturating_sub(s);
    }
    abort.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_characters_are_skipped() {
        // '~' has no Morse; "A~B" encodes the same as "AB".
        assert_eq!(encode_schedule("A~B", 20), encode_schedule("AB", 20));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(encode_schedule("cq", 20), encode_schedule("CQ", 20));
    }

    #[test]
    fn timing_units_and_gaps() {
        let unit = Duration::from_secs_f32(1200.0 / 20.0 / 1000.0); // 60 ms
                                                                    // "A" = .-  → dit(1u) gap(1u) dah(3u), no trailing gap (end of msg).
        let a = encode_schedule("A", 20);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], (unit, unit)); // dit, intra-char gap = 1u
        assert_eq!(a[1], (unit * 3, Duration::ZERO)); // dah, end → no gap

        // "E E" = . (word gap 7u) .  → first dit has a 7-unit trailing gap.
        let ee = encode_schedule("E E", 20);
        assert_eq!(ee.len(), 2);
        assert_eq!(ee[0], (unit, unit * 7)); // word gap
        assert_eq!(ee[1], (unit, Duration::ZERO));
    }
}
