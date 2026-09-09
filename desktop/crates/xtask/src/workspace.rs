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

/// Where the Android SDK is, if this machine has one.
///
/// The application module needs it and the session module does not, so it is asked about
/// rather than assumed. A machine that told Gradle through `local.properties` counts as
/// having one, because that is where Gradle itself looks.
pub(crate) fn android_sdk(root: &Path) -> Option<PathBuf> {
    let configured = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|path| path.is_dir());
    if configured.is_some() {
        return configured;
    }

    let properties = std::fs::read_to_string(root.join("android/local.properties")).ok()?;
    properties
        .lines()
        .filter_map(|line| line.trim().strip_prefix("sdk.dir="))
        .map(PathBuf::from)
        .find(|path| path.is_dir())
}
