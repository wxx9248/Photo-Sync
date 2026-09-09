//! A handful of made-up sessions, to prove the campaign runs at all.
//!
//! The tier runs hundreds of these. What belongs here is only that a seed reproduces itself
//! and that a run which should be quiet is quiet.

use photo_sync_sim::campaign;

#[test]
fn a_seed_reproduces_itself() {
    let first = campaign::run(0x8f31_c0a9_4b2e);
    let again = campaign::run(0x8f31_c0a9_4b2e);

    assert_eq!(first.is_ok(), again.is_ok());
    if let (Err(first), Err(again)) = (first, again) {
        assert_eq!(format!("{first:?}"), format!("{again:?}"));
    }
}

#[test]
fn a_short_campaign_finds_nothing_to_complain_about() {
    for seed in 0..64u64 {
        if let Err(failure) = campaign::run(seed) {
            panic!("{failure}");
        }
    }
}
