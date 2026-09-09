# Handoff

What an agent could not check from inside this repository, and what is worth a person's
attention before the next milestone. This is a working note rather than an authority: nothing
here overrides `SPEC.md`, and anything that becomes a rule belongs in the documents
`AGENTS.md` lists.

Written at the close of milestone M1, and added to as M2 goes.

## 1. Run these once

```sh
./verify doctor          # what this machine can and cannot run
./verify full            # every check, including the real stack
./verify mutants         # does the suite detect a wrong core? expect about 0.98
./verify scenario S-XFER-001 --real
```

`./verify full` takes seconds and `./verify mutants` a few minutes.

## 2. Decisions made without you

Each of these was a fork where the documents did not say, or said something that could not be
built. They are all recorded in the commit that made them; this is the short list.

| Decision | Where it is written down |
|---|---|
| The desktop hashes the bytes that arrive, not the bytes read back from disk | `core/src/digest.rs` |
| Staging identifiers come from a counter, not the `Rng` port, which the core cannot reach | `VERIFICATION.md` §2 |
| A wall clock reaches the core already converted, because a timezone is ambient state | `core/src/naming.rs` |
| A stream is bounded by the size its catalog entry declared | `SPEC.md` §6.5, `R-XFER-005` |
| The exact pairing-code derivation, which the phone half has to match | `STACK.md` §4.6 |
| `rusqlite` 0.39 rather than 0.40, because refinery links SQLite and stops at 0.39 | `STACK.md` §3.5 |
| `x509-parser` reads the peer's public key out of its certificate | `STACK.md` §3.6 |
| Adding a scenario, and turning a requirement on, no longer need a specification change | `xtask/src/checks.rs` |

One choice is in code but not yet in `STACK.md`: rustls uses **`aws-lc-rs`**, its own default
provider, which needs a C toolchain at build time. If that is wrong for the packaging of M9,
it is one line to change and belongs in §3.6 either way.

## 3. What could not be checked here

**There is no application yet.** `desktop/crates/shell/src/main.rs` still prints a version and
exits. Everything works, but only when a test starts it: nothing reads a configuration file,
opens a vault at the configured path, or listens on a real port outside a test. That is the
first thing M2 or M7 should fix, and until it is fixed nobody can try this by hand.

**The Android half does not exist.** `./verify doctor` reports the SDK missing on this machine.
The Kotlin session module needs only a JDK and can be written before the SDK exists.

**A real phone on a real network.** Multicast does not cross this machine's interface, so
discovery against a real device has never run. `DEVELOPMENT.md` already says this belongs on
your desktop.

**Anything on a screen.** `R-PAIR-001` is active and its code is produced, compared between
both ends, and refused when a person says the codes differ. That the six digits appear on a
screen is M7 and is reviewed by hand, as `VERIFICATION.md` §7 says rendering always is.

**Crash behaviour.** Nothing here has ever been interrupted. The commit sequence is written so
that a crash between any two steps is survivable, and the ordering is asserted, but the claim
itself is unchecked until the simulator of M2 can stop the process at an effect boundary.

## 4. Please check these yourself

**The JDK.** `DEVELOPMENT.md` installs `java-21-openjdk-devel` and Fedora 44 has no such
package; only 25 and a 27 preview. Because dnf resolves a transaction as a whole, that one name
makes the whole documented setup command fail. This machine has 25. The document pins 21
because the Android Gradle plugin supports it, so the fix is a decision for M8 rather than a
new version number, and the setup command stays broken until it is made.

**The private key.** `~/.local/share/photo-sync/key.der` is written readable by its owner alone
and a test asserts that. Worth confirming once on a real machine, since a umask is the sort of
thing that differs.

**Pairing has no deadline.** A window a person opens stays open until a phone pairs or someone
closes it. `SPEC.md` §5.2 does not ask for a timeout, but a desktop left open all afternoon is
a desktop that will pair with whatever asks first, and that is a decision worth making on
purpose.

**The pairing code is a cross-language contract.** `STACK.md` §4.6 now writes the derivation
out in full and a test anchors the six digits it produces. The Kotlin half has to arrive at the
same answer. The conformance vector that would prove it lands in M8; a vector only one
implementation reads is not yet doing the job vectors exist for.

## 5. Where M2 stands

Its exit criteria are met. `./verify self-test` catches four known-bad desktops and names the
test that caught each, a 256-seed campaign runs clean in the full tier, and R-RECOVER-001
through R-RECOVER-004 are active with passing tests. Startup recovery and write-log replay
landed with them, which `docs/ROADMAP.md` had listed under M4.

**Two of its work items are not done, so I have not called it closed.** The SQLite virtual
file system is the larger one: until it exists, a crash treats both databases as intact, which
is sound for WAL plus `synchronous=FULL` but does not model a crash inside SQLite's own
writing. The other is the rest of the fault injection `VERIFICATION.md` §L1 lists — a short
write, `ENOSPC` at an arbitrary point, and above all the silent no-op fsync, which is a
negative control in its own right: a suite that still passes against a filesystem that only
pretends to sync is a suite that was never testing durability.

Whether that makes M2 closed or not is yours to say.

## 6. What M2 has changed so far

The simulated filesystem now loses what a real one loses: a file's bytes become durable when
that file is synced, a name when its directory is synced, and a crash keeps nothing else. A
crash also leaves a file at whatever length a buffered write had moved it to, with the tail
past the durable data coming back as zeroes.

It found two things on its first runs, and one of them changed behaviour.

**A partial never survived a power loss.** Nothing synced the staging directory after creating
one, so the file's name was never durable and `SPEC.md` §7.6's instruction to cut every `.part`
file back to its watermark had nothing to act on: every interrupted transfer would have started
from zero. The desktop now syncs that directory when it creates a partial. **This costs one
directory fsync per file** — about 1,180 of them for the library §3.4 describes — and buys back
every interrupted transfer. If that trade is wrong for your disks, it is one effect to remove.

**Staging is cleared but not durably.** §7.3 step 3 clears the write-log, the manifest, and then
the files, and nothing syncs the staging directory afterwards. The manifest and log are in
SQLite and so are durable, but a crash immediately after a commit can bring the files back with
no rows describing them. Nothing is lost and nothing is wrongly deleted; the files are orphaned
and nothing sweeps them up. Whether that wants a directory sync at the end of §7.3 or a sweep at
startup is a decision, and it is the sort of thing the campaign will keep finding until it is
made.

**One approximation to know about.** A crash discards what the filesystem had not written down,
and treats both databases as intact. `STACK.md` §3.5 runs them in WAL mode with
`synchronous=FULL`, so a transaction that returned did survive and one that did not never
existed; what this does not yet model is a crash inside SQLite's own writing. That is what the
virtual file system M2 also calls for is for, and it is not built.

## 7. Known gaps, deliberately left

These are noted where the code makes them, not just here.

* Nothing reads a capture time out of a photograph, so a vault name falls back to the file's
  modification time. Both `SPEC.md` §7.2 sources above it need `nom-exif`, and the conversion
  to local time needs a timezone database. They land together.
* Every wall-clock reading is UTC for the same reason. Names are cosmetic, and §7.2 says so.
* `R-XFER-002`, applying the phone's modification time to the stored copy, has no effect in the
  vocabulary and is not done.
* Resuming a partial across a desktop restart is refused rather than trusted: the desktop
  cannot continue a digest it no longer holds, so it asks for the file again. `R-STAGE-009`
  and M3 fix that.
* A commit that halts part-way leaves the device locked until recovery replays its write-log,
  and recovery is M4. Nothing is lost, but that phone cannot sync again until then.
