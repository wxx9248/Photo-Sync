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

/// One deliberately wrong desktop, and the rule it breaks.
struct Sabotage {
    feature: &'static str,
    breaks: &'static str,
}

const SABOTAGES: [Sabotage; 4] = [
    Sabotage {
        feature: "sabotage-donemark",
        breaks: "§7.3: a done-mark written before the directories are durable",
    },
    Sabotage {
        feature: "sabotage-watermark",
        breaks: "§7.6: a partial trusted at its length rather than its watermark",
    },
    Sabotage {
        feature: "sabotage-dedup",
        breaks: "§7.3: a duplicate earning its phone no index row",
    },
    Sabotage {
        feature: "sabotage-nomination",
        breaks: "§8: nomination trusting the index without looking at the vault",
    },
];

/// Builds a wrong desktop four times over and insists the suite objects each time.
///
/// # Errors
/// When the tests cannot be run at all.
pub fn run(root: &Path) -> Result<bool, String> {
    let desktop = root.join("desktop");
    let mut all_caught = true;

    for sabotage in &SABOTAGES {
        let caught = caught_by(&desktop, sabotage.feature)?;
        match caught {
            Some(test) => println!("  {} caught by {test}", sabotage.breaks),
            None => {
                all_caught = false;
                println!("  {} WENT UNNOTICED", sabotage.breaks);
            }
        }
    }

    println!(
        "self-test: {} of {} known-bad desktops were caught",
        SABOTAGES.iter().len() - usize::from(!all_caught),
        SABOTAGES.len()
    );
    Ok(all_caught)
}

/// Runs the suite against one sabotaged desktop and names the first test that objected.
fn caught_by(desktop: &Path, feature: &str) -> Result<Option<String>, String> {
    let output = Command::new("cargo")
        .args([
            "test",
            "--workspace",
            "--features",
            &format!("photo-sync-core/{feature}"),
        ])
        .current_dir(desktop)
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
    fn every_sabotage_says_which_rule_it_breaks() {
        for sabotage in &SABOTAGES {
            assert!(sabotage.breaks.starts_with('§'), "{}", sabotage.feature);
        }
    }
}
