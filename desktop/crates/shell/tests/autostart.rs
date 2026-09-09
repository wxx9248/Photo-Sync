//! Adding and removing the login-session entry.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::autostart;

fn scratch() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "photo-sync-autostart-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(error) = std::fs::create_dir_all(&path) {
        panic!("cannot make a scratch directory: {error}");
    }
    path
}

fn set(directory: &Path, enabled: bool) {
    if let Err(error) = autostart::set(directory, enabled, Path::new("/usr/bin/photo-sync")) {
        panic!("{error}");
    }
}

#[test]
fn turning_it_on_and_off_again_leaves_nothing_behind() {
    let directory = scratch();
    assert!(!autostart::is_enabled(&directory));

    set(&directory, true);
    assert!(autostart::is_enabled(&directory));

    set(&directory, false);
    assert!(!autostart::is_enabled(&directory));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn setting_it_the_way_it_already_is_changes_nothing() {
    let directory = scratch();
    set(&directory, true);
    set(&directory, true);
    assert!(autostart::is_enabled(&directory));

    set(&directory, false);
    // Removing an entry that is not there is an ordinary thing to ask for, not an error.
    set(&directory, false);
    assert!(!autostart::is_enabled(&directory));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_directory_that_does_not_exist_yet_is_made() {
    let directory = scratch().join("never-created");
    set(&directory, true);

    assert!(autostart::is_enabled(&directory));
    let _ = std::fs::remove_dir_all(&directory);
}
