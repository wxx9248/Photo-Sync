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
    offered(sim).0
}

/// The same, and the offset the desktop said to carry on from.
fn offered(sim: &mut Simulation) -> (FileId, u64) {
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
            return (first.file, first.resume_offset);
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
    agreed(&sim);

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
    agreed(&sim);

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
    agreed(&sim);

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
    agreed(&sim);

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
    agreed(&sim);

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
    agreed(&sim);

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
    agreed(&sim);

    let cut_back = sim.log().iter().any(|effect| {
        matches!(effect, photo_sync_core::Effect::TruncateFile { file: cut, .. } if *cut == file)
    });
    assert!(
        !cut_back,
        "a file whose digest already matched was cut back anyway"
    );
}

// ---- a commit interrupted part-way ---------------------------------------------------------

fn is_rename(effect: &photo_sync_core::Effect) -> bool {
    matches!(effect, photo_sync_core::Effect::RenameIntoVault { .. })
}

fn is_done_mark(effect: &photo_sync_core::Effect) -> bool {
    matches!(
        effect,
        photo_sync_core::Effect::Store {
            request: photo_sync_core::store::StoreRequest::MarkPlanEntriesDone { .. },
            ..
        }
    )
}

/// Runs a session up to the finish signal, then stops the machine just before `stop`.
fn commit_until(sim: &mut Simulation, stop: fn(&photo_sync_core::Effect) -> bool) {
    let file = offer(sim);
    send_all(sim, file);
    sim.deliver_until(Event::FinishRequested { device: phone() }, stop);
}

#[test]
fn a_commit_that_never_moved_a_file_is_finished_afterwards() {
    covers!("R-RECOVER-002");
    let mut sim = desktop();
    commit_until(&mut sim, is_rename);

    // The plan was sealed and nothing had happened yet.
    assert!(sim.storage.vault().is_empty());
    sim.restart();
    agreed(&sim);

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(sim.store.device_files().len(), 1);
    assert!(sim.store.manifest().is_empty());
    assert!(sim.store.sealed_plan(&phone()).is_none());
}

#[test]
fn a_commit_that_moved_the_file_but_never_said_so_is_not_undone() {
    covers!("R-RECOVER-003");
    let mut sim = desktop();
    commit_until(&mut sim, is_done_mark);

    // The rename happened and both directories were synced, so the photograph is in the
    // vault; nothing had written down that it was.
    let before: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();
    assert_eq!(before.len(), 1);
    sim.restart();
    agreed(&sim);

    // The sealed map reserved that name, so the file already sitting there is this desktop's
    // own completed rename rather than a reason to write a second copy.
    let after: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();
    assert_eq!(after, before);
    assert_eq!(sim.store.device_files().len(), 1);
    assert!(sim.store.manifest().is_empty());
}

#[test]
fn a_replayed_commit_records_the_photo_it_committed() {
    covers!("R-RECOVER-004");
    let mut sim = desktop();
    commit_until(&mut sim, is_rename);
    sim.restart();
    agreed(&sim);

    let rows = sim.store.device_files();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, photo_path());
    assert_eq!(rows[0].digest, digest_of(PHOTO));
    assert_eq!(sim.store.content().len(), 1);
}

#[test]
fn a_desktop_with_no_write_log_replays_nothing() {
    covers!("R-RECOVER-001");
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);

    // Verified, staged, and no commit was ever asked for.
    sim.restart();
    agreed(&sim);

    assert!(sim.storage.vault().is_empty());
    assert!(sim.store.device_files().is_empty());
    // The staged file is still there, and the next commit will take it.
    assert_eq!(sim.store.manifest().len(), 1);
}

#[test]
fn a_replayed_commit_still_lets_the_phone_finish_next_time() {
    let mut sim = desktop();
    commit_until(&mut sim, is_rename);
    sim.restart();
    agreed(&sim);

    // The device was locked while its commit was in flight; once replayed it is free.
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0002.jpg",
        PhoneFile::new(MTIME, b"a second photograph".to_vec()),
    );
    let outcome = phone.run_session(&mut sim);

    assert_eq!(sim.storage.vault().len(), 2);
    assert_eq!(outcome.uploaded.len(), 1);
}

/// Asks the desktop and the model whether they agree about what survived.
fn agreed(sim: &Simulation) {
    if let Err(differences) = sim.agree() {
        panic!(
            "the desktop and the model disagree: {}",
            differences.join("; ")
        );
    }
}

#[test]
fn a_transfer_the_power_interrupted_carries_on_from_the_watermark() {
    covers!("R-STAGE-009", "R-STAGE-008", "R-DIFF-003");
    let mut sim = desktop();
    let file = offer(&mut sim);
    let (head, tail) = PHOTO.split_at(10);

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
        data: head.to_vec(),
    });
    // A dropped connection makes the bytes so far durable and finalises the watermark.
    sim.deliver(Event::PeerDisconnected { device: phone() });

    sim.restart();
    agreed(&sim);

    // The desktop no longer holds the digest it was building, so it reads the prefix back off
    // its own disk before it says where to carry on from. SPEC.md §7.6.
    let (resumed, offered) = offered(&mut sim);
    assert_eq!(resumed, file, "a new file was started instead of resuming");
    assert_eq!(offered, head.len() as u64);

    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: offered,
    });
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: offered,
        data: tail.to_vec(),
    });
    // The digest covers the whole photograph, prefix included, so only a desktop that really
    // replayed those first bytes can agree with what the phone states.
    sim.deliver(Event::UploadClosed {
        device: phone(),
        file,
        digest: digest_of(PHOTO),
    });
    sim.deliver(Event::FinishRequested { device: phone() });

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(
        sim.storage.vault().values().next(),
        Some(&PHOTO.to_vec()),
        "the vault copy is not the photograph"
    );
    agreed(&sim);
}

#[test]
fn a_partial_that_cannot_be_read_back_starts_again() {
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
    sim.deliver(Event::PeerDisconnected { device: phone() });
    sim.restart();

    // The file is gone from under the manifest row that describes it.
    sim.storage.remove(file);
    let (asked, offered) = offered(&mut sim);

    assert_eq!(offered, 0, "a prefix nothing could read was trusted anyway");
    assert_ne!(asked, file);
}

/// Connects a phone with a catalog of the caller's choosing and hands back what the desktop
/// answered, so a test can look at what it decided to read as well as what it asked for.
fn answered_for(sim: &mut Simulation, entries: Vec<CatalogEntry>) -> Vec<photo_sync_core::Effect> {
    let total_bytes = entries.iter().map(|entry| entry.size).sum();
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries,
        total_bytes,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    sim.take_log()
}

/// The one catalog entry the rest of these tests use.
fn only_photo() -> CatalogEntry {
    CatalogEntry {
        path: photo_path(),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME),
    }
}

fn reads_in(effects: &[photo_sync_core::Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, photo_sync_core::Effect::ReadStagedRange { .. }))
        .count()
}

fn resume_offset_in(effects: &[photo_sync_core::Effect]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        photo_sync_core::Effect::SendDiff { to_send, .. } => {
            to_send.first().map(|first| first.resume_offset)
        }
        _ => None,
    })
}

/// Gets part of one photograph across and makes it durable, then takes the machine away.
fn interrupted(sim: &mut Simulation, keep: usize) -> FileId {
    let file = offer(sim);
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
        data: PHOTO[..keep].to_vec(),
    });
    sim.deliver(Event::PeerDisconnected { device: phone() });
    sim.restart();
    file
}

#[test]
fn a_photo_already_verified_is_never_read_back_again() {
    covers!("R-STAGE-009");
    let mut sim = desktop();
    let file = offer(&mut sim);
    send_all(&mut sim, file);
    sim.restart();

    // Its digest was checked before it was called verified, and that verdict was written
    // down. Reading the whole photograph again would be asking a settled question.
    let answered = answered_for(&mut sim, vec![only_photo()]);
    assert_eq!(reads_in(&answered), 0, "a verified file was read back");
    agreed(&sim);
}

#[test]
fn a_partial_with_nothing_durable_is_not_read_back() {
    covers!("R-STAGE-009");
    let mut sim = desktop();
    let file = offer(&mut sim);
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset: 0,
    });
    // The row exists and no byte of the photograph ever reached the disk.
    sim.restart();

    let answered = answered_for(&mut sim, vec![only_photo()]);
    assert_eq!(reads_in(&answered), 0, "an empty partial was read back");
    assert_eq!(resume_offset_in(&answered), Some(0));
    agreed(&sim);
}

#[test]
fn a_partial_whose_photograph_grew_is_not_read_back() {
    covers!("R-STAGE-009", "R-DIFF-004");
    let mut sim = desktop();
    interrupted(&mut sim, 10);

    // The phone edited the photograph, so what is on the disk is part of a file nobody is
    // sending any more. Its digest is worth nothing whatever it comes to.
    let answered = answered_for(
        &mut sim,
        vec![CatalogEntry {
            size: PHOTO.len() as u64 + 5,
            ..only_photo()
        }],
    );
    assert_eq!(reads_in(&answered), 0, "a superseded partial was read back");
    assert_eq!(resume_offset_in(&answered), Some(0));
    agreed(&sim);
}

#[test]
fn a_partial_whose_photograph_was_touched_is_not_read_back() {
    covers!("R-STAGE-009", "R-DIFF-004");
    let mut sim = desktop();
    interrupted(&mut sim, 10);

    // Same size, different moment: still not the file the prefix came from.
    let answered = answered_for(
        &mut sim,
        vec![CatalogEntry {
            mtime: Timestamp(MTIME + 1),
            ..only_photo()
        }],
    );
    assert_eq!(reads_in(&answered), 0, "a superseded partial was read back");
    assert_eq!(resume_offset_in(&answered), Some(0));
    agreed(&sim);
}

#[test]
fn a_partial_nothing_can_read_starts_again() {
    covers!("R-STAGE-009");
    let mut sim = desktop();
    interrupted(&mut sim, 10);

    // The disk still has the row and will not give the bytes back.
    sim.faults.refuse_staged_reads = true;
    let answered = answered_for(&mut sim, vec![only_photo()]);

    assert_eq!(reads_in(&answered), 1, "the desktop never tried to read it");
    assert_eq!(
        resume_offset_in(&answered),
        Some(0),
        "a prefix nothing could read was trusted anyway"
    );
    agreed(&sim);
}

#[test]
fn a_long_partial_is_read_back_a_chunk_at_a_time() {
    covers!("R-STAGE-009", "R-STAGE-008");
    // Half again as much as one read covers, so the second read has to start where the first
    // one stopped rather than anywhere else.
    const KEPT: usize = 1024 * 1024 + 512 * 1024;
    let long: Vec<u8> = (0..2 * 1024 * 1024u32).map(|at| (at % 251) as u8).collect();

    let mut sim = desktop();
    let entry = CatalogEntry {
        path: photo_path(),
        size: long.len() as u64,
        mtime: Timestamp(MTIME),
    };
    let answered = answered_for(&mut sim, vec![entry.clone()]);
    let file = match answered.iter().find_map(|effect| match effect {
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
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset: 0,
        data: long[..KEPT].to_vec(),
    });
    sim.deliver(Event::PeerDisconnected { device: phone() });
    sim.restart();

    let answered = answered_for(&mut sim, vec![entry]);
    assert_eq!(
        reads_in(&answered),
        2,
        "a megabyte and a half should take two reads and no more"
    );
    assert_eq!(
        resume_offset_in(&answered),
        Some(KEPT as u64),
        "the prefix was not rebuilt to the watermark"
    );
    agreed(&sim);
}
