//! Hardrock-50 (HR50) serial protocol.
//!
//! Phase 1 (read-only): `HRRX;` (mode/band/temp/voltage), `HRVT;` (voltage).
//! Phase 2 adds frequency tracking + control + TX/ATU telemetry:
//! - `FAxxxxxxxxxxx;` — set VFO freq so the amp picks band/segment (fire-and-forget).
//! - `HRMDx;`         — keying mode 0=OFF 1=PTT 2=COR 3=QRP.
//! - `HRATx;`         — ATU 0=not present, 1=bypass, 2=active (SET/GET).
//! - `HRTU1;`         — tune on the next TX.
//! - `HRMX;` → `HRMX P40 A25 S12 T28C` — last TX PEP/avg/SWR/temp (SWR = S/10; S00 = too low).
//!
//! Commands are ASCII, semicolon-terminated; the HR50 always replies in upper
//! case. Temperature carries its scale as a trailing `C`/`F`, normalized to °C.
//!
//! These parsers are transport-independent (they take the decoded response
//! string) so they can be unit-tested without hardware.

use rigflow_core::radio::amplifier::AmplifierKeyingMode;

/// Command to read the HR50 RX status (mode, band, temperature, voltage).
pub const CMD_HRRX: &[u8] = b"HRRX;";
/// Command to read the HR50 DC input voltage.
pub const CMD_HRVT: &[u8] = b"HRVT;";
/// Command to read the last-transmission PEP/avg/SWR/temp.
pub const CMD_HRMX: &[u8] = b"HRMX;";
/// Command to read the ATU mode/presence (`HRATx;` GET).
pub const CMD_HRAT: &[u8] = b"HRAT;";

/// Build a `FAxxxxxxxxxxx;` frequency command (11-digit Hz).
pub fn cmd_fa(hz: u64) -> Vec<u8> {
    format!("FA{hz:011};").into_bytes()
}

/// Build a `HRMDx;` keying-mode SET command.
pub fn cmd_hrmd(mode: AmplifierKeyingMode) -> Vec<u8> {
    format!("HRMD{};", mode.hr50_code()).into_bytes()
}

/// Build a `HRATx;` ATU-mode SET command (`code` from `AmplifierAtuMode::hr50_code`).
pub fn cmd_hrat(code: u8) -> Vec<u8> {
    format!("HRAT{code};").into_bytes()
}

/// Build the `HRTU1;` "tune on next TX" command.
pub fn cmd_hrtu() -> Vec<u8> {
    b"HRTU1;".to_vec()
}

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

/// Parsed fields of an `HRMX;` response (last transmission).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hr50Mx {
    /// Peak envelope power, watts.
    pub pep_w: Option<f32>,
    /// Average forward power, watts.
    pub avg_w: Option<f32>,
    /// SWR; `None` when the amp reported `S00` (power too low to measure).
    pub swr: Option<f32>,
}

/// Parse `HRMX P40 A25 S12 T28C` → PEP 40 W, avg 25 W, SWR 1.2.  Tokens are
/// space-separated and order-independent; `S00` means SWR was unmeasurable.
pub fn parse_hrmx(resp: &str) -> Option<Hr50Mx> {
    let start = resp.find("HRMX")?;
    let rest = &resp[start + 4..];
    let body = match rest.find(';') {
        Some(end) => &rest[..end],
        None => rest,
    };

    let mut mx = Hr50Mx::default();
    for tok in body.split_whitespace() {
        let (tag, val) = tok.split_at(1);
        match tag {
            "P" | "p" => mx.pep_w = val.trim().parse().ok(),
            "A" | "a" => mx.avg_w = val.trim().parse().ok(),
            "S" | "s" => {
                // SWR is reported ×10 (S12 = 1.2); S00 = too low to measure.
                mx.swr = val.trim().parse::<f32>().ok().and_then(|s| {
                    if s <= 0.0 {
                        None
                    } else {
                        Some(s / 10.0)
                    }
                });
            }
            _ => {} // T<temp> etc. ignored — temperature comes from HRRX.
        }
    }
    Some(mx)
}

/// Parse an `HRATx;` response → ATU code (0 = not present, 1 = bypass, 2 = active).
pub fn parse_hrat(resp: &str) -> Option<u8> {
    let start = resp.find("HRAT")?;
    let rest = &resp[start + 4..];
    let end = rest.find(';').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
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

    #[test]
    fn hrmx_example() {
        let mx = parse_hrmx("HRMX P40 A25 S12 T28C").unwrap();
        assert!((mx.pep_w.unwrap() - 40.0).abs() < 0.01);
        assert!((mx.avg_w.unwrap() - 25.0).abs() < 0.01);
        assert!((mx.swr.unwrap() - 1.2).abs() < 0.01);
    }

    #[test]
    fn hrmx_low_power_swr_is_none() {
        // S00 = not enough power to measure SWR.
        let mx = parse_hrmx("HRMX P02 A01 S00 T30C;").unwrap();
        assert!((mx.pep_w.unwrap() - 2.0).abs() < 0.01);
        assert_eq!(mx.swr, None);
    }

    #[test]
    fn hrmx_malformed_is_none() {
        assert!(parse_hrmx("RX,QRP,12M,27C,13.2V;").is_none());
        assert!(parse_hrmx("").is_none());
    }

    #[test]
    fn hrat_codes() {
        assert_eq!(parse_hrat("HRAT0;"), Some(0)); // no ATU
        assert_eq!(parse_hrat("HRAT1;"), Some(1)); // bypass
        assert_eq!(parse_hrat("HRAT2;"), Some(2)); // active
        assert_eq!(parse_hrat("noise HRAT 2 ;"), Some(2));
        assert_eq!(parse_hrat("HRVT13.2V;"), None);
    }

    #[test]
    fn command_builders() {
        assert_eq!(cmd_fa(14_074_000), b"FA00014074000;".to_vec());
        assert_eq!(cmd_hrmd(AmplifierKeyingMode::Ptt), b"HRMD1;".to_vec());
        assert_eq!(cmd_hrat(2), b"HRAT2;".to_vec());
        assert_eq!(cmd_hrtu(), b"HRTU1;".to_vec());
    }
}
