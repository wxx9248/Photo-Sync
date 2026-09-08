//! Wire types generated from `proto/photosync/v1`.
//!
//! Only the shell uses these. The core speaks its own vocabulary, and the shell converts at the
//! boundary, so a change to the wire format cannot reach the logic without passing through a
//! conversion someone had to write.

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/photosync.v1.rs"));
}

/// Version this build speaks. A peer announcing a different major version is turned away with a
/// message rather than negotiated with.
pub const PROTOCOL_VERSION: u32 = 1;
