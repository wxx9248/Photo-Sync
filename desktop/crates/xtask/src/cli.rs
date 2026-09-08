//! Argument parsing. Kept by hand because the surface is small and stable.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Verify(Tier),
    SpecCheck,
    Matrix,
    ProtectedArtifacts { against: String },
    NotYetBuilt { name: String, milestone: String },
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
        ["verify", "protected-artifacts", "--against", reference]
        | ["protected-artifacts", "--against", reference] => Ok(Command::ProtectedArtifacts {
            against: (*reference).to_string(),
        }),
        [
            "verify",
            name @ ("replay" | "scenario" | "mutants" | "self-test"),
            ..,
        ] => Ok(Command::NotYetBuilt {
            name: (*name).to_string(),
            milestone: "M2".to_string(),
        }),
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
