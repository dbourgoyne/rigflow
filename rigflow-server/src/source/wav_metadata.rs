use std::path::Path;

/// Parse center frequency from a WAV filename.
///
/// Supports patterns like:
/// - SDRuno_20200912_004142Z_3750kHz.wav
/// - capture_3.75MHz_iq.wav
/// - test_3750000Hz.wav
///
/// Returns frequency in Hz.
pub fn parse_center_freq_hz_from_filename(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    parse_center_freq_hz_from_str(name)
}

/// Extracts a frequency from a string by scanning for:
///   <number><unit>
/// where unit ∈ {mhz, khz, hz}.
///
/// Matching is:
/// - case-insensitive
/// - first match wins, based on unit search order (MHz → kHz → Hz)
pub fn parse_center_freq_hz_from_str(s: &str) -> Option<u64> {
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();

    // Order matters: prefer larger units first
    const UNITS: [(&str, f64); 3] = [("mhz", 1_000_000.0), ("khz", 1_000.0), ("hz", 1.0)];

    for (unit, scale) in UNITS {
        let mut search_start = 0;

        while let Some(rel_pos) = lower[search_start..].find(unit) {
            let unit_pos = search_start + rel_pos;

            // Walk backward to capture the numeric token preceding the unit.
            let mut start = unit_pos;

            while start > 0 {
                let c = bytes[start - 1] as char;
                if c.is_ascii_digit() || c == '.' {
                    start -= 1;
                } else {
                    break;
                }
            }

            // No numeric prefix → skip
            if start == unit_pos {
                search_start = unit_pos + unit.len();
                continue;
            }

            let num_str = &lower[start..unit_pos];

            // Reject malformed cases like "."
            if num_str == "." {
                search_start = unit_pos + unit.len();
                continue;
            }

            if let Ok(value) = num_str.parse::<f64>() {
                let hz = (value * scale).round();

                // Ensure finite and non-negative
                if hz.is_finite() && hz >= 0.0 {
                    return Some(hz as u64);
                }
            }

            search_start = unit_pos + unit.len();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_khz() {
        assert_eq!(
            parse_center_freq_hz_from_str("SDRuno_20200912_004142Z_3750kHz.wav"),
            Some(3_750_000)
        );
    }

    #[test]
    fn parses_mhz() {
        assert_eq!(
            parse_center_freq_hz_from_str("capture_3.75MHz_iq.wav"),
            Some(3_750_000)
        );
    }

    #[test]
    fn parses_hz() {
        assert_eq!(
            parse_center_freq_hz_from_str("test_3750000Hz.wav"),
            Some(3_750_000)
        );
    }

    #[test]
    fn parses_decimal_khz() {
        assert_eq!(
            parse_center_freq_hz_from_str("foo_3750.5kHz.wav"),
            Some(3_750_500)
        );
    }

    #[test]
    fn returns_none_when_missing() {
        assert_eq!(
            parse_center_freq_hz_from_str("iq_capture_no_freq.wav"),
            None
        );
    }

    #[test]
    fn prefers_first_match_by_unit_search_order() {
        assert_eq!(
            parse_center_freq_hz_from_str("capture_3.75MHz_3750kHz.wav"),
            Some(3_750_000)
        );
    }
}
