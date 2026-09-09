//! The pairing code, put to the desktop half.
//!
//! `verification/vectors/pairing.tsv` states the six digits both screens have to show for a
//! given pair of keys, and both implementations answer it. The code is only worth reading
//! aloud because it covers *both* keys: somebody standing in the middle holds a different key
//! on each side, so the two screens disagree and a person sees it. That argument collapses if
//! the two ends derive the code differently, and two readings of one paragraph is not a
//! guarantee that they do not.

use photo_sync::identity::pairing_code;
use photo_sync_core::covers;

#[test]
fn the_pairing_code_is_the_one_both_screens_have_to_show() {
    covers!("R-PAIR-001");
    let cases = photo_sync_vectors::read("pairing.tsv");
    assert!(
        cases.len() >= 4,
        "the vectors were not read: {}",
        cases.len()
    );

    for case in &cases {
        assert_eq!(
            pairing_code(&case.bytes("desktop_key"), &case.bytes("phone_key")),
            case.field("code"),
            "{}",
            case.name()
        );
    }
}
