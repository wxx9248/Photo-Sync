//! Keeping the machine awake while a phone is mid-transfer.
//!
//! `SPEC.md` §4 asks for one thing: the desktop inhibits system suspend while any session is
//! active. `STACK.md` §3.9 says how — `org.freedesktop.login1.Manager.Inhibit` with
//! `what="sleep"` and `mode="block"`, holding the file descriptor it hands back for as long
//! as the inhibition should last. Closing that descriptor is what lifts it, so the whole
//! mechanism is the lifetime of one open file.
//!
//! Two things are separated here because they fail in different ways. Which phones are busy
//! is bookkeeping and is always right. Whether the machine can actually be told to stay awake
//! depends on a session bus and a logind that may not be there — a desktop with no login
//! manager still syncs photographs, it just cannot promise the machine will not sleep
//! underneath it.

use std::collections::BTreeSet;
use std::os::fd::OwnedFd;

use photo_sync_core::id::DeviceId;

/// Which phones are in the middle of something.
#[derive(Debug, Default)]
pub struct Busy {
    devices: BTreeSet<DeviceId>,
}

impl Busy {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes that a phone has started, saying whether the machine has just become busy.
    ///
    /// Only the change matters: the first phone to arrive is what takes the inhibitor, and
    /// the ones after it are already covered by the one being held.
    pub fn started(&mut self, device: &DeviceId) -> bool {
        let was_idle = self.devices.is_empty();
        self.devices.insert(device.clone());
        was_idle
    }

    /// Notes that a phone has finished, saying whether the machine has just become idle.
    pub fn ended(&mut self, device: &DeviceId) -> bool {
        self.devices.remove(device);
        self.devices.is_empty()
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        !self.devices.is_empty()
    }
}

/// A promise from the login manager that the machine will not sleep, held open.
#[derive(Debug)]
pub struct Inhibition {
    _held: OwnedFd,
}

/// Why the machine could not be asked to stay awake.
///
/// The three cases are worth telling apart, because only one of them is a problem.
///
/// A machine with no login manager was never going to answer and still syncs photographs
/// perfectly well. A process that is not part of a login session — a build agent, a container
/// — is refused on those grounds and that says nothing about what happens on somebody's
/// desktop. A login manager that had every reason to agree and did not is the case that
/// matters: that machine will fall asleep in the middle of a transfer.
#[derive(Debug)]
pub enum SuspendError {
    /// No system bus, or no login manager on it.
    Unavailable(String),

    /// There is one, and this process may not ask it.
    NotPermitted(String),

    /// There is one, this process may ask, and it would not.
    Refused(String),
}

impl std::fmt::Display for SuspendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "there is no login manager here: {why}"),
            Self::NotPermitted(why) => write!(
                f,
                "the login manager will not take an inhibitor from this process: {why}"
            ),
            Self::Refused(why) => write!(f, "the login manager would not hold off sleeping: {why}"),
        }
    }
}

impl std::error::Error for SuspendError {}

/// Asks the login manager to hold off sleeping while photographs are moving.
///
/// The inhibition lasts exactly as long as the returned value: dropping it closes the
/// descriptor, which is how logind is told the machine may sleep again.
///
/// # Errors
/// When there is no system bus, no login manager on it, or it declines.
pub fn inhibit(why: &str) -> Result<Inhibition, SuspendError> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| SuspendError::Unavailable(error.to_string()))?;
    let answer = connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            // what, who, why, mode. Blocking rather than delaying: a transfer is not
            // something to be hurried through in the seconds before a suspend.
            &("sleep", "Photo Sync", why, "block"),
        )
        .map_err(classify)?;

    let held: zbus::zvariant::OwnedFd = answer
        .body()
        .deserialize()
        .map_err(|error| SuspendError::Refused(error.to_string()))?;
    Ok(Inhibition { _held: held.into() })
}

/// Sorts what came back into the three things it can mean.
///
/// A bus with nothing listening on that name is a machine without a login manager. A denial
/// is the caller being outside a login session, which is what a build agent is and what a
/// person at their desk is not. Anything else is a refusal on the merits.
fn classify(error: zbus::Error) -> SuspendError {
    let described = error.to_string();
    if described.contains("ServiceUnknown")
        || described.contains("NameHasNoOwner")
        || described.contains("was not provided by any .service")
    {
        return SuspendError::Unavailable(described);
    }
    if described.contains("AccessDenied") || described.contains("InteractiveAuthorizationRequired")
    {
        return SuspendError::NotPermitted(described);
    }
    SuspendError::Refused(described)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(name: &str) -> DeviceId {
        DeviceId::new(name)
    }

    #[test]
    fn the_first_phone_to_arrive_is_the_one_that_takes_the_inhibitor() {
        let mut busy = Busy::new();
        assert!(busy.started(&phone("phone-a")));
        assert!(!busy.started(&phone("phone-b")));
        assert!(busy.is_busy());
    }

    #[test]
    fn the_machine_may_sleep_only_once_the_last_phone_has_finished() {
        let mut busy = Busy::new();
        busy.started(&phone("phone-a"));
        busy.started(&phone("phone-b"));

        assert!(
            !busy.ended(&phone("phone-a")),
            "one phone leaving freed the machine"
        );
        assert!(busy.is_busy());
        assert!(busy.ended(&phone("phone-b")));
        assert!(!busy.is_busy());
    }

    #[test]
    fn a_phone_that_arrives_twice_is_still_one_phone() {
        let mut busy = Busy::new();
        busy.started(&phone("phone-a"));
        assert!(!busy.started(&phone("phone-a")));
        assert!(
            busy.ended(&phone("phone-a")),
            "a reconnection left the machine awake"
        );
    }
}
