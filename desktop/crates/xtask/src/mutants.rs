//! Mutation testing: does the suite notice a wrong implementation?
//!
//! `AGENTS.md` makes a score of 0.85 on the changed core modules part of being done, so the
//! command that measures it has to be the same one every time. Two details make the naive
//! invocation lie, and encoding them here is the whole point of this module.
//!
//! The first is that the tests which kill a core mutant no longer live in the core. They
//! drive it through the simulator, so the run has to include that package or almost every
//! mutant survives. The second is that the protobuf crate's build script reads the schema
//! from outside the Cargo workspace, and `cargo-mutants` works in a copy of the workspace
//! alone, so a run that builds every package fails before it starts.

use std::path::Path;

use crate::tools;

/// The score `AGENTS.md` requires of the core modules a change touched.
const REQUIRED: f64 = 0.85;

/// Packages whose tests exercise the core.
const EXERCISED_BY: [&str; 2] = ["photo-sync-core", "photo-sync-sim"];

pub(crate) fn run(root: &Path, module: Option<&str>) -> Result<bool, String> {
    if !tools::cargo_subcommand_available("mutants") {
        println!("mutants: cargo-mutants is not installed, run `cargo install cargo-mutants`");
        return Ok(true);
    }

    let desktop = root.join("desktop");
    let mut arguments = vec![
        "mutants".to_string(),
        "--package".to_string(),
        "photo-sync-core".to_string(),
    ];
    for package in EXERCISED_BY {
        arguments.push("--test-package".to_string());
        arguments.push(package.to_string());
    }

    if let Some(module) = module {
        arguments.push("--file".to_string());
        arguments.push(file_of(module));
    }

    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    // The exit status alone does not say what happened: cargo-mutants reports a timeout the
    // same way it reports a survivor, and the two mean opposite things.
    let _ = tools::run("cargo", &borrowed, &desktop)?;
    report(&desktop.join("mutants.out"))
}

/// Reads what the run found and decides whether it meets the gate.
///
/// A mutant that made the suite hang was detected, however unpleasantly, so it counts with
/// the caught. A mutant nothing noticed is a survivor, and `AGENTS.md` wants each of those
/// killed or written down with the reason.
fn report(output: &Path) -> Result<bool, String> {
    let caught = counted(&output.join("caught.txt"));
    let missed = listed(&output.join("missed.txt"));
    let timed_out = counted(&output.join("timeout.txt"));

    let detected = caught + timed_out;
    let viable = detected + missed.len();
    if viable == 0 {
        return Err("mutants: nothing was testable, see desktop/mutants.out/log".to_string());
    }

    #[allow(clippy::cast_precision_loss)]
    let score = detected as f64 / viable as f64;
    println!(
        "mutants: {detected} of {viable} detected ({caught} caught, {timed_out} by hanging), \
         score {score:.3}"
    );

    for survivor in &missed {
        println!("  survived: {survivor}");
    }
    if !missed.is_empty() {
        println!(
            "  each of these has to be killed, or recorded with its reason in \
             verification/mutants-allow.toml"
        );
    }

    if score < REQUIRED {
        println!("mutants: below the {REQUIRED} the gate in AGENTS.md asks for");
        return Ok(false);
    }
    Ok(true)
}

fn counted(path: &Path) -> usize {
    listed(path).len()
}

fn listed(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The file a module name stands for. `core::desktop::commit` is
/// `crates/core/src/desktop/commit.rs`, and a path is taken as written.
fn file_of(module: &str) -> String {
    if module.contains('/') {
        return module.to_string();
    }

    let mut parts = module.split("::");
    let crate_name = parts.next().unwrap_or("core");
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        return format!("crates/{crate_name}/src/**/*.rs");
    }
    format!("crates/{crate_name}/src/{}.rs", rest.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_path_names_the_file_it_lives_in() {
        assert_eq!(file_of("core::commit"), "crates/core/src/commit.rs");
    }

    #[test]
    fn a_nested_module_names_a_nested_file() {
        assert_eq!(
            file_of("core::desktop::commit"),
            "crates/core/src/desktop/commit.rs"
        );
    }

    #[test]
    fn a_crate_on_its_own_means_all_of_it() {
        assert_eq!(file_of("core"), "crates/core/src/**/*.rs");
    }

    #[test]
    fn a_path_is_taken_as_it_was_written() {
        assert_eq!(
            file_of("crates/core/src/naming.rs"),
            "crates/core/src/naming.rs"
        );
    }
}
