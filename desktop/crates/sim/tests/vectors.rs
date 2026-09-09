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

fn digest_of(bytes: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(bytes);
    running.peek()
}

fn path() -> DevicePath {
    DevicePath::new("DCIM/Camera/IMG_0001.jpg")
}

/// Every case in the file, as a map from column name to value.
fn cases() -> Vec<Vec<(String, String)>> {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../verification/vectors/deletion.tsv"
    );
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => panic!("cannot read {file}: {error}"),
    };

    let mut lines = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    let columns: Vec<String> = match lines.next() {
        Some(header) => header.split('\t').map(str::to_string).collect(),
        None => panic!("the vectors have no header"),
    };

    lines
        .map(|line| {
            columns
                .iter()
                .cloned()
                .zip(line.split('\t').map(str::to_string))
                .collect()
        })
        .collect()
}

fn field<'a>(case: &'a [(String, String)], name: &str) -> &'a str {
    match case.iter().find(|(column, _)| column == name) {
        Some((_, value)) => value,
        None => panic!("the vectors have no column {name}"),
    }
}

#[test]
fn every_shared_case_is_decided_the_way_it_is_written_down() {
    covers!("R-DELETE-006", "R-DELETE-007", "R-DELETE-008");
    let cases = cases();
    assert!(
        cases.len() >= 6,
        "the vectors were not read: {}",
        cases.len()
    );

    for case in &cases {
        let name = field(case, "name");
        let mut phone = Phone::new("phone-a", "Kitchen phone");
        if field(case, "local_present") == "yes" {
            phone.files.insert(
                path(),
                PhoneFile::new(
                    field(case, "local_mtime")
                        .parse()
                        .unwrap_or_else(|_| panic!("{name}: the local time is not a number")),
                    field(case, "local_content").as_bytes().to_vec(),
                ),
            );
        }

        let candidate = DeletionCandidate {
            path: path(),
            size: field(case, "candidate_size")
                .parse()
                .unwrap_or_else(|_| panic!("{name}: the size is not a number")),
            mtime: Timestamp(
                field(case, "candidate_mtime")
                    .parse()
                    .unwrap_or_else(|_| panic!("{name}: the time is not a number")),
            ),
            expected: digest_of(field(case, "vault_content").as_bytes()),
            origin: match field(case, "origin") {
                "this-transfer" => CandidateOrigin::ThisTransfer,
                "earlier" => CandidateOrigin::Earlier,
                other => panic!("{name}: no such origin: {other}"),
            },
        };

        let expected = match field(case, "expect") {
            "deleted" => DeletionResult::Deleted,
            "kept-changed" => DeletionResult::KeptChanged,
            "failed" => DeletionResult::Failed,
            other => panic!("{name}: no such outcome: {other}"),
        };

        assert_eq!(phone.would(&candidate), expected, "{name}");
    }
}
