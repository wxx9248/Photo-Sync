//! Acceptance scenarios: a session written down, and what it should leave behind.
//!
//! A scenario is the readable face of the harness. A person can check one against `SPEC.md`
//! without reading Rust, and the same file runs against the simulator today and against the
//! real stack when milestone M1 finishes it.
//!
//! Deserialization is strict. A misspelled key is an error naming the file, never a field
//! quietly ignored, because a scenario that silently drops half its setup still passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use photo_sync_core::id::{DeviceId, DevicePath, Sha256, Timestamp, VaultName};
use photo_sync_core::store::DeviceFileRow;
use photo_sync_core::{CivilTime, Moment, RunningDigest};
use photo_sync_sim::{Phone, PhoneFile, Simulation};
use photo_sync_testclient::real::Prepared;
use serde::Deserialize;

/// The moment a scenario's desktop believes it is running at, unless it says otherwise.
const DEFAULT_NOW: &str = "2026-09-07 23:00:00";

/// One device is enough for every scenario milestone M1 needs.
const DEVICE: &str = "phone-a";

/// Which desktop a scenario is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Against {
    /// The simulator: milliseconds, and the only place a crash can be arranged.
    Simulation,

    /// A desktop that really runs, with a socket, a directory, and two databases. This
    /// verifies the adapters the simulator replaces, not the rules.
    RealStack,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    pub id: String,
    pub description: String,

    #[serde(default)]
    pub covers: Vec<String>,

    /// The desktop clock, as `yyyy-MM-dd HH:mm:ss` local time.
    #[serde(default)]
    pub now: Option<String>,

    #[serde(default)]
    pub phone: Phone_,

    #[serde(default)]
    pub desktop: Desktop_,

    #[serde(default)]
    pub events: Vec<ScenarioEvent>,

    pub expect: Expect,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Phone_ {
    #[serde(default)]
    pub library: Vec<LibraryEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryEntry {
    pub path: String,
    pub size: u64,
    pub mtime: i64,

    /// Names the bytes. Two entries naming the same content hold the same photograph, which
    /// is how a scenario says "these are duplicates" without writing megabytes down.
    pub content: String,

    /// The capture time the desktop would read out of it, as `yyyy-MM-dd HH:mm:ss`.
    #[serde(default)]
    pub exif_datetime: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Desktop_ {
    #[serde(default)]
    pub index: Index,

    #[serde(default)]
    pub vault: Vault,

    /// Bytes the vault has room for. Absent means as much as anyone could want.
    #[serde(default)]
    pub free_space: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Index {
    #[serde(default)]
    pub device_file: Vec<IndexRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexRow {
    pub device_path: String,
    pub size: u64,
    pub mtime: i64,
    pub content: String,
    pub vault_name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Vault {
    /// Vault names the desktop still holds. A name the index records but this list omits is a
    /// photo the user curated away.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum ScenarioEvent {
    /// The phone runs one whole session, from the handshake to the summary.
    RunSession,

    /// The phone sends this many bytes of the first file it is asked for and then vanishes.
    InterruptTransfer { after_bytes: u64 },

    /// The power goes out and the desktop comes back.
    Restart,
}

/// What the scenario says should be true afterwards. Every list is compared in full and in
/// order, so an unexpected extra is a failure as much as a missing one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expect {
    #[serde(default)]
    pub uploaded: Vec<String>,

    #[serde(default)]
    pub vault: Vec<String>,

    #[serde(default)]
    pub deletion_candidates: Vec<String>,

    #[serde(default)]
    pub phone_deleted: Vec<String>,

    #[serde(default)]
    pub phone_kept: Vec<String>,

    /// Whether the desktop turned the session away.
    #[serde(default)]
    pub rejected: bool,
}

/// What actually happened, in the same shape as [`Expect`].
#[derive(Debug, Default, PartialEq, Eq)]
struct Observed {
    uploaded: Vec<String>,
    vault: Vec<String>,
    deletion_candidates: Vec<String>,
    phone_deleted: Vec<String>,
    phone_kept: Vec<String>,
    rejected: bool,
}

/// Why a scenario could not be checked here.
enum Trouble {
    /// It describes a world only the simulator can arrange.
    OnlySimulated(String),

    /// Something else went wrong, which is a failure like any other.
    Failed(String),
}

pub(crate) struct Outcome {
    pub id: String,
    pub description: String,

    /// Set when this scenario was not checked, and why.
    pub skipped: Option<String>,

    /// The requirements this scenario claims. A failure puts each of them in doubt, so they
    /// are named alongside it rather than left for the reader to look up.
    pub covers: Vec<String>,

    pub failures: Vec<String>,
}

impl Outcome {
    #[must_use]
    pub(crate) fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    #[must_use]
    pub(crate) fn was_checked(&self) -> bool {
        self.skipped.is_none()
    }
}

/// Runs every scenario in the directory, in name order.
pub(crate) fn run_all(directory: &Path, against: Against) -> Result<Vec<Outcome>, String> {
    let mut outcomes = Vec::new();
    for file in files(directory)? {
        outcomes.push(check(&load(&file)?, against));
    }
    Ok(outcomes)
}

/// Runs the one scenario with this identifier.
pub(crate) fn run_one(directory: &Path, id: &str, against: Against) -> Result<Outcome, String> {
    for file in files(directory)? {
        let scenario = load(&file)?;
        if scenario.id == id {
            return Ok(check(&scenario, against));
        }
    }
    Err(format!("no scenario has the identifier {id}"))
}

fn files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    found.sort();
    Ok(found)
}

fn load(file: &Path) -> Result<Scenario, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    toml::from_str(&text).map_err(|error| format!("{}: {error}", file.display()))
}

fn check(scenario: &Scenario, against: Against) -> Outcome {
    let observed = match observe(scenario, against) {
        Ok(observed) => observed,
        Err(Trouble::OnlySimulated(reason)) => {
            let mut outcome = outcome_of(scenario, Vec::new());
            outcome.skipped = Some(reason);
            return outcome;
        }
        Err(Trouble::Failed(reason)) => return outcome_of(scenario, vec![reason]),
    };

    let mut failures = Vec::new();
    compare(
        "uploaded",
        &scenario.expect.uploaded,
        &observed.uploaded,
        &mut failures,
    );
    compare(
        "vault",
        &scenario.expect.vault,
        &observed.vault,
        &mut failures,
    );
    compare(
        "deletion_candidates",
        &scenario.expect.deletion_candidates,
        &observed.deletion_candidates,
        &mut failures,
    );
    compare(
        "phone_deleted",
        &scenario.expect.phone_deleted,
        &observed.phone_deleted,
        &mut failures,
    );
    compare(
        "phone_kept",
        &scenario.expect.phone_kept,
        &observed.phone_kept,
        &mut failures,
    );
    if scenario.expect.rejected != observed.rejected {
        failures.push(format!(
            "rejected: expected {}, observed {}",
            scenario.expect.rejected, observed.rejected
        ));
    }

    outcome_of(scenario, failures)
}

fn outcome_of(scenario: &Scenario, failures: Vec<String>) -> Outcome {
    Outcome {
        id: scenario.id.clone(),
        description: scenario.description.clone(),
        skipped: None,
        covers: scenario.covers.clone(),
        failures,
    }
}

/// Prints a failure the way `docs/VERIFICATION.md` asks: what was being checked, what the
/// requirements were, and what differed.
pub(crate) fn print_failure(outcome: &Outcome) {
    println!("{} failed — {}", outcome.id, outcome.description);
    if !outcome.covers.is_empty() {
        println!("  in doubt: {}", outcome.covers.join(", "));
    }
    for failure in &outcome.failures {
        println!("  {failure}");
    }
}

fn compare(field: &str, expected: &[String], observed: &[String], into: &mut Vec<String>) {
    if expected != observed {
        into.push(format!(
            "{field}: expected {expected:?}, observed {observed:?}"
        ));
    }
}

/// The world a scenario describes, in the shape both desktops need.
struct World {
    phone: Phone,
    imported: Vec<DeviceFileRow>,
    vault: Vec<(VaultName, Vec<u8>)>,
    captures: Vec<(Vec<u8>, CivilTime)>,
    now: CivilTime,
}

fn world_of(scenario: &Scenario) -> Result<World, String> {
    let now = civil(scenario.now.as_deref().unwrap_or(DEFAULT_NOW))?;
    let device = DeviceId::new(DEVICE);
    let mut phone = Phone::new(DEVICE, "Scenario phone");
    let mut captures = Vec::new();

    for entry in &scenario.phone.library {
        let bytes = bytes_of(&entry.content, entry.size);
        if let Some(captured) = &entry.exif_datetime {
            captures.push((bytes.clone(), civil(captured)?));
        }
        phone.files.insert(
            DevicePath::new(&entry.path),
            PhoneFile {
                mtime: Timestamp(entry.mtime),
                content: bytes,
            },
        );
    }

    let mut known: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut imported = Vec::new();
    for row in &scenario.desktop.index.device_file {
        let bytes = bytes_of(&row.content, row.size);
        imported.push(DeviceFileRow {
            device: device.clone(),
            path: DevicePath::new(&row.device_path),
            size: row.size,
            mtime: Timestamp(row.mtime),
            digest: digest_of(&bytes),
            vault_name: VaultName::new(&row.vault_name),
            committed_at: Timestamp(row.mtime),
        });
        known.insert(row.vault_name.clone(), bytes);
    }

    let mut vault = Vec::new();
    for name in &scenario.desktop.vault.files {
        let bytes = known
            .get(name)
            .ok_or_else(|| format!("the vault lists {name}, which no index row names"))?;
        vault.push((VaultName::new(name), bytes.clone()));
    }

    Ok(World {
        phone,
        imported,
        vault,
        captures,
        now,
    })
}

fn observe(scenario: &Scenario, against: Against) -> Result<Observed, Trouble> {
    let world = world_of(scenario).map_err(Trouble::Failed)?;
    match against {
        Against::Simulation => in_simulation(scenario, world),
        Against::RealStack => on_the_real_stack(scenario, world),
    }
}

fn in_simulation(scenario: &Scenario, mut world: World) -> Result<Observed, Trouble> {
    let mut sim = Simulation::new(Moment {
        at: Timestamp(instant_of(world.now)),
        local: world.now,
    });
    if let Some(free) = scenario.desktop.free_space {
        sim.free_space = free;
    }
    for (content, captured) in &world.captures {
        sim.storage.set_capture_time(content, *captured);
    }
    for row in &world.imported {
        let copy = world
            .vault
            .iter()
            .find(|(name, _)| *name == row.vault_name)
            .map(|(_, bytes)| bytes.as_slice());
        sim.remember_import(row.clone(), copy);
    }

    sim.start();
    sim.take_log();

    let mut observed = Observed::default();
    play(&scenario.events, &mut world.phone, &mut sim, &mut observed)?;
    observed.vault = sim
        .storage
        .vault()
        .keys()
        .map(ToString::to_string)
        .collect();

    // The model watched the same session and worked out what must be true. A scenario says
    // what a person expected; this says what follows from the rules whether anyone wrote it
    // down or not.
    if let Err(differences) = sim.agree() {
        return Err(Trouble::Failed(format!(
            "the desktop and the model disagree: {}",
            differences.join("; ")
        )));
    }
    Ok(observed)
}

fn on_the_real_stack(scenario: &Scenario, mut world: World) -> Result<Observed, Trouble> {
    // A scenario that fixes the desktop clock or the free space cannot be arranged on a real
    // machine, and saying so is better than quietly checking something else.
    if scenario.now.is_some() {
        return Err(Trouble::OnlySimulated(
            "it fixes the desktop clock".to_string(),
        ));
    }
    if scenario.desktop.free_space.is_some() {
        return Err(Trouble::OnlySimulated(
            "it fixes how much room the vault has".to_string(),
        ));
    }
    if scenario
        .phone
        .library
        .iter()
        .any(|entry| entry.exif_datetime.is_some())
    {
        // Nothing reads a capture time out of a photograph yet, so a scenario that states one
        // can only be arranged where the desktop is told. `STACK.md` §3.8 names what is
        // missing.
        return Err(Trouble::OnlySimulated(
            "it states a capture time, and nothing reads one out of a photograph yet".to_string(),
        ));
    }

    let scratch = Scratch::new(&scenario.id);
    let prepared = Prepared::in_directory(&scratch.path);
    prepared
        .remember(&world.imported, &world.vault)
        .map_err(Trouble::Failed)?;
    let running = prepared.start().map_err(Trouble::Failed)?;

    let mut observed = Observed::default();
    let mut connected = running
        .connect(&DeviceId::new(DEVICE), "Scenario phone")
        .map_err(Trouble::Failed)?;
    let played = play(
        &scenario.events,
        &mut world.phone,
        &mut connected,
        &mut observed,
    );

    observed.vault = running.vault_names();
    running.stop();
    played?;
    Ok(observed)
}

fn play(
    events: &[ScenarioEvent],
    phone: &mut Phone,
    driver: &mut impl photo_sync_sim::Driver,
    observed: &mut Observed,
) -> Result<(), Trouble> {
    for event in events {
        match event {
            ScenarioEvent::RunSession => {
                let outcome = phone.run_session(driver);
                observed.rejected |= outcome.rejected.is_some();
                observed.uploaded.extend(named(&outcome.uploaded));
                observed
                    .deletion_candidates
                    .extend(outcome.offered.iter().map(|one| one.path.to_string()));
                observed.phone_deleted.extend(named(&outcome.deleted));
                observed.phone_kept.extend(named(&outcome.kept));
            }
            // What arrived before the connection died is not counted as uploaded: the file
            // is still owed, and the next session is what settles it.
            ScenarioEvent::InterruptTransfer { after_bytes } => {
                let outcome = phone.send_partly(driver, *after_bytes);
                observed.rejected |= outcome.rejected.is_some();
            }
            ScenarioEvent::Restart => {
                if !driver.power_cycle() {
                    return Err(Trouble::OnlySimulated(
                        "it takes the power away from the desktop".to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// A directory of its own for one scenario, removed when it ends.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(id: &str) -> Self {
        let path = std::env::temp_dir().join(format!("photo-sync-{id}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn named(paths: &[DevicePath]) -> Vec<String> {
    paths.iter().map(ToString::to_string).collect()
}

/// The bytes a named content stands for. The same name and size always give the same bytes,
/// so two library entries naming one content really are duplicates.
fn bytes_of(name: &str, size: u64) -> Vec<u8> {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for byte in name.bytes() {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let mut bytes = Vec::with_capacity(size as usize);
    while bytes.len() < size as usize {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(size as usize);
    bytes
}

fn digest_of(bytes: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(bytes);
    running.peek()
}

/// Reads `yyyy-MM-dd HH:mm:ss`, which is how a scenario writes a wall clock.
fn civil(text: &str) -> Result<CivilTime, String> {
    let complaint = || format!("{text} is not a time of the form yyyy-MM-dd HH:mm:ss");
    let (date, time) = text.split_once(' ').ok_or_else(complaint)?;
    let date: Vec<&str> = date.split('-').collect();
    let time: Vec<&str> = time.split(':').collect();
    if date.len() != 3 || time.len() != 3 {
        return Err(complaint());
    }

    let number = |field: &str| field.parse::<i64>().map_err(|_| complaint());
    Ok(CivilTime {
        year: i32::try_from(number(date[0])?).map_err(|_| complaint())?,
        month: u8::try_from(number(date[1])?).map_err(|_| complaint())?,
        day: u8::try_from(number(date[2])?).map_err(|_| complaint())?,
        hour: u8::try_from(number(time[0])?).map_err(|_| complaint())?,
        minute: u8::try_from(number(time[1])?).map_err(|_| complaint())?,
        second: u8::try_from(number(time[2])?).map_err(|_| complaint())?,
    })
}

/// An instant for a wall-clock reading, treating it as UTC.
///
/// A scenario has no timezone, and the only thing the instant decides is the `committed_at`
/// column, which nothing compares. Naming this here keeps that assumption in one place.
fn instant_of(time: CivilTime) -> i64 {
    let years = i64::from(time.year) - 1970;
    let days = years * 365 + years / 4 + i64::from(time.day);
    days * 86_400
        + i64::from(time.hour) * 3_600
        + i64::from(time.minute) * 60
        + i64::from(time.second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_name_always_stands_for_the_same_bytes() {
        assert_eq!(bytes_of("alpha", 64), bytes_of("alpha", 64));
        assert_ne!(bytes_of("alpha", 64), bytes_of("beta", 64));
    }

    #[test]
    fn content_is_generated_to_the_size_that_was_asked_for() {
        assert_eq!(bytes_of("alpha", 100).len(), 100);
        assert_eq!(bytes_of("alpha", 0).len(), 0);
    }

    #[test]
    fn a_wall_clock_is_read_field_by_field() {
        let parsed = civil("2026-08-01 12:34:56");

        assert_eq!(
            parsed,
            Ok(CivilTime {
                year: 2026,
                month: 8,
                day: 1,
                hour: 12,
                minute: 34,
                second: 56,
            })
        );
    }

    #[test]
    fn a_time_that_is_not_one_is_an_error_naming_it() {
        let parsed = civil("last tuesday");

        assert!(parsed.is_err());
    }
}
