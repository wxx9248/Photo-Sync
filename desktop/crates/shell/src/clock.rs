//! The desktop clock, as both an instant and a wall-clock reading.
//!
//! The core cannot read a clock and cannot turn one form into the other, because a wall clock
//! needs a timezone and a timezone is ambient state. The shell answers with both.
//!
//! The reading is UTC for now. Converting to the machine's own zone needs a timezone database
//! and belongs with the capture-time extraction of `SPEC.md` §7.2, which is not built either.
//! Until both land, a vault name falling back to the import time reads in UTC rather than in
//! local time, which is a cosmetic difference in a field §7.2 already calls cosmetic.

use photo_sync_core::Moment;
use photo_sync_core::id::Timestamp;

/// Reads the machine clock.
#[must_use]
#[allow(clippy::disallowed_types)]
pub fn now() -> Moment {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let at = i64::try_from(seconds).unwrap_or(i64::MAX);
    Moment {
        at: Timestamp(at),
        local: civil_from_unix(at),
    }
}

pub use photo_sync_core::naming::civil_from_unix;

#[cfg(test)]
mod tests {
    use super::*;
    use photo_sync_core::CivilTime;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> CivilTime {
        CivilTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn the_epoch_reads_as_the_first_of_january_nineteen_seventy() {
        assert_eq!(civil_from_unix(0), at(1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn a_known_instant_reads_as_its_published_time() {
        // 2026-08-24T09:30:00Z, checked against the same value the scenarios use.
        assert_eq!(civil_from_unix(1_787_563_800), at(2026, 8, 24, 9, 30, 0));
    }

    #[test]
    fn the_last_second_of_a_leap_day_reads_correctly() {
        // 2024-02-29T23:59:59Z
        assert_eq!(civil_from_unix(1_709_251_199), at(2024, 2, 29, 23, 59, 59));
    }

    #[test]
    fn the_first_second_of_a_century_reads_correctly() {
        // 2000-03-01T00:00:00Z, the day the arithmetic shifts its year on.
        assert_eq!(civil_from_unix(951_868_800), at(2000, 3, 1, 0, 0, 0));
    }
}
