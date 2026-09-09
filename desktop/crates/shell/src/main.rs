//! The desktop application: everything the other modules do, wired together and run.
//!
//! `SPEC.md` §4 describes a tray-and-window application that starts with the login session
//! and is otherwise not thought about. That shape is what this file is: it reads the
//! settings, opens the vault and its index, starts listening, says on the network that it is
//! here, puts an item in the tray, and shows a window that reports what the rest of it is
//! doing.
//!
//! Two of those can fail on a perfectly good machine. A desktop with no tray host has no
//! tray, and one with no login manager cannot promise not to sleep; neither is a reason to
//! refuse to move photographs, so both are reported once and stepped over.

use std::net::SocketAddr;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use photo_sync::config::{self, Config};
use photo_sync::desk::Desk;
use photo_sync::discovery::Advertising;
use photo_sync::identity::Identity;
use photo_sync::pinning::Paired;
use photo_sync::serve::listen;
use photo_sync::tls::PairingWindow;
use photo_sync::tray::{Asked, Tray};
use photo_sync::views::Phone;
use photo_sync::{autostart, logging, tray};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tokio::sync::Mutex as AsyncMutex;

/// The window markup, compiled into Rust by the build script.
///
/// The lints are turned off over this module and nowhere else: none of it is written here,
/// and holding generated code to the same standard as hand-written code only teaches a person
/// to ignore the output.
#[allow(
    warnings,
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction
)]
mod window {
    slint::include_modules!();
}

use window::{AppWindow, PhoneRow, StagedRow};

/// The port is asked for by the operating system and announced over mDNS, so nothing has to
/// agree on a number in advance.
const ANY_PORT: &str = "0.0.0.0:0";

/// How often the window catches up with what the desktop has been doing.
const REFRESH: std::time::Duration = std::time::Duration::from_millis(500);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = logging::start(&logging::directory())?;

    // Both languages ship from day one on this end too, because the same family operates
    // both halves of this. `STACK.md` §3.2.
    slint::init_translations!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/translations/"));

    let configuration = config::directory();
    let settings = Config::load(&configuration)?;
    tracing::info!(vault = %settings.vault.display(), name = %settings.name, "starting");

    // The setting is what a person decided; this makes the filesystem agree with it.
    if let Ok(program) = std::env::current_exe()
        && let Err(error) = autostart::set(&autostart::directory(), settings.autostart, &program)
    {
        tracing::warn!("{error}");
    }

    let data = data_directory();
    std::fs::create_dir_all(&data)?;
    let identity = Arc::new(Identity::load_or_create(&data.join("identity"))?);
    // The same directory the pairing service writes into. Reading from one place and
    // writing to another is a desktop that forgets every phone when it restarts.
    let paired = Arc::new(Mutex::new(Paired::load(&data)?));
    let desk = Arc::new(AsyncMutex::new(Desk::open(
        &settings.vault,
        &data.join("index.db"),
    )?));

    // The transport lives on its own runtime: the window owns the main thread, because that
    // is where a windowing system insists its event loop runs.
    let runtime = tokio::runtime::Runtime::new()?;
    let address: SocketAddr = ANY_PORT.parse()?;
    let listening = runtime.block_on(listen(
        address,
        Arc::clone(&identity),
        Arc::clone(&paired),
        PairingWindow::closed(),
        Arc::clone(&desk),
        &settings.name,
        &data,
    ))?;
    tracing::info!(port = listening.address.port(), "listening");

    let announced = match Advertising::start(&settings.name, listening.address.port(), &[]) {
        Ok(announced) => Some(announced),
        Err(error) => {
            tracing::warn!("this desktop will not be found automatically: {error}");
            None
        }
    };

    let (asked, asks) = channel();
    let tray_item = match tray::show(Tray::new(asked)) {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::warn!("there is no tray to sit in: {error}");
            None
        }
    };

    let window = AppWindow::new()?;
    window.set_vault(settings.vault.display().to_string().into());
    window.set_autostart(settings.autostart);
    window.set_phones(ModelRc::new(VecModel::from(Vec::<PhoneRow>::new())));
    window.set_staged(ModelRc::new(VecModel::from(Vec::<StagedRow>::new())));

    wire(&window, &configuration, &settings, &desk, &runtime);

    // What the desktop has been doing, brought over to the window at a human pace rather
    // than on every event: a transfer produces thousands of them a second.
    let refreshing = window.as_weak();
    let watching = Arc::clone(&desk);
    let ticking = runtime.handle().clone();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, REFRESH, move || {
        let Some(window) = refreshing.upgrade() else {
            return;
        };
        let shown = ticking.block_on(async { watching.lock().await.views().phones() });
        window.set_phones(ModelRc::new(VecModel::from(
            shown.iter().map(row_of).collect::<Vec<_>>(),
        )));
    });

    // A tray click has to reach the window, which only the windowing thread may touch.
    let opening = window.as_weak();
    std::thread::spawn(move || {
        while let Ok(what) = asks.recv() {
            let opening = opening.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = opening.upgrade() {
                    match what {
                        Asked::Open => {
                            let _ = window.show();
                        }
                        Asked::Quit => {
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            });
        }
    });

    window.run()?;

    tracing::info!("stopping");
    if let Some(tray_item) = tray_item {
        tray_item.shutdown();
    }
    if let Some(announced) = announced {
        announced.stop();
    }
    listening.stop();
    Ok(())
}

/// Connects what the window offers a person to what the rest of this does about it.
fn wire(
    window: &AppWindow,
    configuration: &std::path::Path,
    settings: &Config,
    desk: &Arc<AsyncMutex<Desk>>,
    runtime: &tokio::runtime::Runtime,
) {
    let saving = configuration.to_path_buf();
    let held = settings.clone();
    window.on_vault_changed(move |chosen| {
        // A plain setting. Nothing is moved, which is what §4 says and what the window says
        // underneath the box.
        let mut settings = held.clone();
        settings.vault = std::path::PathBuf::from(chosen.to_string());
        if let Err(error) = settings.save(&saving) {
            tracing::warn!("{error}");
        }
    });

    let saving = configuration.to_path_buf();
    let held = settings.clone();
    window.on_autostart_changed(move |wanted| {
        let mut settings = held.clone();
        settings.autostart = wanted;
        if let Err(error) = settings.save(&saving) {
            tracing::warn!("{error}");
        }
        if let Ok(program) = std::env::current_exe()
            && let Err(error) = autostart::set(&autostart::directory(), wanted, &program)
        {
            tracing::warn!("{error}");
        }
    });

    let committing = Arc::clone(desk);
    let handle = runtime.handle().clone();
    window.on_commit_now(move |device| {
        // §7.3's manual commit: rescues staging left by a phone that never came back.
        let device = photo_sync_core::id::DeviceId::new(device.to_string());
        let committing = Arc::clone(&committing);
        handle.spawn(async move {
            committing
                .lock()
                .await
                .deliver(photo_sync_core::event::Event::ManualCommitRequested { device });
        });
    });
}

fn row_of(phone: &Phone) -> PhoneRow {
    PhoneRow {
        name: SharedString::from(phone.name.clone()),
        detail: SharedString::from(phone.detail.clone()),
        #[allow(clippy::cast_precision_loss)]
        progress: phone.progress as f32 / 100.0,
        busy: phone.busy,
    }
}

/// Where the identity, the pairings and the index live. `STACK.md` §3.9.
fn data_directory() -> std::path::PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(configured) if !configured.is_empty() => std::path::PathBuf::from(configured),
        _ => std::env::var_os("HOME")
            .map_or_else(|| std::path::PathBuf::from("/"), std::path::PathBuf::from)
            .join(".local/share"),
    }
    .join("photo-sync")
}
