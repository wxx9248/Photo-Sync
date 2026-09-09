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
    let taken = suspend::inhibit("moving photographs off a phone");

    // A machine with no system bus has no login manager to ask, and is still a perfectly good
    // machine to sync photographs on. Anywhere there is one, this has to work: skipping on a
    // machine that could have answered would be a passing test that checked nothing.
    if !std::path::Path::new("/run/dbus/system_bus_socket").exists() {
        println!("no system bus here, so this was not checked");
        return;
    }

    match taken {
        // Holding it is the whole mechanism: the promise lasts exactly as long as this value,
        // and dropping it is how logind is told the machine may sleep again.
        Ok(held) => drop(held),
        Err(error) => {
            panic!("the login manager was there and would not hold off sleeping: {error}")
        }
    }
}
