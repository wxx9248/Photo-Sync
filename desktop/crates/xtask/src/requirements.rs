//! The requirement registry, which is the machine-readable projection of the specification.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Registry {
    #[serde(default, rename = "requirement")]
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub section: String,
    pub area: String,
    pub status: Status,
    pub quote: String,

    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Verification is expected now. An active requirement without a passing test fails the
    /// full tier.
    Active,

    /// Not yet claimed by a milestone. Deferring is visible in the registry diff.
    Deferred,
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Ok(Registry {
                requirements: Vec::new(),
            });
        }

        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

        toml::from_str(&text).map_err(|error| format!("cannot parse {}: {error}", path.display()))
    }

    pub fn active(&self) -> impl Iterator<Item = &Requirement> {
        self.requirements
            .iter()
            .filter(|item| item.status == Status::Active)
    }
}
