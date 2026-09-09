//! What the person running the desktop has decided.
//!
//! `STACK.md` §3.9 puts this at `$XDG_CONFIG_HOME/photo-sync/config.toml`. Everything in it
//! has a default that works, so a first run needs no file and no questions: the vault goes to
//! `~/Pictures/Camera`, the desktop announces itself under the machine's own name, and it does
//! not add itself to the login session until somebody says so.
//!
//! The vault path is a plain setting and nothing more. `SPEC.md` §4 is explicit that changing
//! it moves no files: the person relocates the contents if they want them moved, and the
//! degradation either way is graceful, because a photograph absent from the new location is
//! simply never nominated for deletion and never imported twice.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file the settings live in, under the configuration directory.
const FILE: &str = "config.toml";

/// Where photographs are kept when nobody has said otherwise. `SPEC.md` §4.
const DEFAULT_VAULT: &str = "Pictures/Camera";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Where the vault is. Changing this moves nothing.
    pub vault: PathBuf,

    /// What this desktop calls itself, on the phone's screen and in the mDNS record.
    pub name: String,

    /// Whether to start with the login session.
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vault: home().join(DEFAULT_VAULT),
            name: machine_name(),
            autostart: false,
        }
    }
}

/// What went wrong reading or writing the settings.
#[derive(Debug)]
pub enum ConfigError {
    /// The file is there and cannot be read.
    Unreadable(String),

    /// The file is there and is not a configuration.
    Malformed(String),

    /// The file could not be written.
    Unwritable(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the settings could not be read: {why}"),
            Self::Malformed(why) => write!(f, "the settings are not readable as settings: {why}"),
            Self::Unwritable(why) => write!(f, "the settings could not be saved: {why}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads the settings out of a directory, or hands back the defaults if there are none.
    ///
    /// A missing file is an ordinary first run rather than a problem. A file that is there and
    /// cannot be understood is a problem, and is reported rather than replaced: overwriting
    /// somebody's settings because a line in them was mistyped would lose the rest of it.
    ///
    /// # Errors
    /// When the file exists and cannot be read or parsed.
    pub fn load(directory: &Path) -> Result<Self, ConfigError> {
        let path = directory.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(ConfigError::Unreadable(error.to_string())),
        };
        toml::from_str(&text).map_err(|error| ConfigError::Malformed(error.to_string()))
    }

    /// Writes the settings into a directory, creating it if it is not there.
    ///
    /// # Errors
    /// When the directory or the file cannot be written.
    pub fn save(&self, directory: &Path) -> Result<(), ConfigError> {
        std::fs::create_dir_all(directory)
            .map_err(|error| ConfigError::Unwritable(error.to_string()))?;
        let text = toml::to_string_pretty(self)
            .map_err(|error| ConfigError::Unwritable(error.to_string()))?;
        std::fs::write(directory.join(FILE), text)
            .map_err(|error| ConfigError::Unwritable(error.to_string()))
    }
}

/// The directory the settings live in, per the XDG base directory specification.
#[must_use]
pub fn directory() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(configured) if !configured.is_empty() => PathBuf::from(configured),
        _ => home().join(".config"),
    }
    .join("photo-sync")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// The machine's own name, which is what a person recognises in a list of desktops.
fn machine_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|read| read.trim().to_string())
        .filter(|read| !read.is_empty())
        .unwrap_or_else(|| "Photo Sync".to_string())
}
