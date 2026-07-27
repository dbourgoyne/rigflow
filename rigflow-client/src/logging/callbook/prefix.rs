//! Offline prefix baseline — derive DXCC entity + CQ/ITU zone from a callsign's
//! prefix, with no network. The always-on instant floor under the online
//! providers (design §5.1).
//!
//! **v1 uses a curated starter table**, matched longest-prefix-first with basic
//! portable-call handling. It covers the common entities but is deliberately
//! incomplete; the completion is to load the full `cty.dat` (AD1C country file)
//! that N1MM/DXLab use — a self-contained follow-up that keeps this same
//! `resolve()` interface. Online providers return the same fields, so an
//! incomplete table only limits *offline* coverage, never a provider lookup.

use super::{CallbookResult, Source};

/// (prefix, dxcc, country, cq_zone, itu_zone). Longest matching prefix wins, so
/// more-specific entries (KH6, KL, KP4) must be able to beat the generic (K/W/N).
/// US call areas don't change the entity, so a single K/W/N/A entry suffices for
/// the contiguous US; the offshore US prefixes are listed separately.
const TABLE: &[(&str, i64, &str, u8, u8)] = &[
    // United States (contiguous) and offshore
    ("KH6", 110, "Hawaii", 31, 61),
    ("KH7", 110, "Hawaii", 31, 61),
    ("KL", 6, "Alaska", 1, 1),
    ("KP4", 202, "Puerto Rico", 8, 11),
    ("KP2", 285, "US Virgin Islands", 8, 11),
    ("K", 291, "United States", 5, 8),
    ("W", 291, "United States", 5, 8),
    ("N", 291, "United States", 5, 8),
    ("AA", 291, "United States", 5, 8),
    ("AK", 291, "United States", 5, 8),
    ("AL", 291, "United States", 5, 8),
    // Canada
    ("VE", 1, "Canada", 5, 9),
    ("VA", 1, "Canada", 5, 9),
    ("VO", 1, "Canada", 5, 9),
    ("VY", 1, "Canada", 5, 9),
    // England / UK family (broad; sub-entities left to cty.dat)
    ("G", 223, "England", 14, 27),
    ("M", 223, "England", 14, 27),
    ("2E", 223, "England", 14, 27),
    // Germany
    ("DL", 230, "Germany", 14, 28),
    ("DA", 230, "Germany", 14, 28),
    ("DK", 230, "Germany", 14, 28),
    ("DD", 230, "Germany", 14, 28),
    // Japan
    ("JA", 339, "Japan", 25, 45),
    ("JR", 339, "Japan", 25, 45),
    ("JH", 339, "Japan", 25, 45),
    ("7K", 339, "Japan", 25, 45),
    // Oceania
    ("VK", 150, "Australia", 30, 59),
    ("ZL", 170, "New Zealand", 32, 60),
    ("FK", 162, "New Caledonia", 32, 56),
    // South / Central America (entities seen in the log)
    ("PY", 108, "Brazil", 11, 15),
    ("PP", 108, "Brazil", 11, 15),
    ("PT", 108, "Brazil", 11, 15),
    ("HK", 116, "Colombia", 9, 12),
    ("HP", 88, "Panama", 7, 11),
    ("3E", 88, "Panama", 7, 11),
];

/// Resolve entity + zones from `call`, offline. `None` if the prefix isn't in the
/// v1 table (an online provider may still know it).
pub fn resolve(call: &str) -> Option<CallbookResult> {
    let up = call.trim().to_ascii_uppercase();
    if up.is_empty() {
        return None;
    }
    // Portable calls: consider each `/`-segment that isn't a plain suffix, and let
    // the longest matched prefix across them win (so W1AW/KH6 → Hawaii, not US).
    let segments: Vec<&str> = up.split('/').filter(|s| !is_suffix(s)).collect();
    let candidates: Vec<&str> = if segments.is_empty() {
        vec![up.as_str()]
    } else {
        segments
    };

    let (_, dxcc, country, cq, itu) = candidates
        .iter()
        .filter_map(|c| match_entity(c))
        .max_by_key(|(plen, ..)| *plen)?;

    Some(CallbookResult {
        country: Some(country.to_string()),
        dxcc: Some(dxcc),
        cq_zone: Some(cq.to_string()),
        itu_zone: Some(itu.to_string()),
        source: Source::Prefix,
        ..Default::default()
    })
}

/// The table entry whose prefix `call` starts with, longest first. Returns the
/// matched prefix length so a caller can pick the best across portable segments.
fn match_entity(call: &str) -> Option<(usize, i64, &'static str, u8, u8)> {
    TABLE
        .iter()
        .filter(|(p, ..)| call.starts_with(p))
        .max_by_key(|(p, ..)| p.len())
        .map(|&(p, dxcc, country, cq, itu)| (p.len(), dxcc, country, cq, itu))
}

/// A `/`-segment that is a portable/status suffix, not a location prefix — these
/// don't change the DXCC entity (a `/`-number changes US call area only).
fn is_suffix(seg: &str) -> bool {
    matches!(seg, "P" | "M" | "MM" | "AM" | "A" | "QRP" | "QRPP")
        || (seg.len() == 1 && seg.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_common_entities() {
        assert_eq!(resolve("W1AW").unwrap().dxcc, Some(291));
        assert_eq!(
            resolve("N5WXB").unwrap().country.as_deref(),
            Some("United States")
        );
        assert_eq!(resolve("3E40CDW").unwrap().dxcc, Some(88)); // Panama
        assert_eq!(
            resolve("PY1USK").unwrap().country.as_deref(),
            Some("Brazil")
        );
        assert_eq!(resolve("VK3ABC").unwrap().dxcc, Some(150));
    }

    #[test]
    fn longest_prefix_and_offshore_us_win() {
        // KH6 must beat the generic K entry.
        assert_eq!(resolve("KH6XYZ").unwrap().dxcc, Some(110)); // Hawaii
        assert_eq!(resolve("KL7ABC").unwrap().dxcc, Some(6)); // Alaska
        assert_eq!(resolve("KP4AA").unwrap().dxcc, Some(202)); // Puerto Rico
    }

    #[test]
    fn portable_uses_the_location_prefix() {
        // Operating from Hawaii → Hawaii, not the home US call.
        assert_eq!(resolve("W1AW/KH6").unwrap().dxcc, Some(110));
        // A plain /P or /M suffix doesn't change the entity.
        assert_eq!(resolve("G3XYZ/P").unwrap().dxcc, Some(223));
        assert_eq!(resolve("W1AW/7").unwrap().dxcc, Some(291));
    }

    #[test]
    fn unknown_prefix_is_none() {
        assert_eq!(resolve("XX9ZZ"), None);
    }
}
