//! Canonical pure-Rust UTC / RFC-3339 timestamp parsing and calendar arithmetic (`INV-03`).
//!
//! Single source of truth for exact epoch-millisecond temporal fencing across all crates
//! (`fcast-infra`, `fcast-ingest`, `fcast-epistemic`, `fcast-bench`), preventing ASCII
//! lexicographic ordering bugs (`'Z' > '.'`) and invalid-date acceptance (`2026-02-31`).

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TimeParseError {
    #[error("invalid UTC/RFC-3339 timestamp: {0}")]
    InvalidTimestamp(String),
}

/// Returns `true` iff `year` is a proleptic Gregorian leap year.
#[must_use]
pub const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Returns the number of calendar days in `(year, month)` (`month` in `1..=12`).
#[must_use]
pub const fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Howard Hinnant's civil-to-days formula: exact days since `1970-01-01`.
#[must_use]
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m_i = month as i64;
    let doy = (153 * (m_i + if m_i > 2 { -3 } else { 9 }) + 2) / 5 + (day as i64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_digits_u32(slice: &str) -> Option<u32> {
    if slice.is_empty() || !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse::<u32>().ok()
}

fn parse_digits_i64(slice: &str) -> Option<i64> {
    if slice.is_empty() || !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse::<i64>().ok()
}

/// Parses a `YYYY-MM-DD` date prefix into exact days since `1970-01-01`.
#[must_use]
pub fn parse_ymd_days(ts: &str) -> Option<i64> {
    if !ts.is_ascii() || ts.len() < 10 {
        return None;
    }
    let b = ts.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = parse_digits_i64(&ts[0..4])?;
    let month = parse_digits_u32(&ts[5..7])?;
    let day = parse_digits_u32(&ts[8..10])?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Parses a strict RFC-3339 timestamp (`YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`) into exact UTC
/// epoch milliseconds, validating calendar days, leap years, and timezone offset bounds.
pub fn parse_rfc3339_epoch_ms(raw: &str) -> Result<i64, TimeParseError> {
    parse_utc_impl(raw, false)
}

/// Parses a UTC or RFC-3339 timestamp, additionally permitting minute-precision timestamps
/// (`YYYY-MM-DD HH:MM`, `YYYY-MM-DDTHH:MMZ`) and space separators used in news/event feeds.
pub fn parse_utc_epoch_ms(raw: &str) -> Result<i64, TimeParseError> {
    parse_utc_impl(raw.trim(), true)
}

/// Exact epoch-millisecond `lhs <= rhs` check for `INV-03` temporal fencing.
/// Returns `false` if either timestamp fails RFC-3339/UTC parsing (fail-closed).
#[must_use]
pub fn is_utc_le(lhs: &str, rhs: &str) -> bool {
    match (parse_utc_epoch_ms(lhs), parse_utc_epoch_ms(rhs)) {
        (Ok(a), Ok(b)) => a <= b,
        _ => false,
    }
}

fn parse_utc_impl(s: &str, allow_minute_and_space: bool) -> Result<i64, TimeParseError> {
    let err = || TimeParseError::InvalidTimestamp(s.to_owned());
    if !s.is_ascii() {
        return Err(err());
    }
    let b = s.as_bytes();
    let min_len = if allow_minute_and_space { 16 } else { 20 };
    if b.len() < min_len || b[4] != b'-' || b[7] != b'-' || b[13] != b':' {
        return Err(err());
    }
    let sep_ok = b[10] == b'T' || b[10] == b't' || (allow_minute_and_space && b[10] == b' ');
    if !sep_ok {
        return Err(err());
    }

    let year = parse_digits_i64(&s[0..4]).ok_or_else(err)?;
    let month = parse_digits_u32(&s[5..7]).ok_or_else(err)?;
    let day = parse_digits_u32(&s[8..10]).ok_or_else(err)?;
    let hour = parse_digits_u32(&s[11..13]).ok_or_else(err)?;
    let minute = parse_digits_u32(&s[14..16]).ok_or_else(err)?;

    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
    {
        return Err(err());
    }

    let mut idx = 16;
    let mut second = 0_u32;
    let mut millis = 0_i64;

    if idx < b.len() && b[idx] == b':' {
        if idx + 3 > b.len() {
            return Err(err());
        }
        second = parse_digits_u32(&s[idx + 1..idx + 3]).ok_or_else(err)?;
        if second > 59 {
            return Err(err());
        }
        idx += 3;

        if idx < b.len() && b[idx] == b'.' {
            idx += 1;
            let frac_start = idx;
            while idx < b.len() && b[idx].is_ascii_digit() {
                idx += 1;
            }
            let frac = &s[frac_start..idx];
            if frac.is_empty() {
                return Err(err());
            }
            let mut scale = 100_i64;
            for ch in frac.bytes().take(3) {
                millis += i64::from(ch - b'0') * scale;
                scale /= 10;
            }
        }
    } else if !allow_minute_and_space {
        return Err(err());
    }

    let offset_sec: i64 = if idx < b.len() && (b[idx] == b'Z' || b[idx] == b'z') {
        idx += 1;
        0
    } else if idx + 6 == b.len() && (b[idx] == b'+' || b[idx] == b'-') && b[idx + 3] == b':' {
        let sign = if b[idx] == b'+' { 1_i64 } else { -1_i64 };
        let oh = parse_digits_u32(&s[idx + 1..idx + 3]).ok_or_else(err)?;
        let om = parse_digits_u32(&s[idx + 4..idx + 6]).ok_or_else(err)?;
        if oh > 23 || om > 59 {
            return Err(err());
        }
        idx += 6;
        sign * (i64::from(oh) * 3_600 + i64::from(om) * 60)
    } else if allow_minute_and_space && idx == b.len() {
        0
    } else {
        return Err(err());
    };

    if idx != b.len() {
        return Err(err());
    }

    let days = days_from_civil(year, month, day);
    let total_sec = days
        .checked_mul(86_400)
        .and_then(|d| {
            d.checked_add(i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))
        })
        .and_then(|t| t.checked_sub(offset_sec))
        .ok_or_else(err)?;
    total_sec
        .checked_mul(1_000)
        .and_then(|ms| ms.checked_add(millis))
        .ok_or_else(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_calendar_offsets_utf8_and_fractional_ordering() {
        // Reject impossible calendar dates & invalid timezone offsets & non-ASCII
        assert!(parse_rfc3339_epoch_ms("2026-02-29T00:00:00Z").is_err());
        assert!(parse_rfc3339_epoch_ms("2024-02-29T00:00:00Z").is_ok());
        assert!(parse_rfc3339_epoch_ms("2026-02-31T00:00:00Z").is_err());
        assert!(parse_rfc3339_epoch_ms("2026-11-01T10:00:00+25:00").is_err());
        assert!(parse_rfc3339_epoch_ms("2026-11-01T10:00:00+02:60").is_err());
        assert!(parse_rfc3339_epoch_ms("2026-é1-01T10:00:00Z").is_err());
        assert!(parse_rfc3339_epoch_ms("2026-+1-+1T+0:+0:+0Z").is_err());
        assert!(parse_rfc3339_epoch_ms("2026-11-01 10:00").is_err());
        assert!(parse_utc_epoch_ms("2026-11-01 10:00").is_ok());

        // Exact fractional-second & offset ordering (prevents ASCII 'Z' > '.' bug)
        assert!(!is_utc_le(
            "2026-11-01T10:00:00.500Z",
            "2026-11-01T10:00:00Z"
        ));
        assert!(is_utc_le(
            "2026-11-01T10:00:00Z",
            "2026-11-01T10:00:00.500Z"
        ));
        assert_eq!(
            parse_rfc3339_epoch_ms("2026-11-01T12:00:00+02:00").expect("tz"),
            parse_rfc3339_epoch_ms("2026-11-01T10:00:00Z").expect("utc")
        );
    }
}
