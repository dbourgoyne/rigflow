//! Centralized mode/band normalization.
//!
//! Two independent jobs:
//! - [`band_for_freq_hz`] maps a frequency to its ADIF `BAND` string. TX and RX
//!   frequencies are mapped **independently** (a split QSO can straddle two
//!   bands, and 60m is a channelized edge case), so the caller derives `band`
//!   from `freq_hz` and `band_rx` from `freq_rx_hz` with two separate calls.
//! - [`normalize_mode`] maps a (possibly non-compliant) mode/submode pair to a
//!   canonical ADIF `MODE` plus an optional `SUBMODE`, so downstream matching
//!   (confirmation sync in a later phase) has a stable vocabulary.

/// ADIF band table: `(low_hz, high_hz_inclusive, adif_band)`, ordered low→high.
/// Ranges are the ADIF `BAND` enumeration bounds. Endpoints are inclusive so a
/// dial sitting exactly on a band edge still resolves.
const BANDS: &[(u64, u64, &str)] = &[
    (135_700, 137_800, "2190m"),
    (472_000, 479_000, "630m"),
    (1_800_000, 2_000_000, "160m"),
    (3_500_000, 4_000_000, "80m"),
    // 60m: ADIF treats the whole 5 MHz allocation as "60m"; US is channelized
    // (5330.5–5405 kHz) but other regions have a contiguous band. Use the wider
    // region-2/1 bounds so channelized US dials still resolve.
    (5_060_000, 5_450_000, "60m"),
    (7_000_000, 7_300_000, "40m"),
    (10_100_000, 10_150_000, "30m"),
    (14_000_000, 14_350_000, "20m"),
    (18_068_000, 18_168_000, "17m"),
    (21_000_000, 21_450_000, "15m"),
    (24_890_000, 24_990_000, "12m"),
    (28_000_000, 29_700_000, "10m"),
    (50_000_000, 54_000_000, "6m"),
    (70_000_000, 71_000_000, "4m"),
    (144_000_000, 148_000_000, "2m"),
    (222_000_000, 225_000_000, "1.25m"),
    (420_000_000, 450_000_000, "70cm"),
    (902_000_000, 928_000_000, "33cm"),
    (1_240_000_000, 1_300_000_000, "23cm"),
];

/// Returns the ADIF `BAND` string a frequency falls in, or `None` if the
/// frequency is outside every ham band (e.g. a WAV recording tuned to a
/// broadcast station). `None` means "leave BAND unset", never a guess.
pub fn band_for_freq_hz(freq_hz: u64) -> Option<&'static str> {
    BANDS
        .iter()
        .find(|(lo, hi, _)| freq_hz >= *lo && freq_hz <= *hi)
        .map(|(_, _, band)| *band)
}

/// Every ADIF `BAND` string we recognize, low→high. The band enumeration is
/// closed, so export can reject an unknown band outright rather than silently
/// matching nothing.
pub fn known_bands() -> impl Iterator<Item = &'static str> {
    BANDS.iter().map(|(_, _, b)| *b)
}

/// Whether `band` is a recognized ADIF `BAND` (case-insensitive).
pub fn is_known_band(band: &str) -> bool {
    let b = band.trim().to_ascii_lowercase();
    known_bands().any(|k| k.eq_ignore_ascii_case(&b))
}

/// Known submode → canonical parent `MODE`. When a submode value shows up in
/// the `MODE` field (a common non-compliance), we split it back into
/// `(parent_mode, Some(submode))`.
const SUBMODE_PARENT: &[(&str, &str)] = &[
    ("PSK31", "PSK"),
    ("PSK63", "PSK"),
    ("PSK125", "PSK"),
    ("QPSK31", "PSK"),
    ("QPSK63", "PSK"),
    ("FT4", "MFSK"),
    ("JS8", "MFSK"),
    ("JT65A", "JT65"),
    ("JT65B", "JT65"),
    ("JT65C", "JT65"),
];

/// Aliases for the `MODE` field itself (not submodes): things operators or
/// other software write that aren't the ADIF-canonical mode. USB/LSB are CAT
/// sidebands, not ADIF modes — both normalize to `SSB`.
const MODE_ALIAS: &[(&str, &str)] = &[
    ("USB", "SSB"),
    ("LSB", "SSB"),
    ("CWU", "CW"),
    ("CWL", "CW"),
    ("PKTUSB", "MFSK"),
    ("DIG", "MFSK"),
    ("DATA", "MFSK"),
];

/// Normalize a raw `(mode, submode)` pair to canonical ADIF `(MODE, SUBMODE)`.
///
/// Rules, in order:
/// 1. Upper-case and trim both.
/// 2. If a submode is supplied, keep it and, if we know its parent, prefer that
///    parent as the mode.
/// 3. Else if the mode value is itself a known submode, split it into
///    `(parent, Some(submode))`.
/// 4. Else map any mode alias (USB/LSB→SSB, …).
///
/// An unknown mode is passed through upper-cased rather than dropped.
pub fn normalize_mode(mode: &str, submode: Option<&str>) -> (String, Option<String>) {
    let mode_uc = mode.trim().to_ascii_uppercase();
    let submode_uc = submode
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty());

    if let Some(sm) = submode_uc {
        // An explicit submode wins; resolve its canonical parent if known,
        // otherwise trust the supplied mode.
        let parent = parent_of_submode(&sm)
            .map(str::to_string)
            .unwrap_or_else(|| alias_mode(&mode_uc));
        return (parent, Some(sm));
    }

    // No submode: the mode field itself might actually be a submode value.
    if let Some(parent) = parent_of_submode(&mode_uc) {
        return (parent.to_string(), Some(mode_uc));
    }

    (alias_mode(&mode_uc), None)
}

/// A coarse mode family, for filters that mean "all the digital contacts" rather
/// than an exact mode list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModeClass {
    Phone,
    Cw,
    Digital,
    Image,
}

impl ModeClass {
    /// Every class, for UI enumeration.
    pub const ALL: &'static [ModeClass] = &[
        ModeClass::Phone,
        ModeClass::Cw,
        ModeClass::Digital,
        ModeClass::Image,
    ];

    /// Stable lower-case wire/UI name.
    pub fn as_str(self) -> &'static str {
        match self {
            ModeClass::Phone => "phone",
            ModeClass::Cw => "cw",
            ModeClass::Digital => "digital",
            ModeClass::Image => "image",
        }
    }

    /// Parse a class name (case-insensitive). `None` if unknown.
    pub fn parse(s: &str) -> Option<ModeClass> {
        let s = s.trim().to_ascii_lowercase();
        ModeClass::ALL.iter().copied().find(|c| c.as_str() == s)
    }
}

/// Canonical `MODE` → class. Keyed on the **canonical parent mode** (what the
/// `mode` column actually stores after [`normalize_mode`]), never on a submode:
/// FT4 is stored as `MFSK`/`FT4`, so classifying `MFSK` classifies FT4 with it.
///
/// This table is the single source of truth for the mode→class mapping. Export's
/// `mode_classes` filter expands through [`modes_in_class`] rather than
/// hardcoding a list at the call site, so a mode added to [`MODE_ALIAS`] or
/// [`SUBMODE_PARENT`] only has to be classified once, here.
const MODE_CLASS: &[(&str, ModeClass)] = &[
    // Phone
    ("SSB", ModeClass::Phone),
    ("AM", ModeClass::Phone),
    ("FM", ModeClass::Phone),
    ("DIGITALVOICE", ModeClass::Phone),
    // CW
    ("CW", ModeClass::Cw),
    // Digital
    ("FT8", ModeClass::Digital),
    ("MFSK", ModeClass::Digital), // FT4, JS8
    ("JT65", ModeClass::Digital),
    ("JT9", ModeClass::Digital),
    ("JT4", ModeClass::Digital),
    ("Q65", ModeClass::Digital),
    ("FSK441", ModeClass::Digital),
    ("ISCAT", ModeClass::Digital),
    ("MSK144", ModeClass::Digital),
    ("PSK", ModeClass::Digital),
    ("RTTY", ModeClass::Digital),
    ("OLIVIA", ModeClass::Digital),
    ("CONTESTIA", ModeClass::Digital),
    ("DOMINO", ModeClass::Digital),
    ("THOR", ModeClass::Digital),
    ("THRB", ModeClass::Digital),
    ("MT63", ModeClass::Digital),
    ("HELL", ModeClass::Digital),
    ("ARDOP", ModeClass::Digital),
    ("PACKET", ModeClass::Digital),
    ("PACTOR", ModeClass::Digital),
    ("VARA", ModeClass::Digital),
    ("WINMOR", ModeClass::Digital),
    // Image
    ("SSTV", ModeClass::Image),
    ("ATV", ModeClass::Image),
    ("FAX", ModeClass::Image),
];

/// The class of a canonical `MODE`, or `None` for a mode we don't classify (an
/// unknown mode passed through by [`normalize_mode`]). `None` is "unclassified",
/// never a guess — an unclassified mode simply doesn't match any class filter.
pub fn mode_class(mode: &str) -> Option<ModeClass> {
    let m = mode.trim().to_ascii_uppercase();
    MODE_CLASS.iter().find(|(k, _)| *k == m).map(|(_, c)| *c)
}

/// Every canonical `MODE` in a class. Export expands a `mode_classes` filter
/// into an `IN (...)` over these, so the SQL only ever compares the indexed
/// `mode` column against exact values.
pub fn modes_in_class(class: ModeClass) -> Vec<&'static str> {
    MODE_CLASS
        .iter()
        .filter(|(_, c)| *c == class)
        .map(|(m, _)| *m)
        .collect()
}

fn parent_of_submode(sm: &str) -> Option<&'static str> {
    SUBMODE_PARENT
        .iter()
        .find(|(k, _)| *k == sm)
        .map(|(_, v)| *v)
}

fn alias_mode(mode_uc: &str) -> String {
    MODE_ALIAS
        .iter()
        .find(|(k, _)| *k == mode_uc)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| mode_uc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_from_freq_spread() {
        assert_eq!(band_for_freq_hz(1_840_000), Some("160m"));
        assert_eq!(band_for_freq_hz(3_573_000), Some("80m"));
        assert_eq!(band_for_freq_hz(7_074_000), Some("40m"));
        assert_eq!(band_for_freq_hz(10_136_000), Some("30m"));
        assert_eq!(band_for_freq_hz(14_074_000), Some("20m"));
        assert_eq!(band_for_freq_hz(14_200_000), Some("20m"));
        assert_eq!(band_for_freq_hz(18_100_000), Some("17m"));
        assert_eq!(band_for_freq_hz(21_074_000), Some("15m"));
        assert_eq!(band_for_freq_hz(28_074_000), Some("10m"));
        assert_eq!(band_for_freq_hz(50_313_000), Some("6m"));
        assert_eq!(band_for_freq_hz(144_174_000), Some("2m"));
        assert_eq!(band_for_freq_hz(432_100_000), Some("70cm"));
    }

    #[test]
    fn band_edges_inclusive() {
        assert_eq!(band_for_freq_hz(14_000_000), Some("20m"));
        assert_eq!(band_for_freq_hz(14_350_000), Some("20m"));
    }

    #[test]
    fn band_60m_channelized_us() {
        // US 60m channel 1 center — must still resolve to 60m.
        assert_eq!(band_for_freq_hz(5_332_000), Some("60m"));
    }

    #[test]
    fn band_out_of_range_is_none() {
        assert_eq!(band_for_freq_hz(9_000_000), None); // between 40m and 30m
        assert_eq!(band_for_freq_hz(1_000_000), None); // MW broadcast
    }

    #[test]
    fn band_tx_and_rx_independent() {
        // A split QSO can straddle a band edge; each side maps on its own.
        assert_eq!(band_for_freq_hz(7_200_000), Some("40m"));
        assert_eq!(band_for_freq_hz(7_290_000), Some("40m"));
    }

    #[test]
    fn mode_ssb_from_sideband() {
        assert_eq!(normalize_mode("USB", None), ("SSB".into(), None));
        assert_eq!(normalize_mode("lsb", None), ("SSB".into(), None));
        assert_eq!(normalize_mode("SSB", None), ("SSB".into(), None));
    }

    #[test]
    fn mode_cw() {
        assert_eq!(normalize_mode("CW", None), ("CW".into(), None));
        assert_eq!(normalize_mode("CWU", None), ("CW".into(), None));
    }

    #[test]
    fn mode_ft8_is_toplevel() {
        assert_eq!(normalize_mode("FT8", None), ("FT8".into(), None));
    }

    #[test]
    fn mode_submode_in_mode_field_is_split() {
        // "PSK31" logged as the MODE → (PSK, PSK31).
        assert_eq!(
            normalize_mode("PSK31", None),
            ("PSK".into(), Some("PSK31".into()))
        );
        // FT4 is MFSK/FT4 in ADIF.
        assert_eq!(
            normalize_mode("FT4", None),
            ("MFSK".into(), Some("FT4".into()))
        );
    }

    #[test]
    fn mode_explicit_submode_resolves_parent() {
        assert_eq!(
            normalize_mode("PSK", Some("PSK63")),
            ("PSK".into(), Some("PSK63".into()))
        );
        // Unknown submode: trust the supplied (aliased) mode.
        assert_eq!(
            normalize_mode("USB", Some("SOMETHING")),
            ("SSB".into(), Some("SOMETHING".into()))
        );
    }

    #[test]
    fn mode_unknown_passes_through_uppercased() {
        assert_eq!(
            normalize_mode("contestia", None),
            ("CONTESTIA".into(), None)
        );
    }

    #[test]
    fn mode_class_basics() {
        assert_eq!(mode_class("SSB"), Some(ModeClass::Phone));
        assert_eq!(mode_class("cw"), Some(ModeClass::Cw));
        assert_eq!(mode_class("FT8"), Some(ModeClass::Digital));
        assert_eq!(mode_class("SSTV"), Some(ModeClass::Image));
    }

    #[test]
    fn mode_class_follows_normalization() {
        // The class table is keyed on the CANONICAL mode, so classifying has to
        // go through normalize_mode first — USB is not in the table, SSB is.
        let (mode, _) = normalize_mode("USB", None);
        assert_eq!(mode_class(&mode), Some(ModeClass::Phone));

        // FT4 normalizes to MFSK/FT4; classifying the stored parent gets digital.
        let (mode, submode) = normalize_mode("FT4", None);
        assert_eq!(submode.as_deref(), Some("FT4"));
        assert_eq!(mode_class(&mode), Some(ModeClass::Digital));
    }

    #[test]
    fn mode_class_unknown_is_none() {
        assert_eq!(mode_class("CONTESTIA"), Some(ModeClass::Digital));
        assert_eq!(mode_class("NOTAMODE"), None);
    }

    #[test]
    fn olivia_is_a_mode_not_its_own_submode() {
        // Regression: OLIVIA used to self-parent in SUBMODE_PARENT, stamping a
        // bogus <SUBMODE:6>OLIVIA on every Olivia QSO. MODE=OLIVIA is canonical;
        // real Olivia submodes look like "OLIVIA 8/250".
        assert_eq!(normalize_mode("OLIVIA", None), ("OLIVIA".into(), None));
    }

    #[test]
    fn modes_in_class_expands() {
        let phone = modes_in_class(ModeClass::Phone);
        assert!(phone.contains(&"SSB"));
        assert!(phone.contains(&"AM"));
        assert!(phone.contains(&"FM"));
        assert!(!phone.contains(&"CW"));

        // Digital must include MFSK, or FT4/JS8 QSOs silently fall out of a
        // "digital" export.
        let digital = modes_in_class(ModeClass::Digital);
        assert!(digital.contains(&"FT8"));
        assert!(digital.contains(&"MFSK"));
        assert!(digital.contains(&"RTTY"));

        assert_eq!(modes_in_class(ModeClass::Cw), vec!["CW"]);
    }

    #[test]
    fn mode_class_roundtrips_by_name() {
        for c in ModeClass::ALL {
            assert_eq!(ModeClass::parse(c.as_str()), Some(*c));
        }
        assert_eq!(ModeClass::parse("PHONE"), Some(ModeClass::Phone));
        assert_eq!(ModeClass::parse("nope"), None);
    }

    #[test]
    fn every_classified_mode_is_canonical() {
        // A class-table key that isn't its own canonical mode would never match
        // the stored `mode` column. Guards against adding e.g. "USB" or "FT4".
        for (m, _) in MODE_CLASS {
            let (canonical, submode) = normalize_mode(m, None);
            assert_eq!(
                (&canonical[..], submode.as_deref()),
                (*m, None),
                "MODE_CLASS key {m} is not a canonical MODE"
            );
        }
    }
}
