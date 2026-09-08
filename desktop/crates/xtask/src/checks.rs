//! The tiers, and what each one runs.

use std::path::Path;

use crate::cli::Tier;
use crate::report::{CheckResult, Report};
use crate::{coverage, spec_check, tools, workspace};

/// Paths that define correctness. Changing one without changing the specification is the
/// failure mode the gate exists to catch.
const PROTECTED: [&str; 4] = [
    "verification/requirements.toml",
    "verification/vectors/",
    "verification/scenarios/",
    "desktop/crates/model/",
];

const SPECIFICATION: &str = "docs/SPEC.md";

pub fn run_tier(root: &Path, tier: Tier) -> Result<bool, String> {
    let desktop = root.join("desktop");
    let mut report = Report::started(tier.name());

    report.add(formatting(&desktop)?);
    report.add(lints(&desktop)?);
    report.add(unit_tests(&desktop)?);
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
        .args(["diff", "--name-only", &range])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "cannot compare against {against}: is the reference present?"
        ));
    }

    let changed: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();

    let touched: Vec<&String> = changed
        .iter()
        .filter(|path| {
            PROTECTED
                .iter()
                .any(|protected| path.starts_with(protected))
        })
        .collect();

    if touched.is_empty() {
        println!("protected-artifacts: nothing protected changed");
        return Ok(true);
    }

    if changed.iter().any(|path| path == SPECIFICATION) {
        println!(
            "protected-artifacts: {} changed alongside the specification",
            touched.len()
        );
        return Ok(true);
    }

    println!("protected-artifacts: these define correctness and changed without a spec change");
    for path in touched {
        println!("  {path}");
    }
    println!("Change {SPECIFICATION} in the same commit, or revert them.");
    Ok(false)
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

    #[test]
    fn a_skipped_check_does_not_fail_a_tier() {
        assert_eq!(
            CheckResult::skipped("campaign", "not built").status,
            Status::Skipped
        );
    }
}
