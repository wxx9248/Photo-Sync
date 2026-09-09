//! The shared conformance vectors, put to the Rust half.
//!
//! `verification/vectors/deletion.tsv` holds the deletion decisions of `SPEC.md` §8 as cases
//! rather than prose, and both implementations answer them. The Kotlin session module runs
//! the same file. A disagreement is not one end failing a test of its own: it is the two
//! halves of one protocol having drifted apart, which is the thing a conformance vector
//! exists to catch before a phone does.

use photo_sync_core::RunningDigest;
use photo_sync_core::covers;
use photo_sync_core::effect::{CandidateOrigin, DeletionCandidate};
use photo_sync_core::event::DeletionResult;
use photo_sync_core::id::{DevicePath, Sha256, Timestamp};
use photo_sync_sim::{Phone, PhoneFile};
use photo_sync_vectors::Case;

fn digest_of(bytes: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(bytes);
    running.peek()
}

fn path() -> DevicePath {
    DevicePath::new("DCIM/Camera/IMG_0001.jpg")
}

/// The phone the case describes: holding the photograph, or no longer holding it.
fn phone_for(case: &Case) -> Phone {
    let mut phone = Phone::new("phone-a", "Kitchen phone");
    if case.field("local_present") == "yes" {
        phone.files.insert(
            path(),
            PhoneFile::new(
                case.number("local_mtime"),
                case.field("local_content").as_bytes().to_vec(),
            ),
        );
    }
    phone
}

/// What the desktop is offering to have deleted.
fn candidate_of(case: &Case) -> DeletionCandidate {
    DeletionCandidate {
        path: path(),
        size: case.number("candidate_size"),
        mtime: Timestamp(case.number("candidate_mtime")),
        expected: digest_of(case.field("vault_content").as_bytes()),
        origin: match case.field("origin") {
            "this-transfer" => CandidateOrigin::ThisTransfer,
            "earlier" => CandidateOrigin::Earlier,
            other => panic!("{}: no such origin: {other}", case.name()),
        },
    }
}

fn expected_of(case: &Case) -> DeletionResult {
    match case.field("expect") {
        "deleted" => DeletionResult::Deleted,
        "kept-changed" => DeletionResult::KeptChanged,
        "failed" => DeletionResult::Failed,
        other => panic!("{}: no such outcome: {other}", case.name()),
    }
}

#[test]
fn every_shared_case_is_decided_the_way_it_is_written_down() {
    covers!("R-DELETE-006", "R-DELETE-007", "R-DELETE-008");
    let cases = photo_sync_vectors::read("deletion.tsv");
    assert!(
        cases.len() >= 6,
        "the vectors were not read: {}",
        cases.len()
    );

    for case in &cases {
        assert_eq!(
            phone_for(case).would(&candidate_of(case)),
            expected_of(case),
            "{}",
            case.name()
        );
    }
}
