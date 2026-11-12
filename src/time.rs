//! Shared UTC civil-date math (Howard Hinnant's algorithms).
//!
//! Used by audit timestamp rendering and by memory time-bound parsing. The
//! module is crate-private: it is an implementation detail, not a public API.

/// Civil date (year, month, day) from days since 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m as u32, d as u32)
}

/// Days since 1970-01-01 for a civil date. `month` is 1-12 and `day` is 1-31;
/// callers validate the calendar before calling this.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(month) + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_round_trip() {
        for (days, want) in [
            (0, (1970, 1, 1)),
            (19_782, (2024, 2, 29)),
            (2_932_896, (9999, 12, 31)),
            (-1, (1969, 12, 31)),
        ] {
            assert_eq!(civil_from_days(days), want);
            assert_eq!(days_from_civil(want.0, want.1, want.2), days);
        }
    }

    #[test]
    fn known_epochs() {
        assert_eq!(days_from_civil(2026, 11, 12) * 86_400, 1_794_441_600);
        assert_eq!(days_from_civil(2024, 2, 29) * 86_400, 1_709_164_800);
    }
}
