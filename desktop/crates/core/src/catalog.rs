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
