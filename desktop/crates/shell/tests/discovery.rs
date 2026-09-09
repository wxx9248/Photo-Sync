//! Being found on a real network, with a real responder.
//!
//! `SPEC.md` §5.1 says the start order of the two applications does not matter, which is a
//! claim about a responder that keeps answering rather than a search that happens once. The
//! browser here stands in for the phone's `NsdManager`: a second, independent daemon that
//! learns the desktop exists without having been told an address.

use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use photo_sync::discovery::{Advertising, SERVICE_TYPE};
use photo_sync_core::covers;

/// Long enough for a multicast round trip on a quiet machine, short enough that a failure is
/// reported rather than waited on.
const PATIENCE: Duration = Duration::from_secs(10);

#[test]
fn a_phone_finds_the_desktop_without_being_told_where_it_is() {
    covers!("R-DISCOVER-001");
    let advertising = match Advertising::start("Kitchen iMac", 45_123, &[]) {
        Ok(advertising) => advertising,
        Err(error) => panic!("cannot announce the desktop: {error}"),
    };

    let browser = match ServiceDaemon::new() {
        Ok(browser) => browser,
        Err(error) => panic!("cannot browse: {error}"),
    };
    let found = match browser.browse(SERVICE_TYPE) {
        Ok(found) => found,
        Err(error) => panic!("cannot browse: {error}"),
    };

    let deadline = std::time::Instant::now() + PATIENCE;
    let resolved = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "nothing answered in {PATIENCE:?}");
        match found.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(service)) => break service,
            Ok(_) => continue,
            Err(error) => panic!("the browse ended: {error}"),
        }
    };

    // What the phone learns is enough to open a connection and enough to know whether it
    // should: the port, and the protocol version, without a byte of configuration.
    assert_eq!(resolved.get_port(), 45_123);
    assert_eq!(
        resolved.get_property_val_str("v"),
        Some(photo_sync_protocol::PROTOCOL_VERSION.to_string().as_str())
    );
    assert_eq!(resolved.get_property_val_str("name"), Some("Kitchen iMac"));
    assert!(
        resolved.fullname.starts_with("Kitchen iMac."),
        "the desktop answers to {}",
        resolved.fullname
    );

    advertising.stop();
    let _ = browser.shutdown();
}
