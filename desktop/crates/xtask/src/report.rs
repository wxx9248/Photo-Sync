//! The JSON report, which is what an agent reads to decide what to do next.

use std::path::Path;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CheckResult {
    pub name: String,
    pub status: Status,

    /// What to do about a failure, or why a check was skipped.
    pub detail: String,
}

impl CheckResult {
    pub(crate) fn from_outcome(name: &str, passed: bool, failure_detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: if passed {
                Status::Passed
            } else {
                Status::Failed
            },
            detail: if passed {
                String::new()
            } else {
                failure_detail.to_string()
            },
        }
    }

    pub(crate) fn skipped(name: &str, reason: &str) -> Self {
        Self {
            name: name.to_string(),
            status: Status::Skipped,
            detail: reason.to_string(),
        }
    }
}

/// How much of the specification is being verified, and what is not.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Requirements {
    pub active: usize,

    /// Active requirements a passing test claims. The tier is only green when every test
    /// passed, so in a green run this is the number actually verified.
    pub verified: usize,

    pub unverified: Vec<String>,
}

/// One thing that went wrong, with everything needed to act on it.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Failure {
    /// The scenario or test that failed.
    pub test: String,

    /// The requirement it puts in doubt.
    pub requirement: String,

    /// Where that requirement lives in `SPEC.md`.
    pub spec: String,

    /// One command that reproduces this exactly.
    pub repro: String,

    /// What differed.
    pub diff: String,
}

/// What a run of made-up sessions found.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Campaign {
    pub seeds: usize,
    pub failed_seeds: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Report {
    pub tier: String,
    pub started: u64,
    pub duration_s: u64,
    pub result: Status,
    pub requirements: Requirements,
    pub checks: Vec<CheckResult>,
    pub failures: Vec<Failure>,

    /// Absent until a tier that runs one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign: Option<Campaign>,
}

impl Report {
    pub(crate) fn started(tier: &str) -> Self {
        Self {
            tier: tier.to_string(),
            started: seconds_since_epoch(),
            duration_s: 0,
            result: Status::Passed,
            requirements: Requirements::default(),
            checks: Vec::new(),
            failures: Vec::new(),
            campaign: None,
        }
    }

    pub(crate) fn ran(&mut self, campaign: Campaign) {
        self.campaign = Some(campaign);
    }

    pub(crate) fn record(&mut self, requirements: Requirements) {
        self.requirements = requirements;
    }

    pub(crate) fn blame(&mut self, failures: Vec<Failure>) {
        self.failures.extend(failures);
    }

    pub(crate) fn add(&mut self, check: CheckResult) {
        if check.status == Status::Failed {
            self.result = Status::Failed;
        }
        self.checks.push(check);
    }

    pub(crate) fn passed(&self) -> bool {
        self.result != Status::Failed
    }

    pub(crate) fn finish(&mut self, directory: &Path) -> Result<(), String> {
        self.duration_s = seconds_since_epoch().saturating_sub(self.started);

        std::fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;

        let path = directory.join("latest.json");
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("cannot render the report: {error}"))?;
        std::fs::write(&path, text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        self.print_summary(&path);
        Ok(())
    }

    fn print_summary(&self, path: &Path) {
        let failed: Vec<&CheckResult> = self
            .checks
            .iter()
            .filter(|check| check.status == Status::Failed)
            .collect();

        println!(
            "\n{} tier: {}",
            self.tier,
            if self.passed() { "passed" } else { "failed" }
        );
        for check in &failed {
            println!("  {} failed: {}", check.name, check.detail);
        }
        for failure in &self.failures {
            println!(
                "  {} violates {} ({}): {}",
                failure.test, failure.requirement, failure.spec, failure.diff
            );
            println!("    reproduce with: {}", failure.repro);
        }
        println!("report written to {}", path.display());
    }
}

/// The report records when a run happened. The runner is not the core, so it may ask.
fn seconds_since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}
