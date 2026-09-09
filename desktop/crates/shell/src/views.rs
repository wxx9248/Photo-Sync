//! What the window shows, worked out before any of it is drawn.
//!
//! The window itself is markup and a handful of bindings; everything it displays is decided
//! here, in plain Rust that can be checked without a screen. That split is deliberate. A
//! progress figure that is wrong is wrong whether or not anybody was looking at it, and
//! `docs/ROADMAP.md` says the rendering itself is reviewed by hand — so the less that lives
//! only in the markup, the less is riding on somebody's eyes.

use std::collections::BTreeMap;

use photo_sync_core::effect::UiUpdate;
use photo_sync_core::id::DeviceId;

/// One phone, as a person reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phone {
    pub name: String,
    pub detail: String,

    /// How far along, in hundredths, so the comparison in a test is exact.
    pub progress: u32,
    pub busy: bool,
}

/// Everything the window is showing.
#[derive(Debug, Default)]
pub struct Views {
    phones: BTreeMap<DeviceId, Phone>,
}

impl Views {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one thing the desktop said about itself.
    pub fn observe(&mut self, update: &UiUpdate) {
        match update {
            UiUpdate::DeviceProgress {
                device,
                files_done,
                files_total,
                bytes_done,
            } => {
                let entry = self.phones.entry(device.clone()).or_insert_with(|| Phone {
                    name: device.to_string(),
                    detail: String::new(),
                    progress: 0,
                    busy: true,
                });
                entry.busy = true;
                entry.progress = fraction(*files_done, *files_total);
                entry.detail = format!(
                    "{files_done} of {files_total} photographs, {}",
                    size(*bytes_done)
                );
            }
            UiUpdate::CommitStarted { device } => {
                let entry = self.phones.entry(device.clone()).or_insert_with(|| Phone {
                    name: device.to_string(),
                    detail: String::new(),
                    progress: 0,
                    busy: true,
                });
                entry.busy = true;
                entry.detail = "Filing them away…".to_string();
            }
            UiUpdate::CommitFinished { device, summary } => {
                let entry = self.phones.entry(device.clone()).or_insert_with(|| Phone {
                    name: device.to_string(),
                    detail: String::new(),
                    progress: 100,
                    busy: false,
                });
                entry.busy = false;
                entry.progress = 100;
                entry.detail = format!(
                    "{} kept, {} already had",
                    summary.imported, summary.duplicates
                );
            }
            UiUpdate::Error { device, message } => {
                let entry = self.phones.entry(device.clone()).or_insert_with(|| Phone {
                    name: device.to_string(),
                    detail: String::new(),
                    progress: 0,
                    busy: false,
                });
                entry.busy = false;
                entry.detail = message.clone();
            }
            UiUpdate::Forgotten { device, path } => {
                if let Some(entry) = self.phones.get_mut(device) {
                    entry.detail = format!("{path} will be sent again");
                }
            }
        }
    }

    /// The phones to show, in a settled order so the list does not jump about.
    #[must_use]
    pub fn phones(&self) -> Vec<Phone> {
        self.phones.values().cloned().collect()
    }
}

/// How far through, in hundredths. Nothing to do is finished rather than undefined.
fn fraction(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 100;
    }
    let scaled = done.saturating_mul(100) / total;
    u32::try_from(scaled.min(100)).unwrap_or(100)
}

/// A byte count as a person would say it.
fn size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let amount = bytes as f64;
    for (limit, unit) in [
        (1024.0 * 1024.0 * 1024.0, "GB"),
        (1024.0 * 1024.0, "MB"),
        (1024.0, "kB"),
    ] {
        if amount >= limit {
            return format!("{:.1} {unit}", amount / limit);
        }
    }
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_sync_core::effect::CommitSummary;

    fn phone() -> DeviceId {
        DeviceId::new("phone-a")
    }

    #[test]
    fn a_phone_partway_through_reads_as_partway_through() {
        let mut views = Views::new();
        views.observe(&UiUpdate::DeviceProgress {
            device: phone(),
            files_done: 3,
            files_total: 4,
            bytes_done: 2 * 1024 * 1024,
        });

        let shown = views.phones();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].progress, 75);
        assert!(shown[0].busy);
        assert_eq!(shown[0].detail, "3 of 4 photographs, 2.0 MB");
    }

    #[test]
    fn a_phone_with_nothing_to_send_is_not_stuck_at_the_start() {
        assert_eq!(fraction(0, 0), 100);
    }

    #[test]
    fn progress_never_reads_past_the_end() {
        assert_eq!(fraction(9, 4), 100);
    }

    #[test]
    fn a_finished_commit_stops_the_bar_and_says_what_happened() {
        let mut views = Views::new();
        views.observe(&UiUpdate::CommitStarted { device: phone() });
        views.observe(&UiUpdate::CommitFinished {
            device: phone(),
            summary: CommitSummary {
                imported: 2,
                duplicates: 1,
                bytes_committed: 4096,
            },
        });

        let shown = views.phones();
        assert!(!shown[0].busy);
        assert_eq!(shown[0].progress, 100);
        assert_eq!(shown[0].detail, "2 kept, 1 already had");
    }

    #[test]
    fn a_phone_that_hit_trouble_says_so_and_stops() {
        let mut views = Views::new();
        views.observe(&UiUpdate::Error {
            device: phone(),
            message: "the import stopped part-way: no space".to_string(),
        });

        let shown = views.phones();
        assert!(!shown[0].busy);
        assert_eq!(shown[0].detail, "the import stopped part-way: no space");
    }

    #[test]
    fn sizes_read_the_way_a_person_would_say_them() {
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(1024 * 1024 * 3), "3.0 MB");
        assert_eq!(size(1024 * 1024 * 1024 * 2), "2.0 GB");
    }
}
