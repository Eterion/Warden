//! Time as milliseconds since the Unix epoch, and the one timestamp format the inputs use (RFC 3339).

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Parses an RFC 3339 timestamp (`2026-11-07T12:10:00.123+01:00`, `…Z`) into milliseconds since the epoch.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter().try_fold(0i64, |acc, c| {
            c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
        })
    };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut i = 19;
    let mut millis = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            if i - start < 3 {
                millis = millis * 10 + i64::from(b[i] - b'0');
            }
            i += 1;
        }
        if i == start {
            return None;
        }
        for _ in (i - start)..3 {
            millis *= 10;
        }
    }
    let offset_min = match b.get(i) {
        Some(b'Z' | b'z') if i + 1 == b.len() => 0,
        Some(sign @ (b'+' | b'-')) if i + 6 == b.len() && b[i + 3] == b':' => {
            let m = num(i + 1, 2)? * 60 + num(i + 4, 2)?;
            if *sign == b'-' { -m } else { m }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_min * 60;
    Some(secs * 1000 + millis)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date (year, month, day) of a number of days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `2026-11-23` for a number of days since 1970-01-01.
pub fn date_text(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The days since 1970-01-01 of a date written `YYYY-MM-DD` exactly (a real date, nothing around it).
pub fn parse_date(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        b[from..from + len].iter().try_fold(0i64, |acc, c| {
            c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
        })
    };
    let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    // 2026-02-30 is not a day.
    (civil_from_days(days) == (y, m, d)).then_some(days)
}

/// How far this computer's time zone is ahead of UTC at a moment, in seconds (daylight saving included); UTC when
/// it cannot be told.
pub fn local_offset_s(at_ms: i64) -> i32 {
    ::time::OffsetDateTime::from_unix_timestamp(at_ms.div_euclid(1000))
        .ok()
        .and_then(|t| ::time::UtcOffset::local_offset_at(t).ok())
        .map_or(0, ::time::UtcOffset::whole_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_and_offsets() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-11-07T12:10:00Z"),
            Some(1_794_053_400_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-07T13:10:00+01:00"),
            Some(1_794_053_400_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-07T12:10:00.5Z"),
            Some(1_794_053_400_500)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-07T12:10:00.123456+00:00"),
            Some(1_794_053_400_123)
        );
    }

    #[test]
    fn dates_go_both_ways() {
        for days in [-1, 0, 59, 365, 20_415, 20_513, 30_000] {
            assert_eq!(parse_date(&date_text(days)), Some(days), "{days}");
        }
        assert_eq!(date_text(20_415), "2025-11-23");
        assert_eq!(parse_date("2026-11-23"), Some(20_780));
        for bad in [
            "2026-02-30",
            "2026-13-01",
            "2026-1-01",
            " 2026-11-23",
            "2026-11-23T",
            "2026/11/23",
            "",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn refuses_other_shapes() {
        for bad in [
            "",
            "2026-11-07",
            "2026-13-07T00:00:00Z",
            "2026-11-07T12:10:00",
            "2026-11-07T12:10:00.Z",
            "soon",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
    }
}
