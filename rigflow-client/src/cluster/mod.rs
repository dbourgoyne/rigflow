//! DX-cluster spots — pure model, wire-line parser, and spot-store lifecycle
//! (dedupe + expiry). **This module is network-free and unit-tested**; the telnet
//! thread (Phase 2), `UiState` wiring + filtering (Phase 3), and waterfall/spectrum
//! rendering (Phase 4) live elsewhere. Design: `docs/release-review/dx-cluster-design.md`.
//!
//! Clusters are peered — every public node carries the same global feed — and speak
//! line-based telnet. The one line we care about is the spot:
//!
//! ```text
//! DX de W3LPL:     14025.0  JA1XYZ       CW  up 2                1432Z
//!      └spotter    └freq kHz └DX call     └───comment────┘        └time UTC
//! ```
//!
//! Line formats vary across cluster software (DXSpider / AR-Cluster / CC-Cluster):
//! spacing differs and some append the DX country/grid to the comment. So the parser
//! anchors on the `DX de` prefix and the freq/call tokens, tolerates extra trailing
//! fields, and **drops any line it can't confidently parse** rather than guessing.

pub mod client;
pub mod config;

use std::time::{Duration, Instant};

use rigflow_core::radio::ham_band::{HamBand, band_from_frequency};

/// A parsed cluster spot. Wire fields plus a derived `band` and a local
/// `received_at` (set by the caller / parser, not read off the wire) used for
/// aging and expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxSpot {
    /// The spotted station (uppercased).
    pub dx_call: String,
    /// Spot frequency in Hz (wire is kHz).
    pub freq_hz: u64,
    /// Who posted the spot.
    pub spotter: String,
    /// Free-text comment (mode, "up 2", signal report, DX country, …).
    pub comment: String,
    /// Time as reported on the wire, e.g. `"1432Z"` — display-only.
    pub time_utc: String,
    /// Best-effort mode from the comment (`"FT8"`, `"CW"`, …); `None` if unknown.
    pub mode_hint: Option<String>,
    /// Ham band derived from `freq_hz`; `None` for out-of-band (e.g. VHF/UHF)
    /// or unmapped frequencies — still a valid spot, just not on a known band.
    pub band: Option<HamBand>,
    /// When this spot was received locally. Used for aging/expiry only.
    pub received_at: Instant,
}

/// Two spots of the **same call** within this many Hz are treated as the same
/// spot (people spot the same station a hair apart). ±0.2 kHz.
pub const DEDUPE_TOL_HZ: u64 = 200;

/// Default time-to-live for a spot before it expires off the display.
pub const DEFAULT_TTL: Duration = Duration::from_secs(20 * 60);

/// Hard cap on retained spots (memory bound). Oldest is dropped past this.
pub const MAX_SPOTS: usize = 4000;

/// Parse one cluster line into a [`DxSpot`], or `None` if it isn't a spot line
/// (or is malformed). `received_at` is injected so the parser stays pure and the
/// tests are deterministic.
pub fn parse_spot_line(line: &str, received_at: Instant) -> Option<DxSpot> {
    // Anchor: the line must begin with "DX de" (case-insensitive). Everything
    // else the node sends — WWV/WCY bulletins, chat, announcements, prompts — is
    // silently ignored.
    let line = line.trim();
    let rest = strip_prefix_ci(line, "DX de")?.trim_start();

    // Spotter runs up to the first ':' (its own colon; comments rarely lead with
    // one, and the freq/call never contain ':'). Fall back to the first token if
    // a node omits the colon.
    let (spotter, after) = match rest.split_once(':') {
        Some((s, a)) => (s.trim(), a.trim()),
        None => {
            let mut it = rest.splitn(2, char::is_whitespace);
            (
                it.next().unwrap_or("").trim(),
                it.next().unwrap_or("").trim(),
            )
        }
    };
    if spotter.is_empty() {
        return None;
    }

    let mut tokens: Vec<&str> = after.split_whitespace().collect();
    if tokens.len() < 2 {
        return None; // need at least freq + call
    }

    // Frequency (kHz → Hz).
    let khz: f64 = tokens[0].parse().ok()?;
    if !(khz.is_finite() && khz > 0.0) {
        return None;
    }
    let freq_hz = (khz * 1000.0).round() as u64;

    // DX call.
    let dx_call = tokens[1].to_ascii_uppercase();
    if dx_call.is_empty() {
        return None;
    }

    // Trailing "NNNNZ" time, if present, is peeled off the end; the rest is the
    // free-text comment.
    let mut time_utc = String::new();
    if let Some(last) = tokens.last() {
        if is_time_token(last) {
            time_utc = last.to_ascii_uppercase();
            tokens.pop();
        }
    }
    let comment = tokens.get(2..).map(|c| c.join(" ")).unwrap_or_default();

    Some(DxSpot {
        dx_call,
        freq_hz,
        spotter: spotter.to_string(),
        mode_hint: mode_hint_from(&comment),
        comment,
        time_utc,
        band: band_from_frequency(freq_hz),
        received_at,
    })
}

/// Insert a spot, collapsing a prior spot of the same call within
/// [`DEDUPE_TOL_HZ`] (newest wins), and enforcing [`MAX_SPOTS`] by dropping the
/// oldest. Keeps the store deduped and bounded.
pub fn insert_spot(spots: &mut Vec<DxSpot>, spot: DxSpot) {
    spots.retain(|s| !(s.dx_call == spot.dx_call && freq_close(s.freq_hz, spot.freq_hz)));
    spots.push(spot);
    while spots.len() > MAX_SPOTS {
        if let Some((i, _)) = spots.iter().enumerate().min_by_key(|(_, s)| s.received_at) {
            spots.remove(i);
        } else {
            break;
        }
    }
}

/// Drop spots older than `ttl` relative to `now`.
pub fn expire(spots: &mut Vec<DxSpot>, now: Instant, ttl: Duration) {
    spots.retain(|s| now.saturating_duration_since(s.received_at) < ttl);
}

fn freq_close(a: u64, b: u64) -> bool {
    a.abs_diff(b) <= DEDUPE_TOL_HZ
}

/// Case-insensitive prefix strip returning the remainder.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let plen = prefix.len();
    if s.len() >= plen && s[..plen].eq_ignore_ascii_case(prefix) {
        Some(&s[plen..])
    } else {
        None
    }
}

/// A trailing spot-time token: 3–5 ASCII digits followed by `Z`/`z`, e.g.
/// `1432Z`. (Not a real clock — just what the node printed.)
fn is_time_token(tok: &str) -> bool {
    let bytes = tok.as_bytes();
    let n = bytes.len();
    if !(4..=5).contains(&n) {
        return false;
    }
    if !matches!(bytes[n - 1], b'Z' | b'z') {
        return false;
    }
    bytes[..n - 1].iter().all(|b| b.is_ascii_digit())
}

/// Best-effort mode from a comment: first recognised digital/CW/phone keyword.
/// Order matters — check longer/more-specific tokens first.
fn mode_hint_from(comment: &str) -> Option<String> {
    const MODES: [&str; 8] = ["FT8", "FT4", "RTTY", "PSK", "JT65", "CW", "SSB", "USB"];
    let up = comment.to_ascii_uppercase();
    MODES
        .into_iter()
        .find(|m| {
            up.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w == *m)
        })
        .map(|m| m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn parses_dxspider_line() {
        let s = parse_spot_line(
            "DX de W3LPL:     14025.0  JA1XYZ       CW  up 2                1432Z",
            t0(),
        )
        .expect("should parse");
        assert_eq!(s.spotter, "W3LPL");
        assert_eq!(s.freq_hz, 14_025_000);
        assert_eq!(s.dx_call, "JA1XYZ");
        assert_eq!(s.time_utc, "1432Z");
        assert_eq!(s.band, Some(HamBand::B20));
        assert_eq!(s.mode_hint.as_deref(), Some("CW"));
        assert!(s.comment.contains("up 2"));
        assert!(!s.comment.contains("1432Z")); // time peeled off
    }

    #[test]
    fn parses_ar_cluster_line_with_ssid_and_trailing_country() {
        // AR-Cluster: spotter has an SSID suffix, comment carries a signal
        // report and the DX country.
        let s = parse_spot_line(
            "DX de K1ABC-#:   14195.0  EA8XYZ    59 Canary Is           2030Z",
            t0(),
        )
        .expect("should parse");
        assert_eq!(s.spotter, "K1ABC-#");
        assert_eq!(s.freq_hz, 14_195_000);
        assert_eq!(s.dx_call, "EA8XYZ");
        assert_eq!(s.time_utc, "2030Z");
        assert!(s.comment.contains("Canary Is"));
    }

    #[test]
    fn parses_cc_cluster_ft8_line_with_portable_call() {
        let s = parse_spot_line(
            "DX de OH2XYZ:    3573.0  W1AW/4       FT8 -12 dB              2201Z",
            t0(),
        )
        .expect("should parse");
        assert_eq!(s.freq_hz, 3_573_000);
        assert_eq!(s.dx_call, "W1AW/4"); // portable slash preserved
        assert_eq!(s.band, Some(HamBand::B80));
        assert_eq!(s.mode_hint.as_deref(), Some("FT8"));
    }

    #[test]
    fn out_of_band_freq_is_valid_but_bandless() {
        let s = parse_spot_line("DX de K7ABC:  144200.0  W5XYZ  SSB  1200Z", t0())
            .expect("should parse");
        assert_eq!(s.freq_hz, 144_200_000);
        assert_eq!(s.band, None); // 2m not in the HF band table
        assert_eq!(s.mode_hint.as_deref(), Some("SSB"));
    }

    #[test]
    fn missing_time_token_is_ok() {
        let s = parse_spot_line("DX de K1ABC:  7005.0  DL1XYZ  CW", t0()).expect("parse");
        assert_eq!(s.dx_call, "DL1XYZ");
        assert_eq!(s.time_utc, "");
        assert_eq!(s.comment, "CW");
    }

    #[test]
    fn drops_non_spot_lines() {
        for junk in [
            "WWV de VE7CC <18>:   SFI=140, A=7, K=2 -> No storms",
            "To ALL de W3LPL: happy new year",
            "Please enter your call:",
            "",
            "   ",
            "login: ",
            "DX de W3LPL:", // no freq/call
            "DX de W3LPL:  notafreq  JA1XYZ  CW  1432Z",
        ] {
            assert!(
                parse_spot_line(junk, t0()).is_none(),
                "should have dropped: {junk:?}"
            );
        }
    }

    #[test]
    fn dedupe_same_call_within_tolerance_keeps_newest() {
        let base = t0();
        let older = base.checked_sub(Duration::from_secs(60)).unwrap();
        let mut spots = Vec::new();
        insert_spot(
            &mut spots,
            parse_spot_line("DX de A:  14025.0  JA1XYZ  CW  1000Z", older).unwrap(),
        );
        // Same call, 0.1 kHz away → collapses onto the newer one.
        insert_spot(
            &mut spots,
            parse_spot_line("DX de B:  14025.1  JA1XYZ  CW  1001Z", base).unwrap(),
        );
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].spotter, "B");
        assert_eq!(spots[0].received_at, base);
    }

    #[test]
    fn dedupe_keeps_distinct_call_or_far_freq() {
        let now = t0();
        let mut spots = Vec::new();
        insert_spot(
            &mut spots,
            parse_spot_line("DX de A:  14025.0  JA1XYZ  CW  1000Z", now).unwrap(),
        );
        // Different call, same freq → kept.
        insert_spot(
            &mut spots,
            parse_spot_line("DX de A:  14025.0  DL1XYZ  CW  1000Z", now).unwrap(),
        );
        // Same call, 1 kHz away (> tolerance) → kept.
        insert_spot(
            &mut spots,
            parse_spot_line("DX de A:  14026.0  JA1XYZ  CW  1000Z", now).unwrap(),
        );
        assert_eq!(spots.len(), 3);
    }

    #[test]
    fn expire_drops_old_keeps_fresh() {
        let now = t0();
        let mut spots = Vec::new();
        insert_spot(
            &mut spots,
            parse_spot_line("DX de A:  14025.0  FRESH  CW  1000Z", now).unwrap(),
        );
        insert_spot(
            &mut spots,
            parse_spot_line(
                "DX de A:  14030.0  STALE  CW  0900Z",
                now.checked_sub(Duration::from_secs(40 * 60)).unwrap(),
            )
            .unwrap(),
        );
        expire(&mut spots, now, DEFAULT_TTL);
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].dx_call, "FRESH");
    }

    #[test]
    fn max_spots_cap_drops_oldest() {
        let base = t0();
        let mut spots = Vec::new();
        // Fill past the cap; each spot a distinct call, increasing timestamp.
        for i in 0..(MAX_SPOTS + 5) {
            let ts = base
                .checked_sub(Duration::from_secs((MAX_SPOTS + 5 - i) as u64))
                .unwrap();
            let spot = DxSpot {
                dx_call: format!("CALL{i}"),
                freq_hz: 14_000_000 + i as u64 * 1000,
                spotter: "X".into(),
                comment: String::new(),
                time_utc: String::new(),
                mode_hint: None,
                band: Some(HamBand::B20),
                received_at: ts,
            };
            insert_spot(&mut spots, spot);
        }
        assert_eq!(spots.len(), MAX_SPOTS);
        // The five oldest (CALL0..CALL4) should have been evicted.
        assert!(!spots.iter().any(|s| s.dx_call == "CALL0"));
        assert!(
            spots
                .iter()
                .any(|s| s.dx_call == format!("CALL{}", MAX_SPOTS + 4))
        );
    }
}
