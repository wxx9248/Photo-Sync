//! Sessions nobody wrote down.
//!
//! A campaign makes up things a family might do and checks that the desktop and the model
//! still agree about what it holds. Scenarios say what a person expected; this says what
//! follows from the rules whether anyone thought of the case or not.
//!
//! One seed reproduces one run exactly, on any machine, with one command. A run that fails is
//! cut down to the shortest sequence that still fails before it is reported, because the
//! shortest one is the one somebody can read.

use photo_sync_core::event::Event;
use photo_sync_core::id::{DeviceId, VaultName};
use photo_sync_core::{CivilTime, Moment, Timestamp};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRng, TestRunner};

use crate::phone::{Phone, PhoneFile};
use crate::run::Simulation;

/// The moment every campaign believes it is running at.
const NOW: Moment = Moment {
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

/// The phones a run has to choose between. `SPEC.md` §7.3 serializes commits across
/// devices and dedups one device's content against another's through the index, so a
/// campaign with one phone leaves both of those claims to hand-written tests.
const DEVICES: [&str; 2] = ["phone-a", "phone-b"];

/// The alphabet a run is made of. `docs/VERIFICATION.md` §L3 lists what belongs here; these
/// are the ones the simulator can arrange today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A photograph is taken. The number decides its contents, so the same number twice is
    /// the same photograph twice.
    Photograph(u8),

    /// A photograph already on the phone is edited, which replaces it.
    Edit(u8),

    /// One photograph ends up at a second path, which is how the phone comes to hold the
    /// same content twice and the only way the deduplication of §7.3 is ever reached.
    Copy(u8, u8),

    /// The camera writes over a photograph after the desktop has answered the diff, so the
    /// file that arrives is not the one the session agreed to take.
    EditWhileSending(u8),

    /// The camera writes over a photograph after the desktop has offered it for deletion,
    /// which is what the two gates of `SPEC.md` §8 stand between.
    EditBeforeDeleting(u8),

    /// Everything after this addresses the other phone. A run is a family with two of them,
    /// and which one is holding the desktop's attention is the interesting part.
    OtherPhone,

    /// The phone syncs.
    Sync,

    /// The power goes out and the machine comes back.
    Restart,

    /// The power goes out part-way through a commit. The number says how many of the
    /// commit's steps happened before it did, so a seed can stop anywhere in §7.3.
    CrashCommitting(u8),

    /// Somebody deletes a photograph out of the vault.
    Curate(u8),

    /// The disk fills part-way through the next session. `SPEC.md` §4 makes that an ordinary
    /// receive error on the file it happens to, rather than anything the desktop must undo.
    FillDisk(u8),
}

fn operations() -> impl Strategy<Value = Vec<Op>> {
    let one = prop_oneof![
        6 => (0u8..6).prop_map(Op::Photograph),
        2 => (0u8..6).prop_map(Op::Edit),
        2 => (0u8..6).prop_map(Op::EditWhileSending),
        2 => (0u8..6).prop_map(Op::EditBeforeDeleting),
        3 => ((0u8..6), (0u8..6)).prop_map(|(from, to)| Op::Copy(from, to)),
        6 => Just(Op::Sync),
        3 => Just(Op::OtherPhone),
        2 => Just(Op::Restart),
        2 => (0u8..28).prop_map(Op::CrashCommitting),
        1 => (0u8..6).prop_map(Op::Curate),
        2 => (0u8..4).prop_map(Op::FillDisk),
    ];
    proptest::collection::vec(one, 1..14)
}

/// What went wrong, and everything needed to see it again.
#[derive(Clone, Debug)]
pub struct Failure {
    pub seed: u64,
    pub trace: Vec<Op>,
    pub differences: Vec<String>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seed {:#018x}: {} after {:?}",
            self.seed,
            self.differences.join("; "),
            self.trace
        )
    }
}

/// How a seed is turned into a run. Both the running and the reporting of it go through
/// here, because a printed sequence that was drawn differently from the one that ran is a
/// reproduction of nothing.
fn runner_for(seed: u64) -> TestRunner {
    TestRunner::new_with_rng(
        Config {
            cases: 1,
            // A persisted failure would be replayed ahead of this seed's own run, which would
            // make the same token mean different things on different machines.
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::from_seed(proptest::test_runner::RngAlgorithm::ChaCha, &spread(seed)),
    )
}

/// Runs one seed's worth of made-up sessions.
///
/// # Errors
/// When the desktop and the model end up disagreeing, with the shortest sequence that still
/// causes it.
pub fn run(seed: u64) -> Result<(), Failure> {
    let mut runner = runner_for(seed);
    let outcome = runner.run(&operations(), |ops| {
        play(&ops).map_err(|differences| TestCaseError::fail(differences.join("; ")))
    });

    match outcome {
        Ok(()) => Ok(()),
        Err(proptest::test_runner::TestError::Fail(reason, ops)) => Err(Failure {
            seed,
            trace: ops,
            differences: vec![reason.to_string()],
        }),
        Err(proptest::test_runner::TestError::Abort(reason)) => Err(Failure {
            seed,
            trace: Vec::new(),
            differences: vec![reason.to_string()],
        }),
    }
}

/// Plays one sequence and asks the two accounts to agree at every point they should.
fn play(ops: &[Op]) -> Result<(), Vec<String>> {
    let mut sim = Simulation::new(NOW);
    sim.start();
    sim.take_log();

    let mut phones = [
        Phone::new(DEVICES[0], "Kitchen phone"),
        Phone::new(DEVICES[1], "Hallway phone"),
    ];
    let mut at = 0usize;
    let mut generation: u8 = 0;

    for op in ops {
        match op {
            Op::Photograph(which) => {
                phones[at].files.insert(
                    path_for(*which),
                    PhoneFile::new(mtime_for(*which, generation), contents(*which, 0)),
                );
            }
            Op::Edit(which) => {
                let path = path_for(*which);
                if phones[at].files.contains_key(&path) {
                    generation = generation.wrapping_add(1);
                    phones[at].files.insert(
                        path,
                        PhoneFile::new(mtime_for(*which, generation), contents(*which, generation)),
                    );
                }
            }
            Op::Copy(from, to) => {
                if from != to
                    && let Some(held) = phones[at].files.get(&path_for(*from)).cloned()
                {
                    phones[at].files.insert(path_for(*to), held);
                }
            }
            Op::EditWhileSending(which) => {
                let path = path_for(*which);
                if phones[at].files.contains_key(&path) {
                    generation = generation.wrapping_add(1);
                    phones[at].edit_after_diff = Some((
                        path,
                        PhoneFile::new(mtime_for(*which, generation), contents(*which, generation)),
                    ));
                }
                phones[at].run_session(&mut sim);
                phones[at].edit_after_diff = None;
            }
            Op::EditBeforeDeleting(which) => {
                let path = path_for(*which);
                if phones[at].files.contains_key(&path) {
                    generation = generation.wrapping_add(1);
                    phones[at].edit_before_deleting = Some((
                        path,
                        PhoneFile::new(mtime_for(*which, generation), contents(*which, generation)),
                    ));
                }
                phones[at].run_session(&mut sim);
                phones[at].edit_before_deleting = None;
            }
            Op::OtherPhone => at = (at + 1) % DEVICES.len(),
            Op::Sync => {
                phones[at].run_session(&mut sim);
            }
            Op::Restart => sim.restart(),
            Op::CrashCommitting(after) => {
                // Get the photographs across, then take the machine away part-way through
                // putting them into the vault. How far it got is the seed's to choose.
                phones[at].run_session_until_commit(&mut sim);
                sim.deliver_stopping_after(
                    Event::FinishRequested {
                        device: DeviceId::new(DEVICES[at]),
                    },
                    usize::from(*after),
                );
                sim.restart();
            }
            Op::FillDisk(after) => {
                sim.faults.refuse_write_after = Some(usize::from(*after));
                phones[at].run_session(&mut sim);
                sim.faults.refuse_write_after = None;
            }
            Op::Curate(which) => {
                let names: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();
                if let Some(name) = names.get(usize::from(*which) % names.len().max(1)) {
                    sim.curate(name);
                }
            }
        }

        sim.agree()?;
    }
    Ok(())
}

/// Spreads a seed over the thirty-two bytes the generator wants, so a token a person can
/// read and type still names one run exactly.
fn spread(seed: u64) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    let mut state = seed;
    for chunk in bytes.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    bytes
}

fn path_for(which: u8) -> photo_sync_core::DevicePath {
    photo_sync_core::DevicePath::new(format!("DCIM/Camera/IMG_{which:04}.jpg"))
}

fn mtime_for(which: u8, generation: u8) -> i64 {
    1_756_000_000 + i64::from(which) * 100 + i64::from(generation)
}

/// Contents nobody would confuse with another photograph's.
fn contents(which: u8, generation: u8) -> Vec<u8> {
    format!("photograph {which} as of {generation}").into_bytes()
}
