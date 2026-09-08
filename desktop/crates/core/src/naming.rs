//! The name a photo takes in the vault.
//!
//! `SPEC.md` §7.2 builds the name from a wall-clock reading, and a wall clock is not an
//! instant: turning one into the other needs a timezone, and a timezone is ambient state the
//! core is not allowed to read. The shell therefore converts every source before handing it
//! over, and this module only chooses between the readings, renders one, and settles
//! collisions. Names are cosmetic, and nothing about dedup or deletion depends on them.

use std::collections::BTreeSet;

use crate::id::{DevicePath, Timestamp, VaultName};

/// The earliest year a capture time may claim before it is treated as nonsense. `SPEC.md`
/// §7.2 falls through to the next source below this.
const EARLIEST_PLAUSIBLE_YEAR: i32 = 2000;

/// One moment, as both an instant and the local reading of it.
///
/// The core cannot derive either from the other, so whatever reads the clock supplies both.
/// A record of when a batch committed wants the instant; a name built from that time wants
/// the reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub at: Timestamp,
    pub local: CivilTime,
}

/// A wall-clock reading with no timezone attached, which is what a vault name is made of.
///
/// Field order is the comparison order, so one reading is earlier than another exactly when
/// it reads earlier on the same clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl CivilTime {
    /// Whether this reading is worth building a name from. A year before 2000 is a camera
    /// with a flat clock battery, and a reading after the import is a clock set wrong.
    #[must_use]
    pub fn is_plausible(&self, imported_at: &Self) -> bool {
        self.year >= EARLIEST_PLAUSIBLE_YEAR && self <= imported_at
    }

    fn render(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}_{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Picks the reading a name is built from.
///
/// The candidates are the sources of `SPEC.md` §7.2 in the order it lists them, which is
/// capture time first and the file's own modification time after it. The import time is the
/// terminal fallback and is used whether or not anything else was plausible.
#[must_use]
pub fn choose(candidates: &[CivilTime], imported_at: CivilTime) -> CivilTime {
    candidates
        .iter()
        .copied()
        .find(|candidate| candidate.is_plausible(&imported_at))
        .unwrap_or(imported_at)
}

/// The part of a name before any collision suffix and extension.
///
/// A commit uses this to ask which names it is about to assign are already spoken for.
#[must_use]
pub fn stem(candidates: &[CivilTime], imported_at: CivilTime) -> String {
    choose(candidates, imported_at).render()
}

/// Builds the name a file takes in the vault, avoiding every name already spoken for.
///
/// `taken` is both the names the index already holds and the names assigned earlier in this
/// batch, because a batch can easily hold two photos captured in the same second.
#[must_use]
pub fn assign(
    candidates: &[CivilTime],
    imported_at: CivilTime,
    path: &DevicePath,
    taken: &BTreeSet<VaultName>,
) -> VaultName {
    let stem = choose(candidates, imported_at).render();
    let extension = extension_of(path);

    let plain = VaultName::new(join(&stem, None, extension.as_deref()));
    if !taken.contains(&plain) {
        return plain;
    }

    // Suffixes start at one and count up. The batch is finite, so this always terminates.
    for suffix in 1u32.. {
        let candidate = VaultName::new(join(&stem, Some(suffix), extension.as_deref()));
        if !taken.contains(&candidate) {
            return candidate;
        }
    }

    unreachable!("a free suffix exists because the set of taken names is finite")
}

fn join(stem: &str, suffix: Option<u32>, extension: Option<&str>) -> String {
    let mut name = stem.to_string();
    if let Some(suffix) = suffix {
        name.push('_');
        name.push_str(&suffix.to_string());
    }
    if let Some(extension) = extension {
        name.push('.');
        name.push_str(extension);
    }
    name
}

/// The extension a vault copy keeps, lowercased.
///
/// The phone chooses this string, so it is accepted only when it is plainly alphanumeric.
/// Anything else, a separator above all, would decide where the file lands rather than what
/// it is called, and the file keeps no extension instead.
fn extension_of(path: &DevicePath) -> Option<String> {
    let name = path.as_str().rsplit('/').next()?;
    let extension = name.rsplit_once('.')?.1;

    let usable = !extension.is_empty()
        && extension.len() <= 16
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric());

    usable.then(|| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::covers;

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

    fn import() -> CivilTime {
        at(2026, 9, 7, 23, 0, 0)
    }

    fn photo(name: &str) -> DevicePath {
        DevicePath::new(format!("DCIM/Camera/{name}"))
    }

    fn nothing_taken() -> BTreeSet<VaultName> {
        BTreeSet::new()
    }

    #[test]
    fn a_name_reads_as_the_date_and_time_it_was_captured() {
        covers!("R-NAME-001");
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(&[captured], import(), &photo("IMG_1.jpg"), &nothing_taken());

        assert_eq!(name.as_str(), "2026-09-01_123456.jpg");
    }

    #[test]
    fn an_extension_is_kept_and_lowercased() {
        covers!("R-NAME-001");
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(
            &[captured],
            import(),
            &photo("IMG_1.JPEG"),
            &nothing_taken(),
        );

        assert_eq!(name.as_str(), "2026-09-01_123456.jpeg");
    }

    #[test]
    fn a_file_without_an_extension_is_named_without_one() {
        covers!("R-NAME-001");
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(&[captured], import(), &photo("IMG_1"), &nothing_taken());

        assert_eq!(name.as_str(), "2026-09-01_123456");
    }

    #[test]
    fn an_extension_that_could_choose_a_directory_is_dropped() {
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(
            &[captured],
            import(),
            &DevicePath::new("DCIM/Camera/IMG_1.jpg/../../etc"),
            &nothing_taken(),
        );

        assert_eq!(name.as_str(), "2026-09-01_123456");
    }

    #[test]
    fn an_empty_extension_is_dropped() {
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(&[captured], import(), &photo("IMG_1."), &nothing_taken());

        assert_eq!(name.as_str(), "2026-09-01_123456");
    }

    #[test]
    fn an_extension_nobody_would_choose_is_dropped() {
        let captured = at(2026, 9, 1, 12, 34, 56);
        let absurd = "x".repeat(40);

        let name = assign(
            &[captured],
            import(),
            &photo(&format!("IMG_1.{absurd}")),
            &nothing_taken(),
        );

        assert_eq!(name.as_str(), "2026-09-01_123456");
    }

    #[test]
    fn an_extension_that_is_not_plainly_alphanumeric_is_dropped() {
        let captured = at(2026, 9, 1, 12, 34, 56);

        let name = assign(&[captured], import(), &photo("IMG_1.j g"), &nothing_taken());

        assert_eq!(name.as_str(), "2026-09-01_123456");
    }

    #[test]
    fn a_second_photo_from_the_same_second_takes_a_suffix() {
        covers!("R-NAME-002");
        let captured = at(2026, 9, 1, 12, 34, 56);
        let taken = BTreeSet::from([VaultName::new("2026-09-01_123456.jpg")]);

        let name = assign(&[captured], import(), &photo("IMG_2.jpg"), &taken);

        assert_eq!(name.as_str(), "2026-09-01_123456_1.jpg");
    }

    #[test]
    fn suffixes_count_up_until_one_is_free() {
        covers!("R-NAME-002");
        let captured = at(2026, 9, 1, 12, 34, 56);
        let taken = BTreeSet::from([
            VaultName::new("2026-09-01_123456.jpg"),
            VaultName::new("2026-09-01_123456_1.jpg"),
            VaultName::new("2026-09-01_123456_2.jpg"),
        ]);

        let name = assign(&[captured], import(), &photo("IMG_4.jpg"), &taken);

        assert_eq!(name.as_str(), "2026-09-01_123456_3.jpg");
    }

    #[test]
    fn a_name_taken_by_another_extension_does_not_collide() {
        covers!("R-NAME-002");
        let captured = at(2026, 9, 1, 12, 34, 56);
        let taken = BTreeSet::from([VaultName::new("2026-09-01_123456.mp4")]);

        let name = assign(&[captured], import(), &photo("IMG_2.jpg"), &taken);

        assert_eq!(name.as_str(), "2026-09-01_123456.jpg");
    }

    #[test]
    fn a_camera_with_a_flat_clock_falls_through_to_the_next_source() {
        covers!("R-NAME-003");
        let flat = at(1980, 1, 1, 0, 0, 0);
        let modified = at(2026, 8, 30, 9, 0, 0);

        assert_eq!(choose(&[flat, modified], import()), modified);
    }

    #[test]
    fn a_reading_from_after_the_import_falls_through_to_the_next_source() {
        covers!("R-NAME-003");
        let ahead = at(2030, 1, 1, 0, 0, 0);
        let modified = at(2026, 8, 30, 9, 0, 0);

        assert_eq!(choose(&[ahead, modified], import()), modified);
    }

    #[test]
    fn the_import_time_names_a_file_no_other_source_could() {
        covers!("R-NAME-004");
        let flat = at(1980, 1, 1, 0, 0, 0);
        let ahead = at(2030, 1, 1, 0, 0, 0);

        assert_eq!(choose(&[flat, ahead], import()), import());
    }

    #[test]
    fn a_file_with_no_sources_at_all_is_named_for_its_import() {
        covers!("R-NAME-004");
        let name = assign(&[], import(), &photo("IMG_1.jpg"), &nothing_taken());

        assert_eq!(name.as_str(), "2026-09-07_230000.jpg");
    }

    #[test]
    fn a_reading_from_the_import_second_itself_is_plausible() {
        covers!("R-NAME-003");
        assert!(import().is_plausible(&import()));
    }

    #[test]
    fn the_first_second_of_the_year_two_thousand_is_plausible() {
        covers!("R-NAME-003");
        assert!(at(2000, 1, 1, 0, 0, 0).is_plausible(&import()));
    }

    #[test]
    fn the_last_second_before_it_is_not() {
        covers!("R-NAME-003");
        assert!(!at(1999, 12, 31, 23, 59, 59).is_plausible(&import()));
    }
}
