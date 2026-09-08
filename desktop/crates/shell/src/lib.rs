//! The desktop's shell: the half that touches the machine.
//!
//! The core decides what must happen and in what order, and emits an [`Effect`] for each
//! step. Everything in this crate turns one of those into a system call and turns the outcome
//! back into an event. No rule about ordering, deduplication, naming, or safety belongs here;
//! `docs/VERIFICATION.md` explains why that separation is what makes the rules testable.
//!
//! [`Effect`]: photo_sync_core::Effect

pub mod storage;
pub mod store;
