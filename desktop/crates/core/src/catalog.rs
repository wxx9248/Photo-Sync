//! The phone's listing of the files it holds.

use crate::id::{DevicePath, Timestamp};

/// One file as the phone sees it. The three fields together are the identity used by the diff,
/// the staging manifest, and the deletion candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub path: DevicePath,
    pub size: u64,
    pub mtime: Timestamp,
}

/// A catalog frozen at the start of a session. It never changes while the session is alive,
/// including across reconnections, so a repeated diff can only shrink the work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
    total_bytes: u64,
}

impl Catalog {
    pub fn new(entries: Vec<CatalogEntry>, total_bytes: u64) -> Self {
        Self {
            entries,
            total_bytes,
        }
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64) -> CatalogEntry {
        CatalogEntry {
            path: DevicePath::new(format!("DCIM/Camera/{name}")),
            size,
            mtime: Timestamp(1_756_000_000),
        }
    }

    #[test]
    fn a_catalog_keeps_the_entries_and_the_total_it_was_given() {
        let catalog = Catalog::new(vec![entry("one.jpg", 2400), entry("two.jpg", 1000)], 3400);

        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.total_bytes(), 3400);
        assert!(!catalog.is_empty());
    }

    #[test]
    fn a_phone_with_nothing_on_it_has_an_empty_catalog() {
        let catalog = Catalog::new(Vec::new(), 0);

        assert!(catalog.is_empty());
        assert_eq!(catalog.total_bytes(), 0);
    }
}
