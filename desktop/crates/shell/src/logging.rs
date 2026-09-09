//! Where what happened is written down.
//!
//! `STACK.md` §3.9 asks for two destinations: something a person watching a terminal can
//! read, and a rolling file under `$XDG_STATE_HOME/photo-sync/` that outlives the session.
//! The second is the one that matters after the fact — the commit and recovery paths log at
//! INFO precisely so that a reconstruction is possible from the logs alone, and a log that
//! only ever existed on a terminal nobody was watching cannot do that.

use std::path::{Path, PathBuf};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Keeps the file writer alive. Dropping it stops the flushing thread, so an application
/// holds it for as long as it runs.
pub struct Logging {
    _writer: tracing_appender::non_blocking::WorkerGuard,
}

/// Starts logging to the terminal and to a file in `directory`.
///
/// `RUST_LOG` overrides the level, as it does everywhere else; without it the application
/// says what it is doing at INFO and its dependencies keep quiet.
///
/// # Errors
/// When the directory cannot be made.
pub fn start(directory: &Path) -> Result<Logging, std::io::Error> {
    std::fs::create_dir_all(directory)?;

    let files = tracing_appender::rolling::daily(directory, "photo-sync.log");
    let (writer, guard) = tracing_appender::non_blocking(files);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("photo_sync=info,warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .init();

    Ok(Logging { _writer: guard })
}

/// Where logs are kept, per the XDG base directory specification.
#[must_use]
pub fn directory() -> PathBuf {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(configured) if !configured.is_empty() => PathBuf::from(configured),
        _ => std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from("/"), PathBuf::from)
            .join(".local/state"),
    }
    .join("photo-sync")
}
