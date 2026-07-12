//! Split-frequency capture: deriving the ADIF `FREQ` (TX) and `FREQ_RX` (RX)
//! from a snapshot of the radio's receivers.
//!
//! rigflow is an SDR that may run more than one receive channel (dual-watch
//! played as stereo). Only one of them is the station being worked. The client
//! builds a [`CapturedRadioState`] from its live UI state — marking which
//! receiver transmits via the *effective-TX* rule (split → the TX VFO's target
//! plus XIT), never VFO-A blindly — and this module turns it into the two
//! logged frequencies. Keeping the derivation here (with no client/egui deps)
//! makes it unit-testable.

use crate::normalize::band_for_freq_hz;

/// One receive channel's state at capture time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receiver {
    pub freq_hz: u64,
    /// True for the receiver selected for transmit. In split you also *receive*
    /// on your TX frequency (to hear the pileup); that receiver must be marked
    /// `is_tx` so it's excluded from the `FREQ_RX` search.
    pub is_tx: bool,
}

/// A frozen snapshot of the radio's operating state, captured the instant the
/// log entry opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRadioState {
    /// Effective transmit frequency (Hz) — becomes ADIF `FREQ`.
    pub tx_freq_hz: u64,
    /// Normalized ADIF transmit mode (the client maps `DemodMode` → ADIF).
    pub tx_mode: String,
    /// Whether split TX is active. `FREQ_RX` is a split field: when this is
    /// false, `derive_freq_rx` returns `None` regardless of how many receivers
    /// are producing audio (plain dual-watch is not split).
    pub split_active: bool,
    /// Every receive channel currently active (1 or 2 on today's hardware, but
    /// the derivation is general over N).
    pub receivers: Vec<Receiver>,
}

impl CapturedRadioState {
    /// ADIF `FREQ`: the effective transmit frequency.
    pub fn derive_freq_tx(&self) -> u64 {
        self.tx_freq_hz
    }

    /// ADIF `FREQ_RX`: the frequency of the receiver most likely copying the
    /// worked station.
    ///
    /// Rule: among receivers that are **(a)** not the TX receiver and **(b)** on
    /// the same band as TX, pick the one **nearest in frequency to TX**. Gated
    /// on [`split_active`](Self::split_active) — simplex TX yields `None` (write
    /// `FREQ_RX`/`BAND_RX` as NULL). It is a *default*, meant to be shown
    /// editable in the UI: no heuristic can be certain which receiver copied the
    /// DX, only that it's overwhelmingly likely.
    pub fn derive_freq_rx(&self) -> Option<u64> {
        if !self.split_active {
            return None;
        }
        let tx_band = band_for_freq_hz(self.tx_freq_hz)?;
        self.receivers
            .iter()
            .filter(|r| !r.is_tx)
            .filter(|r| band_for_freq_hz(r.freq_hz) == Some(tx_band))
            .min_by_key(|r| r.freq_hz.abs_diff(self.tx_freq_hz))
            .map(|r| r.freq_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx(freq_hz: u64, is_tx: bool) -> Receiver {
        Receiver { freq_hz, is_tx }
    }

    fn state(tx: u64, split: bool, receivers: Vec<Receiver>) -> CapturedRadioState {
        CapturedRadioState {
            tx_freq_hz: tx,
            tx_mode: "SSB".into(),
            split_active: split,
            receivers,
        }
    }

    // (a) Two VFOs, one is TX → picks the other.
    #[test]
    fn two_vfos_picks_non_tx() {
        let s = state(
            14_207_000,
            true,
            vec![rx(14_207_000, true), rx(14_200_000, false)],
        );
        assert_eq!(s.derive_freq_rx(), Some(14_200_000));
    }

    // (b) A receiver sitting on the TX frequency (the pileup echo) is excluded,
    //     so the far DX receiver wins rather than collapsing split→simplex.
    #[test]
    fn tx_echo_receiver_excluded() {
        let s = state(
            14_207_000,
            true,
            vec![
                rx(14_207_000, true),  // TX / pileup echo — excluded
                rx(14_200_000, false), // the DX
            ],
        );
        let rxf = s.derive_freq_rx().unwrap();
        assert_eq!(rxf, 14_200_000);
        assert_ne!(rxf, s.derive_freq_tx(), "must not collapse to simplex");
    }

    // (c) A receiver on a different band is excluded even if numerically nearer
    //     in raw Hz.
    #[test]
    fn other_band_receiver_excluded_even_if_nearer() {
        // A 30m receiver at 10.150 is only 50 kHz from a (hypothetical) tx, but
        // it's a different band; the same-band 20m receiver must win.
        let s = state(
            14_100_000,
            true,
            vec![
                rx(14_100_000, true),  // TX
                rx(10_150_000, false), // 30m — nearer in raw Hz? no, but different band
                rx(14_050_000, false), // 20m — same band, the real DX
            ],
        );
        assert_eq!(s.derive_freq_rx(), Some(14_050_000));
    }

    // Sharper version of (c): the other-band receiver is genuinely closer in raw Hz.
    #[test]
    fn other_band_nearer_in_hz_still_excluded() {
        // TX 14.000 (20m band edge). A 17m receiver at 18.068 is 4.068 MHz away;
        // put an out-of-band receiver 1 kHz away to prove raw-Hz nearness loses.
        let s = state(
            14_000_000,
            true,
            vec![
                rx(14_000_000, true),  // TX
                rx(13_999_000, false), // 1 kHz away but OUT of any ham band
                rx(14_100_000, false), // 100 kHz away, same 20m band → must win
            ],
        );
        assert_eq!(s.derive_freq_rx(), Some(14_100_000));
    }

    // (d) Three receivers → nearest-to-TX on the same band wins.
    #[test]
    fn three_receivers_nearest_on_band_wins() {
        let s = state(
            14_250_000,
            true,
            vec![
                rx(14_250_000, true),  // TX
                rx(14_100_000, false), // 150 kHz away
                rx(14_240_000, false), // 10 kHz away → nearest, wins
                rx(14_300_000, false), // 50 kHz away
            ],
        );
        assert_eq!(s.derive_freq_rx(), Some(14_240_000));
    }

    // (e) Simplex TX → None even with multiple receivers active (dual-watch is
    //     not split).
    #[test]
    fn simplex_multi_receiver_is_none() {
        let s = state(
            14_207_000,
            false, // not split
            vec![rx(14_207_000, true), rx(14_100_000, false)],
        );
        assert_eq!(s.derive_freq_rx(), None);
    }

    #[test]
    fn split_but_no_non_tx_candidate_is_none() {
        // Split active but only the TX receiver exists → nothing to derive.
        let s = state(14_207_000, true, vec![rx(14_207_000, true)]);
        assert_eq!(s.derive_freq_rx(), None);
    }
}
