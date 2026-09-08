//! What survives the power going out.
//!
//! The simulated filesystem makes a write durable only when something asks it to, so these
//! run the desktop up to a chosen point, take the machine away, and ask what is left. That
//! is the only way the claims of `SPEC.md` §7.3 and §7.6 can be checked rather than asserted.

use photo_sync_core::event::Event;
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::{CatalogEntry, CivilTime, Moment, RunningDigest, covers};
use photo_sync_sim::{Phone, PhoneFile, Simulation};

const MTIME: i64 = 1_756_000_000;
const PHOTO: &[u8] = b"the bytes of one photograph";

const IMPORTED_AT: Moment = Moment {
    at: Timestamp(1_757_000_000),
    local: CivilTime {
        year: 2026,
        month: 9,
        day: 7,
        hour: 23,
        minute: 0,
        second: 0,
    },
};

fn desktop() -> Simulation {
    let mut sim = Simulation::new(IMPORTED_AT);
    sim.start();
    sim.take_log();
    sim
}

fn phone() -> DeviceId {
    DeviceId::new("phone-a")
}

fn photo_path() -> DevicePath {
    DevicePath::new("DCIM/Camera/IMG_0001.jpg")
}

fn digest_of(bytes: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(bytes);
    running.peek()
}

/// Connects a phone and hands over a catalog of one photo, returning what it is asked for.
fn offer(sim: &mut Simulation) -> FileId {
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: photo_path(),
            size: PHOTO.len() as u64,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: PHOTO.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });

    for effect in sim.take_log() {
        if let photo_sync_core::Effect::SendDiff { to_send, .. } = effect
            && let Some(first) = to_send.first()
        {
            return first.file;
        }
    }
    panic!("the desktop asked for nothing");
}

fn send_all(sim: &mut Simulation, file: FileId) {
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: 0,
    });
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: 0,
        data: PHOTO.to_vec(),
    });
    sim.deliver(Event::UploadClosed {
        device: phone(),
        file,
        digest: digest_of(PHOTO),
    });
}

#[test]
fn a_partial_is_cut_back_to_its_watermark_when_the_machine_returns() {
    covers!("R-STAGE-007");
    let mut sim = desktop();
    let file = offer(&mut sim);
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: 0,
    });
    // Ten bytes arrive and are made durable by the sync the connection loss forces; the rest
    // arrive and are not.
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: 0,
        data: PHOTO[..10].to_vec(),
    });
    sim.deliver(Event::PeerDisconnected { device: phone() });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });

    sim.restart();

    // The file survived, cut back to the bytes a sync had proven. A crash can leave a file
    // longer than that, and the tail past it is whatever the disk happened to hold.
    assert_eq!(
        sim.storage
            .staging()
            .get(&file)
            .map(|staged| staged.bytes.clone()),
        Some(PHOTO[..10].to_vec())
    );
    assert!(sim.storage.vault().is_empty());
}

#[test]
fn a_photo_that_reached_the_vault_is_still_there_afterwards() {
    covers!("R-COMMIT-008");
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);
    sim.deliver(Event::FinishRequested { device: phone() });
    let name = only_name(&sim);

    sim.restart();

    // SPEC.md §7.3 syncs both directories before it writes a done-mark, so a committed photo
    // survives a power loss the instant its commit says it did.
    assert_eq!(sim.storage.vault().get(&name), Some(&PHOTO.to_vec()));
}

#[test]
fn a_commit_makes_the_vault_durable_before_it_says_it_is_done() {
    covers!("R-COMMIT-008");
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);
    sim.deliver(Event::FinishRequested { device: phone() });
    let name = only_name(&sim);

    // Asked of the picture a crash would leave, rather than of the one a reader sees.
    assert_eq!(
        sim.storage.durable_vault().get(&name),
        Some(&PHOTO.to_vec())
    );
}

#[test]
fn staging_is_emptied_durably_when_the_commit_finishes() {
    covers!("R-COMMIT-012");
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);

    sim.deliver(Event::FinishRequested { device: phone() });
    sim.restart();

    assert!(!sim.storage.holds(file));
    assert!(sim.storage.durable_staging().is_empty());
}

#[test]
fn a_photo_the_desktop_never_finished_taking_is_asked_for_again() {
    covers!("R-DIFF-004");
    let mut sim = desktop();
    let file = offer(&mut sim);
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: 0,
    });
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: 0,
        data: PHOTO[..10].to_vec(),
    });
    sim.restart();

    // The desktop cannot continue a digest it no longer holds, so it starts the file again
    // rather than trusting a prefix it cannot check. SPEC.md §7.6 allows exactly that.
    let asked = offer(&mut sim);

    assert_eq!(sim.storage.vault().len(), 0);
    send_all(&mut sim, asked);
    sim.deliver(Event::FinishRequested { device: phone() });
    assert_eq!(sim.storage.vault().len(), 1);
}

#[test]
fn a_whole_session_survives_a_restart_between_sessions() {
    let mut sim = desktop();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, PHOTO.to_vec()),
    );
    phone.run_session(&mut sim);
    let before: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();

    sim.restart();

    let after: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();
    assert_eq!(before, after);
    assert_eq!(after.len(), 1);
}

fn only_name(sim: &Simulation) -> VaultName {
    let mut names = sim.storage.vault().keys();
    match (names.next(), names.next()) {
        (Some(name), None) => name.clone(),
        _ => panic!("the vault holds {} files", sim.storage.vault().len()),
    }
}

#[test]
fn a_partial_longer_than_its_watermark_is_cut_back_to_it() {
    covers!("R-STAGE-007");
    // Three pieces: two that together cross the interval the desktop lets sit unsynced, and
    // one after it that never does. Only the first two were ever proven.
    let piece = vec![b'p'; 9 << 20];
    let tail = vec![b't'; 1 << 20];
    let proven = piece.len() * 2;
    let size = (proven + tail.len()) as u64;

    let mut sim = desktop();
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: photo_path(),
            size,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: size,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    let file = match sim.take_log().into_iter().find_map(|effect| match effect {
        photo_sync_core::Effect::SendDiff { to_send, .. } => to_send.first().map(|one| one.file),
        _ => None,
    }) {
        Some(file) => file,
        None => panic!("the desktop asked for nothing"),
    };

    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: 0,
    });
    for (offset, bytes) in [(0, &piece), (piece.len(), &piece)] {
        sim.deliver(Event::ChunkArrived {
            device: phone(),
            file,
            offset: offset as u64,
            data: bytes.clone(),
        });
    }
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: proven as u64,
        data: tail,
    });

    sim.restart();

    // The file came back longer than anything a sync had proven, with a tail that is not the
    // photograph. Recovery cuts it back to the watermark rather than to the length.
    let kept = match sim.storage.staging().get(&file) {
        Some(staged) => staged.bytes.clone(),
        None => panic!("the partial did not survive"),
    };
    assert_eq!(kept.len(), proven);
    assert!(kept.iter().all(|byte| *byte == b'p'));
}

#[test]
fn a_verified_leftover_is_left_alone_when_the_machine_returns() {
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);

    // Verified but not committed: the phone never sent the finish signal.
    sim.restart();

    let cut_back = sim.log().iter().any(|effect| {
        matches!(effect, photo_sync_core::Effect::TruncateFile { file: cut, .. } if *cut == file)
    });
    assert!(
        !cut_back,
        "a file whose digest already matched was cut back anyway"
    );
}
