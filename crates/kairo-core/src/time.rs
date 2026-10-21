// SPDX-License-Identifier: GPL-3.0-or-later
//! UTC timestamps for logs and contract files, without a date-time dependency.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, or 0 if the clock is set before 1970.
pub fn unix_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The current time as RFC 3339 UTC with milliseconds, e.g. `2026-10-21T20:15:03.123Z`.
pub fn now_utc() -> String {
    format_utc_millis(unix_millis_now())
}

/// Formats milliseconds since the Unix epoch as RFC 3339 UTC with milliseconds.
pub fn format_utc_millis(unix_ms: u64) -> String {
    let millis = unix_ms % 1_000;
    let secs = unix_ms / 1_000;
    let (year, month, day) = civil_from_days(secs / 86_400);
    let secs_of_day = secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3_600,
        secs_of_day % 3_600 / 60,
        secs_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
///
/// Howard Hinnant's `civil_from_days`, for non-negative day counts only, so every step stays
/// in unsigned arithmetic. The epoch is shifted to 0000-03-01 so leap days end each year.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        // Expected values computed independently with Python's datetime.
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (946_684_799_999, "1999-12-31T23:59:59.999Z"),
            (951_868_799_999, "2000-02-29T23:59:59.999Z"),
            (1_709_208_000_000, "2024-02-29T12:00:00.000Z"),
            (1_792_613_703_123, "2026-10-21T20:15:03.123Z"),
            (2_147_483_648_000, "2038-01-19T03:14:08.000Z"),
            (4_107_542_400_001, "2100-03-01T00:00:00.001Z"),
        ];
        for (ms, expected) in cases {
            assert_eq!(format_utc_millis(ms), expected, "{ms}");
        }
    }

    #[test]
    fn now_is_after_this_code_was_written() {
        assert!(unix_millis_now() > 1_792_540_800_000);
        assert_eq!(now_utc().len(), "2026-10-21T20:15:03.123Z".len());
    }
}
