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

        // No login manager here. A build machine is the usual reason, and it is still a
        // perfectly good machine to sync photographs on. Said out loud, so a run that skipped
        // this is not mistaken for a run that checked it.
        Err(suspend::SuspendError::Unavailable(why)) => {
            println!("not checked here: {why}");
        }

        // There was one and it said no, which is the case worth failing over: that machine
        // will go to sleep in the middle of a transfer.
        Err(error) => panic!("{error}"),
    }
}
