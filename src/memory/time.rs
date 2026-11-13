//! Time bounds for `mem query` / `mem export`.
//!
//! One grammar for CLI and MCP: `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM:SSZ`, or
//! `@<unix-seconds>`. Bounds are UTC and half-open: `[since, until)`.

use crate::error::{EgError, Result};
use crate::time::days_from_civil;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimeWindow {
    pub since: Option<i64>,
    pub until: Option<i64>,
}

/// Parse one bound. `command` (`mem query`, `mem export`) and `flag`
/// (`--since`, `--until`) shape the usage error exactly.
pub fn parse_time_bound(raw: &str, command: &str, flag: &str) -> Result<i64> {
    if let Some(digits) = raw.strip_prefix('@') {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(usage(command, flag, raw));
        }
        return digits.parse::<i64>().map_err(|_| usage(command, flag, raw));
    }

    let bytes = raw.as_bytes();
    if bytes.len() == 10 {
        let (year, month, day) = parse_date(bytes).ok_or_else(|| usage(command, flag, raw))?;
        return Ok(days_from_civil(year, month, day) * 86_400);
    }
    if bytes.len() == 20 {
        return parse_rfc3339(bytes).ok_or_else(|| usage(command, flag, raw));
    }
    Err(usage(command, flag, raw))
}

/// Reject an inverted window. Equal bounds are valid and yield an empty window.
pub fn validate_window(window: &TimeWindow, command: &str) -> Result<()> {
    if let (Some(since), Some(until)) = (window.since, window.until) {
        if since > until {
            return Err(EgError::Usage(format!(
                "{command}: --since must not be after --until"
            )));
        }
    }
    Ok(())
}

fn usage(command: &str, flag: &str, raw: &str) -> EgError {
    EgError::Usage(format!(
        "{command}: {flag} expects YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @<unix-seconds> (got '{raw}')"
    ))
}

fn parse_date(bytes: &[u8]) -> Option<(i64, u32, u32)> {
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = parse_digits(&bytes[0..4])?;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let month = month as u32;
    if day == 0 || day > i64::from(days_in_month(year, month)) {
        return None;
    }
    Some((year, month, day as u32))
}

fn parse_rfc3339(bytes: &[u8]) -> Option<i64> {
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let (year, month, day) = parse_date(&bytes[0..10])?;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

fn parse_digits(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0i64;
    for b in bytes {
        value = value * 10 + i64::from(b - b'0');
    }
    Some(value)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(raw: &str) -> Result<i64> {
        parse_time_bound(raw, "mem query", "--since")
    }

    #[test]
    fn parses_iso_and_epoch() {
        assert_eq!(bound("2026-11-12").unwrap(), 1_794_441_600);
        assert_eq!(bound("@1794441600").unwrap(), 1_794_441_600);
    }

    #[test]
    fn parses_rfc3339() {
        assert_eq!(bound("2026-11-12T00:00:00Z").unwrap(), 1_794_441_600);
        assert_eq!(bound("2026-11-12T01:02:03Z").unwrap(), 1_794_441_600 + 3723);
    }

    #[test]
    fn accepts_leap_day() {
        assert_eq!(bound("2024-02-29").unwrap(), 1_709_164_800);
    }

    #[test]
    fn rejects_invalid_bounds() {
        for raw in [
            "20261111",
            "2026-02-30",
            "2023-02-29",
            "1969-12-31",
            "10000-01-01",
            "2026-11-12T00:00:60Z",
            "2026-11-12T00:00:00+07:00",
            "2026-11-12T00:00:00.5Z",
            "2026-11-12T24:00:00Z",
            "2026-11-12 ",
            "@",
            "@-1",
            "@999999999999999999999",
        ] {
            assert!(bound(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn validates_order() {
        let ok = TimeWindow {
            since: Some(1),
            until: Some(1),
        };
        assert!(validate_window(&ok, "mem query").is_ok());
        let bad = TimeWindow {
            since: Some(2),
            until: Some(1),
        };
        assert!(validate_window(&bad, "mem query").is_err());
    }
}
