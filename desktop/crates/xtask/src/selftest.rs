//! Negative controls: does the harness notice a desktop that is wrong?
//!
//! Every other check asks whether the implementation is right. This asks whether the
//! instrument works at all. `docs/VERIFICATION.md` §L6 puts it plainly: a harness that cannot
//! demonstrate it detects a known-bad implementation has no standing to certify anything.
//!
//! Each sabotage is a rule `SPEC.md` states, broken on purpose behind a feature nothing else
//! turns on. A run that passes with one of these enabled is a run that was never testing that
//! rule, and the set grows with every real bug found.

use std::path::Path;
use std::process::Command;

/// How a wrong world is arranged.
enum Arrange {
    /// A deliberately wrong desktop, built behind a feature.
    Sabotage(&'static str),

    /// A world the desktop cannot be right in. Nothing is expected to cope; what is being
    /// checked is that the suite notices.
    Lie(&'static str),
}

/// One wrong world, and the rule it breaks.
struct Control {
    how: Arrange,
    breaks: &'static str,
}

const CONTROLS: [Control; 10] = [
    Control {
        how: Arrange::Sabotage("sabotage-donemark"),
        breaks: "§7.3: a done-mark written before the directories are durable",
    },
    Control {
        how: Arrange::Sabotage("sabotage-watermark"),
        breaks: "§7.6: a partial trusted at its length rather than its watermark",
    },
    Control {
        how: Arrange::Sabotage("sabotage-dedup"),
        breaks: "§7.3: a duplicate earning its phone no index row",
    },
    Control {
        how: Arrange::Sabotage("sabotage-nomination"),
        breaks: "§8: nomination trusting the index without looking at the vault",
    },
    Control {
        how: Arrange::Sabotage("sabotage-seal"),
        breaks: "§7.3: a rename made before the write-log was sealed",
    },
    Control {
        how: Arrange::Sabotage("sabotage-rowsfirst"),
        breaks: "§7.3: staging cleared before the index rows were durable",
    },
    Control {
        how: Arrange::Sabotage("sabotage-clearorder"),
        breaks: "§7.3: the manifest cleared before the write-log",
    },
    Control {
        how: Arrange::Sabotage("sabotage-halt"),
        breaks: "§7.3: a commit carrying on past a rename it could not make",
    },
    Control {
        how: Arrange::Sabotage("sabotage-lockrelease"),
        breaks: "§7.3: index rows going in after the commit lock was released",
    },
    Control {
        how: Arrange::Lie("PHOTO_SYNC_LYING_FSYNC"),
        breaks: "§L1: a filesystem that says it synced and did not",
    },
];

/// Arranges a wrong world five times over and insists the suite objects each time.
///
/// # Errors
/// When the tests cannot be run at all.
pub(crate) fn run(root: &Path) -> Result<bool, String> {
    let desktop = root.join("desktop");
    let mut all_caught = true;

    let mut caught_count = 0;
    for control in &CONTROLS {
        match caught_by(&desktop, &control.how)? {
            Some(test) => {
                caught_count += 1;
                println!("  {} caught by {test}", control.breaks);
            }
            None => {
                all_caught = false;
                println!("  {} WENT UNNOTICED", control.breaks);
            }
        }
    }

    println!(
        "self-test: {caught_count} of {} wrong worlds were noticed",
        CONTROLS.len()
    );
    Ok(all_caught)
}

/// Runs the suite against one sabotaged desktop and names the first test that objected.
fn caught_by(desktop: &Path, how: &Arrange) -> Result<Option<String>, String> {
    let mut command = Command::new("cargo");
    command.arg("test").arg("--workspace").current_dir(desktop);
    match how {
        Arrange::Sabotage(feature) => {
            command
                .arg("--features")
                .arg(format!("photo-sync-core/{feature}"));
        }
        Arrange::Lie(variable) => {
            command.env(variable, "1");
        }
    }

    let output = command
        .output()
        .map_err(|error| format!("cannot run the tests: {error}"))?;

    // A sabotaged desktop should fail; a clean exit means nothing was watching this rule.
    if output.status.success() {
        return Ok(None);
    }
    Ok(first_failing_test(&String::from_utf8_lossy(&output.stdout)))
}

/// The name of the first test that objected, out of a test runner's own output.
fn first_failing_test(output: &str) -> Option<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("test "))
        .find_map(|rest| rest.strip_suffix(" ... FAILED"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_objection_is_the_one_reported() {
        let output = "test one_thing ... ok\ntest another ... FAILED\ntest third ... FAILED\n";

        assert_eq!(first_failing_test(output), Some("another".to_string()));
    }

    #[test]
    fn a_run_where_nothing_objected_names_nothing() {
        assert_eq!(first_failing_test("test one_thing ... ok\n"), None);
    }

    #[test]
    fn every_control_says_which_rule_it_breaks() {
        for control in &CONTROLS {
            assert!(control.breaks.starts_with('§'), "{}", control.breaks);
        }
    }
}
