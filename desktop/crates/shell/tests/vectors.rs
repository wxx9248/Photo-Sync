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

/// Every case in a vector file, as a map from column name to value.
fn cases(name: &str) -> Vec<Vec<(String, String)>> {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../verification/vectors")
        .join(name);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => panic!("cannot read {}: {error}", file.display()),
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

/// Hexadecimal as the vectors write it.
fn bytes(text: &str, name: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).unwrap_or_else(|_| panic!("{name}: not hex"));
            u8::from_str_radix(digits, 16).unwrap_or_else(|_| panic!("{name}: not hex"))
        })
        .collect()
}

#[test]
fn the_pairing_code_is_the_one_both_screens_have_to_show() {
    covers!("R-PAIR-001");
    let cases = cases("pairing.tsv");
    assert!(
        cases.len() >= 4,
        "the vectors were not read: {}",
        cases.len()
    );

    for case in &cases {
        let name = field(case, "name");
        assert_eq!(
            pairing_code(
                &bytes(field(case, "desktop_key"), name),
                &bytes(field(case, "phone_key"), name),
            ),
            field(case, "code"),
            "{name}"
        );
    }
}
