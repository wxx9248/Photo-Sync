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
/// Where the port from the last run is kept, beside the identity and the index.
const PORT_FILE: &str = "port";

/// How often the window catches up with what the desktop has been doing.
const REFRESH: std::time::Duration = std::time::Duration::from_millis(500);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = logging::start(&logging::directory())?;

    // Both languages ship from day one on this end too, because the same family operates
    // both halves of this. `STACK.md` §3.2.
    let locales = locale_directory();
    tracing::info!(locales = %locales.display(), "translations");
    slint::init_translations!(locales);

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
    // `[::]` rather than `0.0.0.0`: the responder advertises every address this machine holds
    // and a phone picks among them, so a desktop listening on one family only is a desktop
    // that can be found and not reached. Linux accepts IPv4 on an IPv6 socket unless
    // `bindv6only` is set, so this listens for both.
    let address: SocketAddr = format!("[::]:{}", port_to_ask_for(&data)).parse()?;
    // One window, shared: the tray opens it, the pairing service shows a code in it, and the
    // person answers through it. `PairingWindow::closed()` here meant the desktop could never
    // meet a phone it did not already know.
    let pairing = PairingWindow::closed();
    let listening = runtime.block_on(listen(
        address,
        Arc::clone(&identity),
        Arc::clone(&paired),
        pairing.clone(),
        Arc::clone(&desk),
        &settings.name,
        &data,
    ))?;
    tracing::info!(port = listening.address.port(), "listening");
    remember_port(&data, listening.address.port());

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

    // §5.2 asks a person whether two screens match, and this is where their answer lands.
    // Only the answer. Closing the window here would wipe the answer before the call waiting
    // on it had read it, and the phone would be told no however the button was pressed; the
    // pairing service closes it once it has what it asked for.
    let agreeing = pairing.clone();
    window.on_pair_confirmed(move || agreeing.confirm());
    let refusing = pairing.clone();
    window.on_pair_rejected(move || refusing.reject());

    // What the desktop has been doing, brought over to the window at a human pace rather
    // than on every event: a transfer produces thousands of them a second.
    let refreshing = window.as_weak();
    let watching = Arc::clone(&desk);
    let showing = pairing.clone();
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
        // The code appears while a phone is waiting on an answer, and goes when it is given.
        window.set_pairing_code(showing.showing().unwrap_or_default().into());
    });

    // A tray click has to reach the window, which only the windowing thread may touch.
    let opening = window.as_weak();
    let opening_pairing = pairing.clone();
    std::thread::spawn(move || {
        while let Ok(what) = asks.recv() {
            let opening = opening.clone();
            let opening_pairing = opening_pairing.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = opening.upgrade() {
                    match what {
                        Asked::Open => {
                            let _ = window.show();
                        }
                        Asked::Pair => {
                            // The window has to be up, because the code appears in it and
                            // somebody has to read it.
                            opening_pairing.open();
                            tracing::info!("open to a phone that is not paired yet");
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

    // Closing the window puts it away and nothing more. `SPEC.md` §4 asks for an application
    // that is always ready, and a tray icon is only worth having if the process behind it is
    // still listening: a desktop that stopped advertising when its window was dismissed is one
    // a phone cannot find until somebody opens it again. Leaving is the tray's Quit item,
    // which is what `tray.rs` already says of it.
    window
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    // `run()` would end the loop as soon as the last window went away, hiding included, so
    // the loop is run in the mode that ends only when something asks it to.
    window.show()?;
    slint::run_event_loop_until_quit()?;

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

/// The port to ask for: the one this desktop used last time, if it is still free.
///
/// An mDNS record outlives the process that published it. A desktop that is killed rather
/// than closed sends no goodbye, and a phone can hold the stale record for minutes --- so it
/// dials the old port and finds nothing there. Keeping the port across restarts makes that
/// record true again, which is cheaper than teaching every phone to doubt its own cache.
///
/// Zero means "any", which is what a first run and a taken port both get.
fn port_to_ask_for(data: &std::path::Path) -> u16 {
    let Ok(text) = std::fs::read_to_string(data.join(PORT_FILE)) else {
        return 0;
    };
    let Ok(remembered) = text.trim().parse::<u16>() else {
        return 0;
    };
    // Asking and finding out are the same act, so this binds to see and drops immediately.
    // Something else could take it in between; the cost of that is one changed port.
    match std::net::TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, remembered)) {
        Ok(_) => remembered,
        Err(_) => 0,
    }
}

fn remember_port(data: &std::path::Path, port: u16) {
    if let Err(error) = std::fs::write(data.join(PORT_FILE), port.to_string()) {
        // Worth saying and not worth stopping for: the next start picks another port and a
        // phone with a stale record waits for its cache to expire.
        tracing::warn!("this desktop will not remember its port: {error}");
    }
}

/// Where the identity, the pairings and the index live. `STACK.md` §3.9.
/// Where the compiled message catalogues are on this machine.
///
/// Three places, in the order they should win. A packager who says outright is obeyed; an
/// installed desktop finds them under the standard prefix, which is where `packaging/PKGBUILD`
/// puts them; and a checkout falls back to the ones the build compiled for it.
///
/// The path used to be the build machine's source directory, baked in. That directory holds
/// no compiled catalogue and does not exist on anybody else's computer, so the window spoke
/// English in both cases however the locale was set.
fn locale_directory() -> std::path::PathBuf {
    if let Some(said) = std::env::var_os("PHOTO_SYNC_LOCALE_DIR") {
        return std::path::PathBuf::from(said);
    }

    let installed = std::path::PathBuf::from("/usr/share/locale");
    if holds_catalogue(&installed) {
        return installed;
    }

    std::path::PathBuf::from(env!("PHOTO_SYNC_BUILT_LOCALE_DIR"))
}

/// Whether any language under `directory` carries this application's catalogue.
fn holds_catalogue(directory: &std::path::Path) -> bool {
    let Ok(languages) = std::fs::read_dir(directory) else {
        return false;
    };
    languages
        .filter_map(Result::ok)
        .any(|language| language.path().join("LC_MESSAGES/photo-sync.mo").is_file())
}

fn data_directory() -> std::path::PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(configured) if !configured.is_empty() => std::path::PathBuf::from(configured),
        _ => std::env::var_os("HOME")
            .map_or_else(|| std::path::PathBuf::from("/"), std::path::PathBuf::from)
            .join(".local/share"),
    }
    .join("photo-sync")
}
