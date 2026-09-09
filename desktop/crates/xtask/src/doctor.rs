//! Checks whether a machine can run the verification tiers.
//!
//! A fresh machine is the normal starting point, so the answer has to say which tiers work
//! right now and give the exact command that fixes each gap.

use std::path::Path;

use crate::tools;

/// What a missing tool costs. Ordered so the earliest tier appears first in the summary.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Need {
    Quick,
    Full,
    Nightly,
    Android,

    /// Not a tier. Repository configuration that a fresh clone needs once.
    Setup,
}

impl Need {
    fn name(self) -> &'static str {
        match self {
            Need::Quick => "quick",
            Need::Full => "full",
            Need::Nightly => "nightly",
            Need::Android => "android",
            Need::Setup => "setup",
        }
    }
}

struct Check {
    name: &'static str,
    need: Need,
    found: Option<String>,
    fix: &'static str,
}

impl Check {
    fn is_satisfied(&self) -> bool {
        self.found.is_some()
    }
}

pub(crate) fn run(root: &Path) -> Result<bool, String> {
    let checks = collect(root);
    report(&checks);

    // A missing tool never fails this command. It reports what works and what does not, and the
    // caller decides whether the tier they wanted is one of them.
    Ok(checks
        .iter()
        .filter(|check| check.need == Need::Quick)
        .all(Check::is_satisfied))
}

fn collect(root: &Path) -> Vec<Check> {
    vec![
        tool(
            "rustc",
            Need::Quick,
            "install rust, see docs/DEVELOPMENT.md",
        ),
        tool(
            "cargo",
            Need::Quick,
            "install rust, see docs/DEVELOPMENT.md",
        ),
        subcommand(
            "fmt",
            "rustfmt",
            Need::Quick,
            "rustup component add rustfmt",
        ),
        subcommand(
            "clippy",
            "clippy",
            Need::Quick,
            "rustup component add clippy",
        ),
        tool("protoc", Need::Quick, "install the protobuf compiler"),
        subcommand(
            "nextest",
            "cargo-nextest",
            Need::Full,
            "cargo install cargo-nextest",
        ),
        subcommand(
            "mutants",
            "cargo-mutants",
            Need::Nightly,
            "cargo install cargo-mutants",
        ),
        subcommand(
            "deny",
            "cargo-deny",
            Need::Nightly,
            "cargo install cargo-deny",
        ),
        subcommand(
            "audit",
            "cargo-audit",
            Need::Nightly,
            "cargo install cargo-audit",
        ),
        tool(
            "java",
            Need::Android,
            "install a JDK 21, see docs/DEVELOPMENT.md",
        ),
        android_sdk(),
        hooks(root),
    ]
}

fn tool(program: &'static str, need: Need, fix: &'static str) -> Check {
    Check {
        name: program,
        need,
        found: tools::version(program, &["--version"]),
        fix,
    }
}

fn subcommand(
    subcommand: &'static str,
    name: &'static str,
    need: Need,
    fix: &'static str,
) -> Check {
    Check {
        name,
        need,
        found: tools::version("cargo", &[subcommand, "--version"]),
        fix,
    }
}

fn android_sdk() -> Check {
    let found = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .find_map(|variable| std::env::var(variable).ok())
        .filter(|path| Path::new(path).is_dir());

    Check {
        name: "android sdk",
        need: Need::Android,
        found,
        fix: "install the command line tools and set ANDROID_HOME",
    }
}

fn hooks(root: &Path) -> Check {
    let configured = tools::capture("git", &["config", "core.hooksPath"], root)
        .filter(|value| value.trim() == "hooks");

    Check {
        name: "git hooks",
        need: Need::Setup,
        found: configured,
        fix: "git config core.hooksPath hooks",
    }
}

fn report(checks: &[Check]) {
    let (setup, tools): (Vec<&Check>, Vec<&Check>) =
        checks.iter().partition(|check| check.need == Need::Setup);

    println!("environment");
    print_lines(&tools);

    println!("\nrepository");
    print_lines(&setup);

    print_summary(&tools);
}

fn print_lines(checks: &[&Check]) {
    for check in checks {
        match &check.found {
            Some(detail) => println!("  {:<16} ok       {}", check.name, shorten(detail)),
            None => println!("  {:<16} missing  {}", check.name, check.fix),
        }
    }
}

fn print_summary(tools: &[&Check]) {
    let mut blocked: Vec<&str> = tools
        .iter()
        .filter(|check| !check.is_satisfied())
        .map(|check| check.need.name())
        .collect();
    blocked.sort_unstable();
    blocked.dedup();

    if blocked.is_empty() {
        println!("\nevery tier can run here");
        return;
    }

    println!("\nnot ready for: {}", blocked.join(", "));
    println!("A missing tool is skipped rather than failed, so the other tiers still run.");
}

/// Version output is often several lines. The first is the one worth showing.
fn shorten(detail: &str) -> &str {
    detail.lines().next().unwrap_or(detail).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_line_of_a_version_is_shown() {
        assert_eq!(shorten("cargo 1.98.0\nextra noise"), "cargo 1.98.0");
    }

    #[test]
    fn needs_order_from_the_earliest_tier() {
        assert!(Need::Quick < Need::Full);
        assert!(Need::Full < Need::Nightly);
    }
}
