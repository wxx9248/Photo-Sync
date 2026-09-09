//! Settings, and the one thing changing them must never do.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::config::Config;
use photo_sync_core::covers;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn load(directory: &Path) -> Config {
    match Config::load(directory) {
        Ok(config) => config,
        Err(error) => panic!("cannot read the settings: {error}"),
    }
}

fn save(config: &Config, directory: &Path) {
    if let Err(error) = config.save(directory) {
        panic!("cannot save the settings: {error}");
    }
}

#[test]
fn a_desktop_nobody_has_configured_still_knows_where_to_put_things() {
    let scratch = Scratch::new();
    let config = load(&scratch.path);

    assert!(config.vault.ends_with("Pictures/Camera"));
    assert!(!config.name.is_empty());
    assert!(
        !config.autostart,
        "a first run added itself to the login session"
    );
}

#[test]
fn what_was_saved_is_what_comes_back() {
    let scratch = Scratch::new();
    let mut config = load(&scratch.path);
    config.vault = PathBuf::from("/srv/photographs");
    config.name = "Kitchen iMac".to_string();
    config.autostart = true;

    save(&config, &scratch.path);

    assert_eq!(load(&scratch.path), config);
}

#[test]
fn settings_nobody_can_read_are_reported_rather_than_replaced() {
    let scratch = Scratch::new();
    let file = scratch.path.join("config.toml");
    if let Err(error) = std::fs::write(&file, "vault = [this is not toml\n") {
        panic!("cannot write the file: {error}");
    }

    assert!(
        Config::load(&scratch.path).is_err(),
        "a file nobody could read was treated as an empty one"
    );
    // Still there. Overwriting somebody's settings because one line was mistyped would lose
    // the rest of them.
    assert!(file.is_file());
}

#[test]
fn changing_where_the_vault_is_moves_nothing() {
    covers!("R-UI-003");
    let scratch = Scratch::new();
    let old_vault = scratch.path.join("old");
    let new_vault = scratch.path.join("new");
    if let Err(error) = std::fs::create_dir_all(&old_vault) {
        panic!("cannot make a vault: {error}");
    }
    let photograph = old_vault.join("2026-08-24_093000.jpg");
    if let Err(error) = std::fs::write(&photograph, b"one photograph") {
        panic!("cannot write a photograph: {error}");
    }

    let mut config = load(&scratch.path);
    config.vault = old_vault.clone();
    save(&config, &scratch.path);

    // The person points the setting somewhere else. `SPEC.md` §4: this is a plain setting and
    // moves nothing. What was in the old place is still in the old place, and the new place is
    // not created behind their back.
    config.vault = new_vault.clone();
    save(&config, &scratch.path);

    assert_eq!(load(&scratch.path).vault, new_vault);
    assert!(
        photograph.is_file(),
        "changing the setting moved a photograph"
    );
    assert!(
        !new_vault.exists(),
        "changing the setting made the new vault on its own"
    );
}
