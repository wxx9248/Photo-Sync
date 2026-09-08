//! The JSON report, which is what an agent reads to decide what to do next.

use std::path::Path;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub status: Status,

    /// What to do about a failure, or why a check was skipped.
    pub detail: String,
}

impl CheckResult {
    pub fn from_outcome(name: &str, passed: bool, failure_detail: &str) -> Self {
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

    pub fn skipped(name: &str, reason: &str) -> Self {
        Self {
            name: name.to_string(),
            status: Status::Skipped,
            detail: reason.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub tier: String,
    pub started: u64,
    pub checks: Vec<CheckResult>,
    pub result: Status,
}

impl Report {
    pub fn started(tier: &str) -> Self {
        Self {
            tier: tier.to_string(),
            started: seconds_since_epoch(),
            checks: Vec::new(),
            result: Status::Passed,
        }
    }

    pub fn add(&mut self, check: CheckResult) {
        if check.status == Status::Failed {
            self.result = Status::Failed;
        }
        self.checks.push(check);
    }

    pub fn passed(&self) -> bool {
        self.result != Status::Failed
    }

    pub fn finish(&self, directory: &Path) -> Result<(), String> {
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
        println!("report written to {}", path.display());
    }
}

// The report records when a run happened, which is the one place wall-clock time belongs.
#[allow(clippy::disallowed_types)]
fn seconds_since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}
