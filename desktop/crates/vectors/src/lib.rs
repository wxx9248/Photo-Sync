//! The shared conformance vectors, read the same way by every Rust test.
//!
//! `verification/vectors/` states in cases what `SPEC.md` states in prose, and both
//! implementations answer the same files. The Kotlin session module reads them too. A reader
//! per test suite is a format that can drift between suites, so there is one here and the
//! tests say only what their own columns mean.

use std::str::FromStr;

/// One row of a vector file, keyed by the column names in its header.
pub struct Case {
    columns: Vec<(String, String)>,
}

impl Case {
    /// What this case is called. Every vector file has a `name` column, because a failure
    /// naming the row is worth more than one naming the line number.
    #[must_use]
    pub fn name(&self) -> &str {
        self.field("name")
    }

    /// One column of this case.
    ///
    /// # Panics
    ///
    /// When the file has no such column. The vectors are part of the repository rather than
    /// anything a peer sends, so a missing column is a mistake in the test, not an input to
    /// handle.
    #[must_use]
    pub fn field(&self, column: &str) -> &str {
        match self.columns.iter().find(|(named, _)| named == column) {
            Some((_, value)) => value,
            None => panic!("the vectors have no column {column}"),
        }
    }

    /// One column that is written as a number.
    ///
    /// # Panics
    ///
    /// When the column does not parse as one.
    pub fn number<T: FromStr>(&self, column: &str) -> T {
        let text = self.field(column);
        let Ok(value) = text.parse() else {
            panic!("{}: {column} is not a number: {text}", self.name())
        };
        value
    }

    /// One column that is written as hexadecimal, as the bytes it stands for.
    ///
    /// # Panics
    ///
    /// When the column is not an even number of hexadecimal digits.
    #[must_use]
    pub fn bytes(&self, column: &str) -> Vec<u8> {
        let text = self.field(column);
        assert!(
            text.len().is_multiple_of(2),
            "{}: {column} is not a whole number of bytes",
            self.name()
        );
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let Ok(digits) = std::str::from_utf8(pair) else {
                    panic!("{}: {column} is not hexadecimal", self.name())
                };
                let Ok(byte) = u8::from_str_radix(digits, 16) else {
                    panic!("{}: {column} is not hexadecimal", self.name())
                };
                byte
            })
            .collect()
    }
}

/// Every case in one vector file, named the way the directory names it.
///
/// Blank lines and lines starting with `#` are commentary. The first line left is the header,
/// and every line after it is a case.
///
/// # Panics
///
/// When the file cannot be read or holds no header. Either means the repository is not in the
/// state the test was written against, which no test can carry on from.
#[must_use]
pub fn read(file: &str) -> Vec<Case> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../verification/vectors")
        .join(file);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    };

    let mut lines = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    let Some(header) = lines.next() else {
        panic!("{file} has no header")
    };
    let columns: Vec<String> = header.split('\t').map(str::to_string).collect();

    lines
        .map(|line| Case {
            columns: columns
                .iter()
                .cloned()
                .zip(line.split('\t').map(str::to_string))
                .collect(),
        })
        .collect()
}
