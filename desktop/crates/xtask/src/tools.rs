//! Running external tools, and noticing when one is not installed.
//!
//! A missing tool is reported and skipped rather than failing the tier, so work can start
//! before every tool exists on a machine.

use std::path::Path;
use std::process::Command;

pub fn cargo_subcommand_available(subcommand: &str) -> bool {
    Command::new("cargo")
        .args([subcommand, "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn run(program: &str, arguments: &[&str], directory: &Path) -> Result<bool, String> {
    let status = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .status()
        .map_err(|error| format!("cannot run {program}: {error}"))?;

    Ok(status.success())
}
