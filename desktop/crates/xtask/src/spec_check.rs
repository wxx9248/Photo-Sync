//! Confirms every registry quote still appears in the specification.
//!
//! Whitespace is collapsed before comparing, because the specification wraps its lines and a
//! quoted sentence usually spans two of them. Nothing else is normalised, so a reworded
//! requirement is reported rather than accepted.

use std::path::Path;

use crate::requirements::Registry;
use crate::workspace;

pub(crate) struct Outcome {
    pub checked: usize,
    pub missing: Vec<String>,
}

impl Outcome {
    pub(crate) fn passed(&self) -> bool {
        self.missing.is_empty()
    }
}

pub(crate) fn run(root: &Path) -> Result<Outcome, String> {
    let registry = Registry::load(&workspace::requirements_file(root))?;
    let spec = std::fs::read_to_string(workspace::spec_file(root))
        .map_err(|error| format!("cannot read the specification: {error}"))?;
    let haystack = collapse(&spec);

    let mut missing = Vec::new();
    for requirement in &registry.requirements {
        if !haystack.contains(&collapse(&requirement.quote)) {
            missing.push(requirement.id.clone());
        }
    }

    let outcome = Outcome {
        checked: registry.requirements.len(),
        missing,
    };
    report(&outcome);
    Ok(outcome)
}

fn report(outcome: &Outcome) {
    if outcome.passed() {
        println!("spec-check: {} quotes match", outcome.checked);
        return;
    }

    println!(
        "spec-check: {} quotes no longer appear in docs/SPEC.md",
        outcome.missing.len()
    );
    for id in &outcome.missing {
        println!("  {id}");
    }
    println!("Confirm whether the requirement changed, then update both files together.");
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::collapse;

    #[test]
    fn wrapped_lines_compare_equal_to_a_single_line() {
        let wrapped = "fsync the vault and staging\n  directories, then append";
        let single = "fsync the vault and staging directories, then append";

        assert_eq!(collapse(wrapped), collapse(single));
    }
}
