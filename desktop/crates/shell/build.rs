//! Compiles the window markup into Rust, and the message catalogues into the form gettext
//! reads, before the crate itself is built.

use std::path::{Path, PathBuf};

fn main() {
    slint_build::compile("ui/app.slint").expect("the window markup does not compile");
    compile_translations();
}

/// Turns each `.po` into the `.mo` that gettext actually loads.
///
/// The catalogues are written and reviewed as `.po` and nothing reads those at run time.
/// Without this step a checkout runs in English however its locale is set, which is how a
/// translation can be complete, committed, and never once displayed.
fn compile_translations() {
    let source = Path::new("ui/translations");
    println!("cargo::rerun-if-changed={}", source.display());

    let built =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("locale");
    println!(
        "cargo::rustc-env=PHOTO_SYNC_BUILT_LOCALE_DIR={}",
        built.display()
    );

    let Ok(languages) = std::fs::read_dir(source) else {
        println!("cargo::warning=there are no translations to compile");
        return;
    };

    for language in languages.filter_map(Result::ok) {
        let catalogue = language.path().join("LC_MESSAGES/photo-sync.po");
        if !catalogue.is_file() {
            continue;
        }
        println!("cargo::rerun-if-changed={}", catalogue.display());

        let into = built.join(language.file_name()).join("LC_MESSAGES");
        if let Err(error) = std::fs::create_dir_all(&into) {
            println!("cargo::warning=cannot make {}: {error}", into.display());
            continue;
        }

        // A machine without gettext still builds, in English. The application is worth more
        // than its translations are, and `docs/DEVELOPMENT.md` says which package this is.
        match std::process::Command::new("msgfmt")
            .arg("-o")
            .arg(into.join("photo-sync.mo"))
            .arg(&catalogue)
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => println!(
                "cargo::warning=msgfmt refused {}: {status}",
                catalogue.display()
            ),
            Err(error) => println!("cargo::warning=cannot run msgfmt: {error}"),
        }
    }
}
