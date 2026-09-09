//! The tiers, and what each one runs.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::Tier;
use crate::report::{Campaign, CheckResult, Failure, Report, Requirements};
use crate::requirements::{Registry, Status};
use crate::{campaign, coverage, mutants, scenario, selftest, spec_check, tools, workspace};

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

/// What `docs/VERIFICATION.md` §L6 asks of the full tier.
const CAMPAIGN_SEEDS: usize = 256;

/// The nightly tier has hours rather than minutes.
const NIGHTLY_SEEDS: usize = 4096;

const REGISTRY: &str = "verification/requirements.toml";

/// Wide enough that nobody scrolls past it.
const BANNER: &str = "========================================================================";

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
    let (check, failures) = scenarios(root, scenario::Against::Simulation, "scenarios")?;
    report.add(check);
    report.blame(failures);

    if matches!(tier, Tier::Full | Tier::Nightly) {
        let (check, failures) =
            scenarios(root, scenario::Against::RealStack, "scenarios (real stack)")?;
        report.add(check);
        report.blame(failures);
    }

    report.add(corpus(root)?);
    if matches!(tier, Tier::Full | Tier::Nightly) {
        let (check, ran) = fresh_seeds(tier);
        report.add(check);
        report.ran(ran);
    }

    report.add(specification(root)?);
    report.add(requirement_matrix(root)?);
    report.record(requirement_summary(root)?);

    if matches!(tier, Tier::Nightly) {
        report.add(mutation(root)?);
        report.add(CheckResult::from_outcome(
            "self-test",
            selftest::run(root)?,
            "the harness did not notice a desktop that was wrong",
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

/// Whether the suite still detects a wrong core. `AGENTS.md` asks for this on the modules a
/// change touched; the nightly tier asks it of all of them.
fn mutation(root: &Path) -> Result<CheckResult, String> {
    if !tools::cargo_subcommand_available("mutants") {
        return Ok(CheckResult::skipped(
            "mutation",
            "cargo-mutants is not installed, run `cargo install cargo-mutants`",
        ));
    }

    let passed = mutants::run(root, None)?;
    Ok(CheckResult::from_outcome(
        "mutation",
        passed,
        "the suite did not detect enough wrong implementations",
    ))
}

fn scenarios(
    root: &Path,
    against: scenario::Against,
    name: &str,
) -> Result<(CheckResult, Vec<Failure>), String> {
    let outcomes = scenario::run_all(&workspace::scenarios_directory(root), against)?;
    if outcomes.is_empty() {
        return Ok((
            CheckResult::skipped(name, "none are written yet, see docs/VERIFICATION.md"),
            Vec::new(),
        ));
    }

    let checked: Vec<&scenario::Outcome> = outcomes
        .iter()
        .filter(|outcome| outcome.was_checked())
        .collect();
    let failed: Vec<&scenario::Outcome> = checked
        .iter()
        .filter(|outcome| !outcome.passed())
        .copied()
        .collect();
    for outcome in &failed {
        scenario::print_failure(outcome);
    }
    for outcome in outcomes.iter().filter(|outcome| !outcome.was_checked()) {
        if let Some(reason) = &outcome.skipped {
            println!("  {} is not checked here: {reason}", outcome.id);
        }
    }

    println!(
        "{name}: {} run, {} elsewhere, {} failed",
        checked.len(),
        outcomes.len() - checked.len(),
        failed.len()
    );
    let check = CheckResult::from_outcome(
        name,
        failed.is_empty(),
        "an acceptance scenario did not hold",
    );
    Ok((check, blame(root, &failed)?))
}

/// Turns failing scenarios into entries an agent can act on: the requirement each one puts in
/// doubt, where that requirement lives in the specification, and one command to see it again.
fn blame(root: &Path, failed: &[&scenario::Outcome]) -> Result<Vec<Failure>, String> {
    let registry = Registry::load(&workspace::requirements_file(root))?;
    let sections: BTreeMap<&str, &str> = registry
        .requirements
        .iter()
        .map(|item| (item.id.as_str(), item.section.as_str()))
        .collect();

    let mut blamed = Vec::new();
    for outcome in failed {
        let diff = outcome.failures.join("; ");
        for requirement in &outcome.covers {
            blamed.push(Failure {
                test: outcome.id.clone(),
                requirement: requirement.clone(),
                spec: sections
                    .get(requirement.as_str())
                    .map_or_else(|| "unknown".to_string(), ToString::to_string),
                repro: format!("./verify scenario {}", outcome.id),
                diff: diff.clone(),
            });
        }
    }
    Ok(blamed)
}

fn requirement_summary(root: &Path) -> Result<Requirements, String> {
    let matrix = coverage::build(root)?;
    let unverified: Vec<String> = matrix
        .unclaimed()
        .iter()
        .map(|requirement| requirement.id.clone())
        .collect();

    Ok(Requirements {
        active: matrix.active.len(),
        verified: matrix.active.len() - unverified.len(),
        unverified,
    })
}

/// Every seed that has ever failed, replayed. `AGENTS.md` commits each beside its fix, and
/// the fastest tier runs them from then on so a fixed bug stays fixed.
fn corpus(root: &Path) -> Result<CheckResult, String> {
    let outcome = campaign::replay_corpus(&workspace::corpus_directory(root))?;
    if outcome.seeds == 0 {
        return Ok(CheckResult::skipped(
            "corpus",
            "no seed has ever failed, so there is nothing to replay",
        ));
    }

    campaign::print_failures(&outcome);
    println!("corpus: {} seeds replayed", outcome.seeds);
    Ok(CheckResult::from_outcome(
        "corpus",
        outcome.passed(),
        "a seed that was fixed has come back",
    ))
}

/// Sessions nobody has tried yet.
fn fresh_seeds(tier: Tier) -> (CheckResult, Campaign) {
    let seeds = match tier {
        Tier::Nightly => NIGHTLY_SEEDS,
        _ => CAMPAIGN_SEEDS,
    };
    let outcome = campaign::run(seeds);

    campaign::print_failures(&outcome);
    println!(
        "campaign: {} seeds, {} failed",
        outcome.seeds,
        outcome.failures.len()
    );
    let check = CheckResult::from_outcome(
        "campaign",
        outcome.passed(),
        "a made-up session ended somewhere the model did not expect",
    );
    let ran = Campaign {
        seeds: outcome.seeds,
        failed_seeds: outcome
            .failures
            .iter()
            .map(|failure| format!("{:#018x}", failure.seed))
            .collect(),
    };
    (check, ran)
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

    if let [only] = touched.as_slice()
        && only.path == REGISTRY
        && let Some(before) = registry_at(root, against, REGISTRY)
        && let Ok(after) = std::fs::read_to_string(workspace::requirements_file(root))
        && only_activates(&before, &after)
    {
        println!("protected-artifacts: the registry only turned requirements on");
        return Ok(true);
    }

    // `docs/VERIFICATION.md` §L6 says the hook and the nightly audit *flag* a change like
    // this. Refusing it outright would forbid writing an oracle at all, since building one
    // touches the same paths as bending one; what tells those apart is a person reading the
    // diff. So this says so as loudly as it can and leaves the judgement where it belongs.
    println!();
    println!("{BANNER}");
    println!(
        "protected-artifacts: {} of these define what correct means, and",
        touched.len()
    );
    println!("changed without {SPECIFICATION} changing beside them:");
    for change in touched {
        println!("    {}", change.path);
    }
    println!();
    println!("A reviewer has to decide whether this is building an oracle or bending one.");
    println!("An oracle edited until it agrees with the implementation has stopped being one.");
    println!("{BANNER}");
    println!();
    Ok(true)
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

/// Whether a registry change only turns requirements on.
///
/// Closing a milestone flips requirements from deferred to active, and `docs/ROADMAP.md` says
/// so. That direction can only make the build stricter: an active requirement without a
/// passing test fails the tier. The gate exists to stop an oracle being weakened into
/// agreement with the implementation, and this is the opposite, so it is allowed on its own.
/// Every other edit to the registry, deferring one included, still wants a specification
/// change beside it.
fn only_activates(before: &str, after: &str) -> bool {
    let (Ok(before), Ok(after)) = (Registry::read(before), Registry::read(after)) else {
        return false;
    };
    if before.requirements.len() != after.requirements.len() {
        return false;
    }

    before
        .requirements
        .iter()
        .zip(after.requirements.iter())
        .all(|(was, now)| {
            was.id == now.id
                && was.section == now.section
                && was.area == now.area
                && was.quote == now.quote
                && was.note == now.note
                && matches!(
                    (was.status, now.status),
                    (Status::Deferred, Status::Deferred)
                        | (Status::Active, Status::Active)
                        | (Status::Deferred, Status::Active)
                )
        })
}

/// The registry as it stood at a reference, or nothing if it cannot be read there.
fn registry_at(root: &Path, reference: &str, path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["show", &format!("{reference}:{path}")])
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
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

    fn registry(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(id, status)| {
                format!(
                    "[[requirement]]\nid = \"{id}\"\nsection = \"§1\"\narea = \"XFER\"\n\
                     status = \"{status}\"\nquote = \"a clause\"\n"
                )
            })
            .collect()
    }

    #[test]
    fn turning_a_requirement_on_is_allowed_on_its_own() {
        let before = registry(&[("R-XFER-001", "deferred"), ("R-XFER-002", "deferred")]);
        let after = registry(&[("R-XFER-001", "active"), ("R-XFER-002", "deferred")]);

        assert!(only_activates(&before, &after));
    }

    #[test]
    fn turning_one_off_again_is_not() {
        let before = registry(&[("R-XFER-001", "active")]);
        let after = registry(&[("R-XFER-001", "deferred")]);

        assert!(!only_activates(&before, &after));
    }

    #[test]
    fn rewording_a_requirement_is_not() {
        let before = registry(&[("R-XFER-001", "deferred")]);
        let after = before.replace("a clause", "a different clause");

        assert!(!only_activates(&before, &after));
    }

    #[test]
    fn adding_a_requirement_is_not() {
        let before = registry(&[("R-XFER-001", "deferred")]);
        let after = registry(&[("R-XFER-001", "deferred"), ("R-XFER-002", "deferred")]);

        assert!(!only_activates(&before, &after));
    }

    #[test]
    fn removing_one_is_not() {
        let before = registry(&[("R-XFER-001", "deferred"), ("R-XFER-002", "deferred")]);
        let after = registry(&[("R-XFER-001", "deferred")]);

        assert!(!only_activates(&before, &after));
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
