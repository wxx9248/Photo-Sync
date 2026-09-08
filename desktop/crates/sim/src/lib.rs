//! The deterministic simulator.
//!
//! A seed and a step budget reproduce a run exactly. The simulator owns a virtual clock, a
//! filesystem that models durability the way real storage does, a network that drops and
//! reorders, and the ability to stop the process at any effect boundary.
//!
//! Scope today is the seed and the run identity that failures are reported with. The
//! filesystem, the SQLite virtual file system, and the scenario generator arrive in milestone
//! M2, described in `docs/ROADMAP.md`.

/// Identifies one simulator run. Printed with every failure so the run can be replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seed(pub u64);

impl Seed {
    pub fn to_token(self) -> String {
        format!("{:#018x}", self.0)
    }

    pub fn parse(token: &str) -> Option<Self> {
        let digits = token.strip_prefix("0x").unwrap_or(token);
        u64::from_str_radix(digits, 16).ok().map(Seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_token_round_trips() {
        let seed = Seed(0x8f31_c0a9_4b2e);

        let parsed = Seed::parse(&seed.to_token());

        assert_eq!(parsed, Some(seed));
    }
}
