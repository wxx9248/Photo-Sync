//! The verification runner.
//!
//! One entry point for every tier, so a person, a hook, and a build server all invoke the same
//! checks. See `docs/VERIFICATION.md`.

mod checks;
mod cli;
mod coverage;
mod doctor;
mod report;
mod requirements;
mod scenario;
mod spec_check;
mod tools;
mod workspace;

use std::process::ExitCode;

use cli::Command;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let command = match cli::parse(&arguments) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}\n\n{}", cli::usage());
            return ExitCode::from(2);
        }
    };

    match run(command) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether the requested work passed.
fn run(command: Command) -> Result<bool, String> {
    let root = workspace::repository_root()?;

    match command {
        Command::Verify(tier) => checks::run_tier(&root, tier),
        Command::SpecCheck => spec_check::run(&root).map(|outcome| outcome.passed()),
        Command::Matrix => coverage::print_matrix(&root),
        Command::Doctor => doctor::run(&root),
        Command::Scenario { id } => {
            let outcome = scenario::run_one(&workspace::scenarios_directory(&root), &id)?;
            report_scenario(&outcome);
            Ok(outcome.passed())
        }
        Command::ProtectedArtifacts { against } => checks::protected_artifacts(&root, &against),
        Command::NotYetBuilt { name, milestone } => Err(format!(
            "`{name}` arrives in milestone {milestone}. See docs/ROADMAP.md."
        )),
    }
}

fn report_scenario(outcome: &scenario::Outcome) {
    if outcome.passed() {
        println!("{}: passed — {}", outcome.id, outcome.description);
        return;
    }
    scenario::print_failure(outcome);
}
