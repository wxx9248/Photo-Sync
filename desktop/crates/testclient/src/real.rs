//! A desktop that really runs, for a scenario to be checked against.
//!
//! The same scenario file describes the world for the simulator and for this. What differs is
//! only where the world is kept: memory there, a directory and two databases here.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photo_sync::desk::Desk;
use photo_sync::identity::Identity;
use photo_sync::pinning::Paired;
use photo_sync::serve::{Listening, listen};
use photo_sync::store::Store;
use photo_sync::tls::PairingWindow;
use photo_sync_core::id::{DeviceId, VaultName};
use photo_sync_core::store::{DeviceFileRow, StoreRequest};
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;

use crate::Connected;

/// A desktop's world, before anything is listening.
pub struct Prepared {
    root: PathBuf,
}

impl Prepared {
    #[must_use]
    pub fn in_directory(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn vault(&self) -> PathBuf {
        self.root.join("Camera")
    }

    fn index(&self) -> PathBuf {
        self.root.join("data/index.db")
    }

    /// Records photos an earlier session imported, and the vault copies behind them.
    ///
    /// # Errors
    /// When the databases or the vault cannot be written.
    pub fn remember(
        &self,
        imported: &[DeviceFileRow],
        vault: &[(VaultName, Vec<u8>)],
    ) -> Result<(), String> {
        std::fs::create_dir_all(self.vault()).map_err(|error| error.to_string())?;
        for (name, bytes) in vault {
            std::fs::write(self.vault().join(name.as_str()), bytes)
                .map_err(|error| error.to_string())?;
        }

        if imported.is_empty() {
            return Ok(());
        }
        let mut store =
            Store::open(&self.vault(), &self.index()).map_err(|error| error.to_string())?;
        store
            .run(&StoreRequest::InsertCommittedBatch {
                contents: imported
                    .iter()
                    .map(|row| photo_sync_core::store::ContentRow {
                        digest: row.digest,
                        vault_name: row.vault_name.clone(),
                    })
                    .collect(),
                device_files: imported.to_vec(),
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Opens the desktop and starts listening on a port the machine chooses.
    ///
    /// # Errors
    /// When the vault, the databases, or the socket cannot be opened.
    pub fn start(self) -> Result<Running, String> {
        let runtime = Runtime::new().map_err(|error| error.to_string())?;
        let desk = Desk::open(&self.vault(), &self.index()).map_err(|error| error.to_string())?;

        let identity = Identity::load_or_create(&self.root.join("identity"))
            .map_err(|error| error.to_string())?;
        let paired = Arc::new(Mutex::new(Paired::new()));

        let listening = runtime
            .block_on(listen(
                "127.0.0.1:0".parse().map_err(|_| "no loopback address")?,
                Arc::new(identity.clone()),
                Arc::clone(&paired),
                PairingWindow::closed(),
                Arc::new(AsyncMutex::new(desk)),
                "Scenario desktop",
                &self.root,
            ))
            .map_err(|error| error.to_string())?;

        let address = listening.address;
        Ok(Running {
            root: self.root,
            runtime,
            listening: Some(listening),
            address,
            identity,
            paired,
        })
    }
}

/// A desktop answering on a socket.
pub struct Running {
    root: PathBuf,
    runtime: Runtime,
    listening: Option<Listening>,
    address: SocketAddr,
    identity: Identity,
    paired: Arc<Mutex<Paired>>,
}

impl Running {
    /// Pairs a phone and connects it.
    ///
    /// Pairing is recorded directly, which is the state a completed exchange leaves behind.
    /// The exchange itself is the Pairing service, which is not built yet.
    ///
    /// # Errors
    /// When the phone's identity cannot be made or the desktop cannot be reached.
    pub fn connect(&self, device: &DeviceId, name: &str) -> Result<Connected, String> {
        let phone = Identity::load_or_create(&self.root.join(format!("phone-{device}")))
            .map_err(|error| error.to_string())?;

        {
            let mut paired = self
                .paired
                .lock()
                .map_err(|_| "the pairing record is stuck")?;
            paired.pair(phone.public_key(), device, name);
        }

        Connected::dial(
            self.runtime.handle(),
            self.address,
            &phone,
            self.identity.public_key(),
            device,
        )
    }

    /// What the vault holds, in name order.
    #[must_use]
    pub fn vault_names(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.root.join("Camera")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().to_str().map(ToString::to_string))
            .collect();
        names.sort();
        names
    }

    pub fn stop(mut self) {
        if let Some(listening) = self.listening.take() {
            listening.stop();
        }
    }
}
