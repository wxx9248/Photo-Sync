//! The tiers, and what each one runs.

use std::path::Path;

use crate::cli::Tier;
use crate::report::{CheckResult, Report};
use crate::{coverage, scenario, spec_check, tools, workspace};

/// Paths that define correctness. Changing one without changing the specification is the
/// failure mode the gate exists to catch.
const PROTECTED: [&str; 4] = [
    "verification/requirements.toml",
    "verification/vectors/",
    "verification/scenarios/",
    "desktop/crates/model/",
];

/// Where a new file is evidence rather than a changed oracle.
///
/// `AGENTS.md` and `docs/VERIFICATION.md` protect the expectations of scenarios that already
/// exist, not the act of writing a new one. Guarding the directory as a whole would mean a
/// specification change for every scenario added, which teaches the opposite lesson to the
/// one the gate exists to teach.
const ADDABLE: &str = "verification/scenarios/";

const SPECIFICATION: &str = "docs/SPEC.md";

/// One path the range changed, and whether the change created it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub added: bool,
}

pub fn run_tier(root: &Path, tier: Tier) -> Result<bool, String> {
    let desktop = root.join("desktop");
    let mut report = Report::started(tier.name());

    report.add(formatting(&desktop)?);
    report.add(lints(&desktop)?);
    report.add(unit_tests(&desktop)?);
    report.add(scenarios(root)?);
    report.add(specification(root)?);
    report.add(requirement_matrix(root)?);

    if matches!(tier, Tier::Full | Tier::Nightly) {
        report.add(CheckResult::skipped(
            "campaign",
            "the simulator arrives in milestone M2, see docs/ROADMAP.md",
        ));
    }

    if matches!(tier, Tier::Nightly) {
        report.add(CheckResult::skipped(
            "mutation",
            "cargo-mutants is not installed on this machine",
        ));
    }

    report.finish(&workspace::reports_directory(root))?;
    Ok(report.passed())
}

fn formatting(desktop: &Path) -> Result<CheckResult, String> {
    if !tools::cargo_subcommand_available("fmt") {
        return Ok(CheckResult::skipped(
            "formatting",
            "rustfmt is not installed",
        ));
    }

    let passed = tools::run("cargo", &["fmt", "--check"], desktop)?;
    Ok(CheckResult::from_outcome(
        "formatting",
        passed,
        "run `cargo fmt` in desktop/",
    ))
}

fn lints(desktop: &Path) -> Result<CheckResult, String> {
    if !tools::cargo_subcommand_available("clippy") {
        return Ok(CheckResult::skipped("lints", "clippy is not installed"));
    }

    let passed = tools::run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        desktop,
    )?;
    Ok(CheckResult::from_outcome(
        "lints",
        passed,
        "clippy reported problems",
    ))
}

fn unit_tests(desktop: &Path) -> Result<CheckResult, String> {
    let passed = if tools::cargo_subcommand_available("nextest") {
        tools::run("cargo", &["nextest", "run", "--workspace"], desktop)?
    } else {
        tools::run("cargo", &["test", "--workspace"], desktop)?
    };

    Ok(CheckResult::from_outcome(
        "tests",
        passed,
        "unit tests failed",
    ))
}

fn scenarios(root: &Path) -> Result<CheckResult, String> {
    let outcomes = scenario::run_all(&workspace::scenarios_directory(root))?;
    if outcomes.is_empty() {
        return Ok(CheckResult::skipped(
            "scenarios",
            "none are written yet, see docs/VERIFICATION.md",
        ));
    }

    let failed: Vec<&scenario::Outcome> = outcomes
        .iter()
        .filter(|outcome| !outcome.passed())
        .collect();
    for outcome in &failed {
        scenario::print_failure(outcome);
    }

    println!("scenarios: {} run, {} failed", outcomes.len(), failed.len());
    Ok(CheckResult::from_outcome(
        "scenarios",
        failed.is_empty(),
        "an acceptance scenario did not hold",
    ))
}

fn specification(root: &Path) -> Result<CheckResult, String> {
    let outcome = spec_check::run(root)?;
    Ok(CheckResult::from_outcome(
        "spec-check",
        outcome.passed(),
        "a registry quote no longer appears in the specification",
    ))
}

fn requirement_matrix(root: &Path) -> Result<CheckResult, String> {
    let passed = coverage::print_matrix(root)?;
    Ok(CheckResult::from_outcome(
        "matrix",
        passed,
        "an active requirement has no test",
    ))
}

pub fn protected_artifacts(root: &Path, against: &str) -> Result<bool, String> {
    let range = format!("{against}...HEAD");
    let output = std::process::Command::new("git")
        .args(["diff", "--name-status", &range])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "cannot compare against {against}: is the reference present?"
        ));
    }

    let changed = parse_changes(&String::from_utf8_lossy(&output.stdout));
    let touched = oracles_touched(&changed);

    if touched.is_empty() {
        println!("protected-artifacts: nothing protected changed");
        return Ok(true);
    }

    if changed.iter().any(|change| change.path == SPECIFICATION) {
        println!(
            "protected-artifacts: {} changed alongside the specification",
            touched.len()
        );
        return Ok(true);
    }

    println!("protected-artifacts: these define correctness and changed without a spec change");
    for change in touched {
        println!("  {}", change.path);
    }
    println!("Change {SPECIFICATION} in the same commit, or revert them.");
    Ok(false)
}

/// Reads `git diff --name-status`. A rename is reported as its destination, which is the
/// path a reviewer would look at.
fn parse_changes(output: &str) -> Vec<Change> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let status = fields.next()?;
            let path = fields.next_back()?;
            Some(Change {
                path: path.to_string(),
                added: status.starts_with('A'),
            })
        })
        .collect()
}

/// The changes that alter something defining correctness.
fn oracles_touched(changed: &[Change]) -> Vec<&Change> {
    changed
        .iter()
        .filter(|change| {
            let protected = PROTECTED
                .iter()
                .any(|protected| change.path.starts_with(protected));
            protected && !(change.added && change.path.starts_with(ADDABLE))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Status;

    #[test]
    fn every_protected_path_is_a_prefix_someone_can_match() {
        for path in PROTECTED {
            assert!(
                !path.starts_with('/'),
                "{path} should be repository relative"
            );
        }
    }

    fn change(status: &str, path: &str) -> Change {
        Change {
            path: path.to_string(),
            added: status.starts_with('A'),
        }
    }

    #[test]
    fn a_new_scenario_is_evidence_rather_than_a_changed_oracle() {
        let changed = [change("A", "verification/scenarios/S-XFER-001.toml")];

        assert!(oracles_touched(&changed).is_empty());
    }

    #[test]
    fn editing_a_scenario_that_already_exists_is_still_guarded() {
        let changed = [change("M", "verification/scenarios/S-XFER-001.toml")];

        assert_eq!(oracles_touched(&changed).len(), 1);
    }

    #[test]
    fn deleting_a_scenario_is_still_guarded() {
        let changed = [change("D", "verification/scenarios/S-XFER-001.toml")];

        assert_eq!(oracles_touched(&changed).len(), 1);
    }

    #[test]
    fn a_new_requirement_is_guarded_the_same_as_a_changed_one() {
        let changed = [change("A", "verification/requirements.toml")];

        assert_eq!(oracles_touched(&changed).len(), 1);
    }

    #[test]
    fn ordinary_code_is_not_protected() {
        let changed = [change("M", "desktop/crates/core/src/desktop.rs")];

        assert!(oracles_touched(&changed).is_empty());
    }

    #[test]
    fn a_rename_is_read_as_its_destination() {
        let parsed = parse_changes("R100\tdocs/OLD.md\tdocs/NEW.md\n");

        assert_eq!(parsed, vec![change("R100", "docs/NEW.md")]);
    }

    #[test]
    fn a_status_line_yields_the_path_and_whether_it_is_new() {
        let parsed = parse_changes("A\tverification/scenarios/one.toml\nM\tdocs/SPEC.md\n");

        assert_eq!(
            parsed,
            vec![
                change("A", "verification/scenarios/one.toml"),
                change("M", "docs/SPEC.md"),
            ]
        );
    }

    #[test]
    fn a_skipped_check_does_not_fail_a_tier() {
        assert_eq!(
            CheckResult::skipped("campaign", "not built").status,
            Status::Skipped
        );
    }
}
