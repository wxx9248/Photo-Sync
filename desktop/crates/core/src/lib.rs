//! Logic for the desktop half of Photo Sync.
//!
//! Everything in this crate is driven by [`Event`] values and answers with [`Effect`] values.
//! Nothing here opens a file, waits on a socket, reads a clock, or starts a thread. The shell
//! performs the effects and reports what happened as further events, which lets the same logic
//! run inside the simulator described in `docs/VERIFICATION.md`.

pub mod catalog;
pub mod commit;
pub mod desktop;
pub mod digest;
pub mod effect;
pub mod event;
pub mod id;
pub mod naming;
pub mod port;
mod session;
pub mod store;

pub use catalog::{Catalog, CatalogEntry};
pub use desktop::Desktop;
pub use digest::RunningDigest;
pub use effect::{Directory, Effect, RejectReason};
pub use event::Event;
pub use id::{DeviceId, DevicePath, FileId, OpId, Sha256, Timestamp, VaultName};
pub use naming::{CivilTime, Moment};

/// Declares which requirements a test covers.
///
/// The macro expands to nothing. The runner finds these declarations by reading the sources,
/// and correlates them with test results to build the traceability matrix.
#[macro_export]
macro_rules! covers {
    ($($id:literal),+ $(,)?) => {};
}
