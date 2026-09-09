//! Locating the repository from wherever the runner was invoked.

use std::path::{Path, PathBuf};

pub(crate) fn repository_root() -> Result<PathBuf, String> {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));

    for candidate in start.ancestors() {
        if candidate.join("docs/SPEC.md").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }

    Err("could not find the repository root: docs/SPEC.md is missing".to_string())
}

pub(crate) fn requirements_file(root: &Path) -> PathBuf {
    root.join("verification/requirements.toml")
}

pub(crate) fn spec_file(root: &Path) -> PathBuf {
    root.join("docs/SPEC.md")
}

pub(crate) fn reports_directory(root: &Path) -> PathBuf {
    root.join("verification/reports")
}

pub(crate) fn corpus_directory(root: &Path) -> PathBuf {
    root.join("verification/corpus")
}

pub(crate) fn scenarios_directory(root: &Path) -> PathBuf {
    root.join("verification/scenarios")
}
