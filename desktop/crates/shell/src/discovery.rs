//! Saying the desktop is here, so a phone can find it without being told an address.
//!
//! `SPEC.md` §5.1 puts the whole of discovery in one sentence: mDNS/DNS-SD, service type
//! `_photosync._tcp`, and the start order of the two applications does not matter. The last
//! part is what makes this a responder rather than a search: the desktop says what it is for
//! as long as it is running, and a phone that wakes up later hears it.
//!
//! The TXT record carries the protocol version and the desktop's display name. `STACK.md`
//! §5.4 puts the version in two places on purpose — here and in the handshake — so a phone
//! can tell an incompatible desktop apart from an absent one before it opens a connection.

use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// The service type `SPEC.md` §5.1 fixes, in the form DNS-SD wants it.
pub const SERVICE_TYPE: &str = "_photosync._tcp.local.";

/// TXT keys. Short, because a TXT record is not a place for prose.
const VERSION_KEY: &str = "v";
const NAME_KEY: &str = "name";

/// A desktop announcing itself, until it is dropped.
pub struct Advertising {
    daemon: ServiceDaemon,
    fullname: String,
}

/// What went wrong announcing the desktop.
#[derive(Debug)]
pub struct DiscoveryError(String);

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DiscoveryError {}

impl Advertising {
    /// Starts announcing a desktop on this machine.
    ///
    /// The instance name is the desktop's own display name, which is what a person picks out
    /// of a list, and the addresses are the ones a phone should try. An empty list means
    /// every address the machine has, which is what a home network wants.
    ///
    /// # Errors
    /// When the responder cannot start or the service description is not a legal one.
    pub fn start(
        display_name: &str,
        port: u16,
        addresses: &[IpAddr],
    ) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(problem)?;
        let host = format!("{}.local.", instance(display_name));
        let properties = [
            (
                VERSION_KEY,
                photo_sync_protocol::PROTOCOL_VERSION.to_string(),
            ),
            (NAME_KEY, display_name.to_string()),
        ];

        let mut service = ServiceInfo::new(
            SERVICE_TYPE,
            display_name,
            &host,
            addresses,
            port,
            &properties[..],
        )
        .map_err(problem)?;

        // No addresses given means every address this machine has, and means keeping up with
        // them: a laptop that moves between wifi and a cable is the ordinary case, and a
        // record naming an address it no longer has is worse than no record at all.
        if addresses.is_empty() {
            service = service.enable_addr_auto();
        }

        let fullname = service.get_fullname().to_string();
        daemon.register(service).map_err(problem)?;
        Ok(Self { daemon, fullname })
    }

    /// The name this desktop answers to, which is the instance name inside the service type.
    #[must_use]
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Stops announcing and waits for the goodbye to go out.
    ///
    /// A responder that simply exits leaves phones believing in a desktop that is no longer
    /// there until the records expire, so leaving is worth saying out loud.
    pub fn stop(self) {
        if let Ok(status) = self.daemon.unregister(&self.fullname) {
            let _ = status.recv();
        }
        if let Ok(status) = self.daemon.shutdown() {
            let _ = status.recv();
        }
    }
}

/// A display name reduced to something a hostname may contain.
fn instance(display_name: &str) -> String {
    let cleaned: String = display_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "photo-sync".to_string()
    } else {
        trimmed
    }
}

fn problem(error: impl std::fmt::Display) -> DiscoveryError {
    DiscoveryError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_something_a_hostname_can_hold() {
        assert_eq!(instance("Kitchen iMac"), "kitchen-imac");
    }

    #[test]
    fn a_name_of_nothing_usable_still_gives_a_host() {
        assert_eq!(instance("!!!"), "photo-sync");
    }
}
