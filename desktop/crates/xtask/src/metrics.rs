//! Numbers to watch, not gates. `docs/VERIFICATION.md` §5.
//!
//! These fail nothing. Timing on a machine that is also doing other things is noisy, and a
//! suite that goes red because somebody started a build would teach people to ignore it. What
//! they are for is telling one kind of slowness from another: the phone, the network and the
//! desktop all look the same from the outside, and when a transfer got slower somebody has to
//! know which of them to look at.
//!
//! Everything here runs against the real desktop over a real socket, with a phone that is a
//! program in this process --- no MediaStore, no Wi-Fi, no radio. What is left is the
//! desktop's own cost, which is the half a repository can measure.

use std::path::{Path, PathBuf};
use std::time::Instant;

use photo_sync_core::event::Event;
use photo_sync_core::id::DeviceId;
use photo_sync_sim::{Phone, PhoneFile};
use photo_sync_testclient::real::Prepared;

use crate::report::Metric;
use crate::{tools, workspace};

/// The mtime every made-up photograph carries. Any fixed value does.
const MTIME: i64 = 1_756_000_000;

/// How many photographs the per-file measurement sends, and how large each is.
///
/// A megabyte is about what a camera makes, and a hundred and fifty of them is long enough to
/// average over and short enough that a nightly still finishes. What this measures is
/// per-file cost: on a spinning disk it is almost entirely the durability points each file
/// needs.
const TRANSFER_FILES: usize = 150;
const TRANSFER_FILE_BYTES: usize = 1_000_000;

/// One large photograph, for the per-byte cost instead: hashing, writing, and TLS.
///
/// `docs/VERIFICATION.md` §5 asked for two gigabytes. A quarter of that says the same thing
/// on a machine whose vault is a spinning disk, and leaves the nightly time for the rest.
const BULK_BYTES: usize = 256 * 1024 * 1024;

/// How large a library the diff is timed against. §6.2 has to hold this whole catalog.
const CATALOG_ENTRIES: usize = 50_000;

/// How many staged files the commit is timed over.
///
/// §7.3 claims a commit is pure metadata work. That is the claim this watches. The document
/// asks for ten thousand; a thousand is what fits, because staging each one first costs this
/// machine's disk four durability points whatever the file's size.
const COMMIT_ENTRIES: usize = 1_000;

/// How far a number may move before it is worth a person's attention.
///
/// A quarter. These run on whatever machine is free, beside whatever else it is doing, and a
/// tighter band would cry wolf often enough that nobody would read it.
const TOLERANCE: f64 = 0.25;

/// Measures, compares with the last accepted run, and says what moved.
///
/// The harness itself is built without optimisation, and a desktop built that way answers
/// several times slower than the one people install --- a number from it would be a number
/// about `cargo build`. So this hands the work to a release build of itself and reads back
/// what that one measured.
pub(crate) fn run(root: &Path) -> Result<Vec<Metric>, String> {
    if cfg!(debug_assertions) {
        return in_a_release_build(root);
    }
    measure_and_compare(root)
}

/// Runs the measurement again, optimised, and reads what it wrote.
fn in_a_release_build(root: &Path) -> Result<Vec<Metric>, String> {
    println!("metrics: building the desktop with optimisation, which is what it is measuring");
    let desktop = root.join("desktop");
    let ran = tools::run(
        "cargo",
        &[
            "run",
            "--release",
            "--quiet",
            "--package",
            "xtask",
            "--",
            "metrics",
            "--measure",
        ],
        &desktop,
    )?;
    if !ran {
        return Err("the optimised measurement did not finish".to_string());
    }

    let path = latest_file(root);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("cannot read the measurements: {error}"))
}

/// The measurement itself, which only ever runs in an optimised build.
///
/// The scratch directory sits beside the repository rather than in `/tmp`, which on many
/// machines is memory: a vault on tmpfs answers about six times faster than one on a disk,
/// and measuring that would be measuring the wrong machine.
pub(crate) fn measure_and_compare(root: &Path) -> Result<Vec<Metric>, String> {
    let scratch = workspace::metrics_directory(root).join("scratch");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;

    let baseline = read_baseline(&baseline_file(root));
    let mut measured = measure(&scratch)?;
    let _ = std::fs::remove_dir_all(&scratch);

    for metric in &mut measured {
        metric.against(baseline.get(&metric.name).copied(), TOLERANCE);
        let moved = if metric.regressed { "  <-- moved" } else { "" };
        println!(
            "metrics: {} {:.2} {}{moved}",
            metric.name, metric.value, metric.unit
        );
    }
    if measured.iter().any(|metric| metric.regressed) {
        println!(
            "metrics: something got slower. Nothing fails on it; \
             docs/VERIFICATION.md §5 says why, and {} is how to accept a new number.",
            baseline_file(root).display()
        );
    }

    let path = latest_file(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let rendered = serde_json::to_string_pretty(&measured).map_err(|error| error.to_string())?;
    std::fs::write(&path, rendered).map_err(|error| error.to_string())?;

    Ok(measured)
}

/// What the last run measured, whether or not anybody accepted it as the baseline.
fn latest_file(root: &Path) -> PathBuf {
    workspace::metrics_directory(root).join("latest.json")
}

fn read_baseline(path: &Path) -> std::collections::BTreeMap<String, f64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Runs every measurement, in the directory given.
///
/// The directory decides what is being measured as much as the code does: a vault on a
/// memory-backed filesystem answers about six times faster than one on a spinning disk, and
/// both are true.
pub(crate) fn measure(scratch: &Path) -> Result<Vec<Metric>, String> {
    Ok(vec![
        transfer_per_file(&scratch.join("per-file"))?,
        transfer_per_byte(&scratch.join("per-byte"))?,
        diff_of_a_large_library(&scratch.join("diff"))?,
        commit_of_a_batch(&scratch.join("commit"))?,
    ])
}

/// Photographs a second, with the per-file cost in the way.
fn transfer_per_file(root: &Path) -> Result<Metric, String> {
    let mut phone = Phone::new("phone-a", "Kitchen phone");
    for at in 0..TRANSFER_FILES {
        phone = phone.holding(
            &format!("DCIM/Camera/IMG_{at:05}.jpg"),
            PhoneFile::new(MTIME + at as i64, made_up(TRANSFER_FILE_BYTES + at, at)),
        );
    }

    let desktop = start(root)?;
    let mut connected = desktop.connect(&phone.device.clone(), &phone.name.clone())?;
    let started = Instant::now();
    let outcome = phone.run_session(&mut connected);
    let took = started.elapsed().as_secs_f64();
    desktop.stop();

    if outcome.uploaded.len() != TRANSFER_FILES {
        return Err(format!(
            "only {} of {TRANSFER_FILES} photographs crossed",
            outcome.uploaded.len()
        ));
    }
    Ok(Metric::more_is_better(
        "transfer.photographs_per_second",
        TRANSFER_FILES as f64 / took,
        "photographs/s",
    ))
}

/// Megabytes a second, with one photograph large enough that per-file cost stops mattering.
fn transfer_per_byte(root: &Path) -> Result<Metric, String> {
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/VID_0001.mp4",
        PhoneFile::new(MTIME, made_up(BULK_BYTES, 1)),
    );

    let desktop = start(root)?;
    let mut connected = desktop.connect(&phone.device.clone(), &phone.name.clone())?;
    let started = Instant::now();
    let outcome = phone.run_session(&mut connected);
    let took = started.elapsed().as_secs_f64();
    desktop.stop();

    if outcome.uploaded.len() != 1 {
        return Err("the large photograph did not cross".to_string());
    }
    Ok(Metric::more_is_better(
        "transfer.megabytes_per_second",
        BULK_BYTES as f64 / 1e6 / took,
        "MB/s",
    ))
}

/// How long a large library takes to submit and diff, which §6.1 and §6.2 do once a session.
fn diff_of_a_large_library(root: &Path) -> Result<Metric, String> {
    let device = DeviceId::new("phone-a");
    let entries: Vec<photo_sync_core::catalog::CatalogEntry> = (0..CATALOG_ENTRIES)
        .map(|at| photo_sync_core::catalog::CatalogEntry {
            path: photo_sync_core::id::DevicePath::new(format!("DCIM/Camera/IMG_{at:06}.jpg")),
            size: 1_000_000 + at as u64,
            mtime: photo_sync_core::id::Timestamp(MTIME + at as i64),
        })
        .collect();
    let total_bytes = entries.iter().map(|entry| entry.size).sum();

    let desktop = start(root)?;
    let mut connected = desktop.connect(&device, "Kitchen phone")?;

    use photo_sync_sim::Driver;
    connected.deliver(Event::PeerConnected {
        device: device.clone(),
        name: "Kitchen phone".to_string(),
    });
    connected.take_log();

    let started = Instant::now();
    connected.deliver(Event::CatalogSubmitted {
        device: device.clone(),
        entries,
        total_bytes,
    });
    connected.deliver(Event::DiffRequested {
        device: device.clone(),
    });
    connected.take_log();
    let took = started.elapsed().as_secs_f64();
    desktop.stop();

    Ok(Metric::less_is_better(
        &format!("diff.seconds_for_{CATALOG_ENTRIES}_entries"),
        took,
        "s",
    ))
}

/// How long the commit of a batch takes, which §7.3 says is metadata work and nothing else.
fn commit_of_a_batch(root: &Path) -> Result<Metric, String> {
    let mut phone = Phone::new("phone-a", "Kitchen phone");
    for at in 0..COMMIT_ENTRIES {
        // Small on purpose: what is being timed is the renaming and the index, not the bytes.
        phone = phone.holding(
            &format!("DCIM/Camera/IMG_{at:05}.jpg"),
            PhoneFile::new(MTIME + at as i64, made_up(64 + at, at)),
        );
    }

    let desktop = start(root)?;
    let mut connected = desktop.connect(&phone.device.clone(), &phone.name.clone())?;
    phone.run_session_until_commit(&mut connected);

    use photo_sync_sim::Driver;
    let started = Instant::now();
    connected.deliver(Event::FinishRequested {
        device: phone.device.clone(),
    });
    connected.take_log();
    let took = started.elapsed().as_secs_f64();
    desktop.stop();

    Ok(Metric::less_is_better(
        &format!("commit.seconds_for_{COMMIT_ENTRIES}_entries"),
        took,
        "s",
    ))
}

fn start(root: &Path) -> Result<photo_sync_testclient::real::Running, String> {
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    Prepared::in_directory(root).start()
}

/// Bytes that differ from every other file's, so nothing is deduplicated away.
fn made_up(length: usize, seed: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; length];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = (at.wrapping_mul(seed + 1).wrapping_add(seed) % 251) as u8;
    }
    bytes
}

/// Where the measurements are kept between runs, so one can be compared with the last.
pub(crate) fn baseline_file(root: &Path) -> PathBuf {
    workspace::metrics_directory(root).join("baseline.json")
}
