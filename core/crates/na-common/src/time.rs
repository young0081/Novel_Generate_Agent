//! Tiny time helpers with zero external dependencies.
//!
//! We deliberately avoid pulling in `chrono`/`time` for a couple of formatting
//! helpers. Epoch milliseconds are the canonical timestamp stored everywhere;
//! [`format_utc`] renders them as an ISO-8601-ish string for audit logs and the UI.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current wall-clock time as Unix epoch milliseconds (UTC).
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Render epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
///
/// Uses Howard Hinnant's civil-from-days algorithm so it is correct across the
/// proleptic Gregorian calendar without any dependency.
pub fn format_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Convenience: format the current time.
pub fn now_utc_string() -> String {
    format_utc(now_millis())
}

/// Convert a count of days since the Unix epoch into a (year, month, day) triple.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn known_timestamp() {
        // 2021-01-01T00:00:00.000Z == 1609459200000 ms
        assert_eq!(format_utc(1_609_459_200_000), "2021-01-01T00:00:00.000Z");
    }

    #[test]
    fn with_millis_and_time() {
        // 2026-06-22T12:34:56.789Z
        // days from 1970-01-01 to 2026-06-22 = 20626; secs = 20626*86400 + 45296
        let ms = (20_626u64 * 86_400 + 12 * 3600 + 34 * 60 + 56) * 1000 + 789;
        assert_eq!(format_utc(ms), "2026-06-22T12:34:56.789Z");
    }

    #[test]
    fn now_is_after_2024() {
        assert!(now_millis() > 1_704_067_200_000);
    }
}
