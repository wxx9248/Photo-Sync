//! Starting with the login session, when somebody has asked for that.
//!
//! `STACK.md` §3.9 makes this a desktop entry in the XDG autostart directory rather than a
//! systemd user unit, and gives the reason: the application needs the graphical session for
//! its tray and for the sleep inhibitor it holds, so it is a session application and not a
//! service. Turning the setting off removes the file; nothing else about the installation
//! changes either way.

use std::path::{Path, PathBuf};

/// The file this writes, named after the application so a person can recognise it.
const ENTRY: &str = "photo-sync.desktop";

/// What went wrong adding or removing the entry.
#[derive(Debug)]
pub struct AutostartError(String);

impl std::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the autostart entry could not be changed: {}", self.0)
    }
}

impl std::error::Error for AutostartError {}

/// The directory login sessions read entries from.
#[must_use]
pub fn directory() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(configured) if !configured.is_empty() => PathBuf::from(configured),
        _ => std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from("/"), PathBuf::from)
            .join(".config"),
    }
    .join("autostart")
}

/// Whether an entry is there now.
#[must_use]
pub fn is_enabled(directory: &Path) -> bool {
    directory.join(ENTRY).is_file()
}

/// Makes the setting true of the filesystem, whichever way it is being changed.
///
/// Writing the entry twice and removing one that is not there are both ordinary: the setting
/// is what a person changed, and this makes the world match it.
///
/// # Errors
/// When the directory or the file cannot be written, or the file cannot be removed.
pub fn set(directory: &Path, enabled: bool, command: &Path) -> Result<(), AutostartError> {
    let entry = directory.join(ENTRY);
    if !enabled {
        return match std::fs::remove_file(&entry) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AutostartError(error.to_string())),
        };
    }

    std::fs::create_dir_all(directory).map_err(|error| AutostartError(error.to_string()))?;
    std::fs::write(&entry, desktop_entry(command))
        .map_err(|error| AutostartError(error.to_string()))
}

/// The entry itself. `X-GNOME-Autostart-enabled` is honoured by Plasma as well, and is what
/// the desktop environment's own settings page toggles when a person turns this off there.
fn desktop_entry(command: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Photo Sync\n\
         Comment=Keeps the family's photographs off their phones\n\
         Exec={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        command.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_names_the_program_that_will_run() {
        let written = desktop_entry(Path::new("/usr/bin/photo-sync"));
        assert!(written.contains("Exec=/usr/bin/photo-sync\n"));
        assert!(written.starts_with("[Desktop Entry]\n"));
    }
}
