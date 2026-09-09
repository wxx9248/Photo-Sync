//! The desktop's shell: the half that touches the machine.
//!
//! The core decides what must happen and in what order, and emits an [`Effect`] for each
//! step. Everything in this crate turns one of those into a system call and turns the outcome
//! back into an event. No rule about ordering, deduplication, naming, or safety belongs here;
//! `docs/VERIFICATION.md` explains why that separation is what makes the rules testable.
//!
//! [`Effect`]: photo_sync_core::Effect

pub mod autostart;
pub mod clock;
pub mod config;
pub mod desk;
pub mod discovery;
pub mod identity;
pub mod logging;
pub mod pinning;
pub mod serve;
pub mod server;
pub mod storage;
pub mod store;
pub mod suspend;
pub mod tls;
pub mod tray;
pub mod views;
