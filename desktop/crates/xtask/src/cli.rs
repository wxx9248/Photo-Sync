//! Argument parsing. Kept by hand because the surface is small and stable.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Verify(Tier),
    SpecCheck,
    Matrix,
    Doctor,
    Scenario { id: String, real: bool },
    Mutants { module: Option<String> },
    Replay { seed: String },
    SelfTest,
    ProtectedArtifacts { against: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Quick,
    Full,
    Nightly,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Quick => "quick",
            Tier::Full => "full",
            Tier::Nightly => "nightly",
        }
    }
}

pub fn parse(arguments: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();

    match words.as_slice() {
        ["verify"] | [] => Ok(Command::Verify(Tier::Quick)),
        ["verify", "quick"] => Ok(Command::Verify(Tier::Quick)),
        ["verify", "full"] => Ok(Command::Verify(Tier::Full)),
        ["verify", "nightly"] => Ok(Command::Verify(Tier::Nightly)),
        ["verify", "spec-check"] | ["spec-check"] => Ok(Command::SpecCheck),
        ["verify", "matrix"] | ["matrix"] => Ok(Command::Matrix),
        ["verify", "doctor"] | ["doctor"] => Ok(Command::Doctor),
        ["verify", "protected-artifacts", "--against", reference]
        | ["protected-artifacts", "--against", reference] => Ok(Command::ProtectedArtifacts {
            against: (*reference).to_string(),
        }),
        ["verify", "scenario", id] | ["scenario", id] => Ok(Command::Scenario {
            id: (*id).to_string(),
            real: false,
        }),
        ["verify", "scenario", id, "--real"] | ["scenario", id, "--real"] => {
            Ok(Command::Scenario {
                id: (*id).to_string(),
                real: true,
            })
        }
        ["verify", "mutants"] | ["mutants"] => Ok(Command::Mutants { module: None }),
        ["verify", "mutants", module] | ["mutants", module] => Ok(Command::Mutants {
            module: Some((*module).to_string()),
        }),
        ["verify", "replay", seed] | ["replay", seed] => Ok(Command::Replay {
            seed: (*seed).to_string(),
        }),
        ["verify", "self-test"] | ["self-test"] => Ok(Command::SelfTest),
        _ => Err(format!("unrecognised arguments: {}", words.join(" "))),
    }
}

pub fn usage() -> String {
    [
        "usage: ./verify <tier|command>",
        "",
        "  quick                          fast checks, run constantly while working",
        "  full                           quick plus the campaign and cross-language tests",
        "  nightly                        full plus mutation, emulator, and audits",
        "  spec-check                     registry quotes still match SPEC.md",
        "  matrix                         requirement coverage",
        "  doctor                         whether this machine can run each tier",
        "  scenario <id> [--real]         one acceptance scenario, simulated or on the real stack",
        "  mutants [module]               does the suite detect a wrong implementation?",
        "  replay <seed>                  one campaign run, exactly as it happened",
        "  self-test                      does the harness notice a desktop that is wrong?",
        "  protected-artifacts --against <ref>",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_invocation_runs_the_quick_tier() {
        assert_eq!(parse(&[]), Ok(Command::Verify(Tier::Quick)));
    }

    #[test]
    fn commands_are_reachable_with_and_without_the_verify_word() {
        let with = parse(&["verify".into(), "matrix".into()]);
        let without = parse(&["matrix".into()]);

        assert_eq!(with, without);
    }

    #[test]
    fn an_unknown_command_is_an_error_rather_than_a_default() {
        assert!(parse(&["polish".into()]).is_err());
    }
}
