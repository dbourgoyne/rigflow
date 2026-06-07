//! Hardrock-50 (HR50) serial protocol — Phase 1, read-only.
//!
//! Only the two commands Phase 1 needs are used (per the integration spec):
//!
//! - `HRRX;` → `RX,<mode>,<band>,<temp>,<voltage>;`  e.g. `RX,QRP,12M,27C,13.2V;`
//! - `HRVT;` → `HRVTxx.xV;`                          e.g. `HRVT13.2V;`
//!
//! Commands are ASCII, semicolon-terminated; the HR50 always replies in upper
//! case. Temperature in the `HRRX;` reply carries its scale as a trailing letter
//! (`C` or `F`) — the user's amp is set to Fahrenheit, so we **normalize to °C**.
//!
//! These parsers are transport-independent (they take the decoded response
//! string) so they can be unit-tested without hardware.

/// Command to read the HR50 RX status (mode, band, temperature, voltage).
pub const CMD_HRRX: &[u8] = b"HRRX;";
/// Command to read the HR50 DC input voltage.
pub const CMD_HRVT: &[u8] = b"HRVT;";

/// Parsed fields of an `HRRX;` response.  All fields are optional so a partially
/// malformed reply still yields whatever was readable.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hr50Rx {
    pub mode: Option<String>,
    pub band: Option<String>,
    /// Temperature normalized to degrees Celsius.
    pub temperature_c: Option<f32>,
    pub voltage_v: Option<f32>,
}

/// Parse an `HRRX;` response of the form `RX,<mode>,<band>,<temp>,<voltage>;`.
///
/// Returns `None` only when the reply is not a recognizable RX status (no `RX,`
/// marker) — that signals "not an HR50 / invalid data" for detection. A
/// recognizable-but-partial reply returns `Some` with the readable fields.
pub fn parse_hrrx(resp: &str) -> Option<Hr50Rx> {
    // Tolerate echo/noise around the actual reply: find the `RX,` marker and the
    // terminating `;`, and work on the slice between them.
    let start = resp.find("RX,")?;
    let rest = &resp[start + 3..];
    let body = match rest.find(';') {
        Some(end) => &rest[..end],
        None => rest, // tolerate a missing terminator
    };

    let mut fields = body.split(',').map(str::trim);
    let mode = fields.next().filter(|s| !s.is_empty()).map(str::to_string);
    let band = fields.next().filter(|s| !s.is_empty()).map(str::to_string);
    let temperature_c = fields.next().and_then(parse_temperature_c);
    let voltage_v = fields.next().and_then(parse_voltage);

    Some(Hr50Rx {
        mode,
        band,
        temperature_c,
        voltage_v,
    })
}

/// Parse an `HRVT;` response of the form `HRVTxx.xV;` → volts.
pub fn parse_hrvt(resp: &str) -> Option<f32> {
    let start = resp.find("HRVT")?;
    let rest = &resp[start + 4..];
    let end = rest.find(';').unwrap_or(rest.len());
    parse_voltage(&rest[..end])
}

/// Parse a temperature token like `27C` or `80F`, returning **degrees Celsius**.
/// A bare number with no scale suffix is treated as Celsius.
fn parse_temperature_c(tok: &str) -> Option<f32> {
    let tok = tok.trim();
    let (num, fahrenheit) = match tok.chars().last() {
        Some('C') | Some('c') => (&tok[..tok.len() - 1], false),
        Some('F') | Some('f') => (&tok[..tok.len() - 1], true),
        _ => (tok, false),
    };
    let value: f32 = num.trim().parse().ok()?;
    Some(if fahrenheit {
        (value - 32.0) * 5.0 / 9.0
    } else {
        value
    })
}

/// Parse a voltage token like `13.2V` (trailing `V` optional) → volts.
fn parse_voltage(tok: &str) -> Option<f32> {
    let tok = tok.trim();
    let num = tok.strip_suffix(['V', 'v']).unwrap_or(tok);
    num.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrrx_celsius_example() {
        let rx = parse_hrrx("RX,QRP,12M,27C,13.2V;").unwrap();
        assert_eq!(rx.mode.as_deref(), Some("QRP"));
        assert_eq!(rx.band.as_deref(), Some("12M"));
        assert!((rx.temperature_c.unwrap() - 27.0).abs() < 0.01);
        assert!((rx.voltage_v.unwrap() - 13.2).abs() < 0.01);
    }

    #[test]
    fn hrrx_fahrenheit_normalized_to_celsius() {
        // 80F == 26.666…C
        let rx = parse_hrrx("RX,PTT,20M,80F,13.8V;").unwrap();
        assert_eq!(rx.mode.as_deref(), Some("PTT"));
        assert_eq!(rx.band.as_deref(), Some("20M"));
        assert!((rx.temperature_c.unwrap() - 26.667).abs() < 0.01);
        assert!((rx.voltage_v.unwrap() - 13.8).abs() < 0.01);
    }

    #[test]
    fn hrrx_tolerates_echo_and_whitespace() {
        // Leading echo of the command + trailing junk, plus spaces.
        let rx = parse_hrrx("HRRX; RX, COR , 40M , 31C , 12.0V ;\r\n").unwrap();
        assert_eq!(rx.mode.as_deref(), Some("COR"));
        assert_eq!(rx.band.as_deref(), Some("40M"));
        assert!((rx.temperature_c.unwrap() - 31.0).abs() < 0.01);
        assert!((rx.voltage_v.unwrap() - 12.0).abs() < 0.01);
    }

    #[test]
    fn hrrx_missing_marker_is_none() {
        assert!(parse_hrrx("HRVT13.2V;").is_none());
        assert!(parse_hrrx("garbage").is_none());
        assert!(parse_hrrx("").is_none());
    }

    #[test]
    fn hrrx_partial_reply_returns_readable_fields() {
        // Only mode + band present; temp/voltage absent.
        let rx = parse_hrrx("RX,OFF,;").unwrap();
        assert_eq!(rx.mode.as_deref(), Some("OFF"));
        assert_eq!(rx.band, None);
        assert_eq!(rx.temperature_c, None);
        assert_eq!(rx.voltage_v, None);
    }

    #[test]
    fn hrvt_example() {
        assert!((parse_hrvt("HRVT13.2V;").unwrap() - 13.2).abs() < 0.01);
    }

    #[test]
    fn hrvt_tolerates_echo_and_no_terminator() {
        assert!((parse_hrvt("HRVT11.9V").unwrap() - 11.9).abs() < 0.01);
        assert!((parse_hrvt("noise HRVT 14.1V ;").unwrap() - 14.1).abs() < 0.01);
    }

    #[test]
    fn hrvt_malformed_is_none() {
        assert!(parse_hrvt("RX,QRP,12M,27C,13.2V;").is_none());
        assert!(parse_hrvt("HRVT;").is_none());
        assert!(parse_hrvt("").is_none());
    }
}
