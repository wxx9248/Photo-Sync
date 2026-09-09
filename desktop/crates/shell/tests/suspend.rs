//! Asking the real login manager to keep the machine awake.
//!
//! The bookkeeping — which phones are busy — is checked beside the code that does it. This is
//! the other half: whether `org.freedesktop.login1` actually answers, and whether what it
//! answers with is a descriptor that can be held. A machine with no login manager is a
//! perfectly good machine to sync photographs on, so a missing one is reported and skipped
//! rather than failed.

use photo_sync::suspend;
use photo_sync_core::covers;

#[test]
fn the_login_manager_agrees_to_hold_off_sleeping() {
    covers!("R-UI-002");
    match suspend::inhibit("moving photographs off a phone") {
        // Holding it is the whole mechanism: the promise lasts exactly as long as this value,
        // and dropping it is how logind is told the machine may sleep again.
        Ok(held) => drop(held),

        // Nothing to ask, or nothing this process may ask. A build agent is outside any
        // login session and is refused on those grounds, which says nothing about what
        // happens on somebody's desktop. Said out loud, so a run that skipped this is not
        // mistaken for a run that checked it.
        Err(
            error
            @ (suspend::SuspendError::Unavailable(_) | suspend::SuspendError::NotPermitted(_)),
        ) => {
            println!("not checked here: {error}");
        }

        // There was one and it said no, which is the case worth failing over: that machine
        // will go to sleep in the middle of a transfer.
        Err(error) => panic!("{error}"),
    }
}
