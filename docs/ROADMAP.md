# Build order

The order work happens in, and what each milestone must prove before the next one starts.

The rule behind the order is that the harness comes before the features it checks. Code written
before the simulator exists is code that was never checked by the thing meant to check it, and
the harness then grows around whatever that code assumed.

## How milestones interact with the requirement registry

Every requirement in `verification/requirements.toml` starts as `deferred`. A milestone lists
the requirement IDs it closes, and closing a milestone means flipping those to `active`. Once a
requirement is `active`, `./verify full` fails while it has no passing test. This gives the gate
something to enforce from the first milestone without demanding the whole spec at once.

## Milestones

M0 and M1 are complete. The work in progress is **M2**.

### M0 Foundation

Goal: a repository where work can start and be checked.

Work: directory skeleton, licence, ignore rules, Cargo workspace, Gradle project, the `.proto`
schema, the `Event` and `Effect` vocabulary, the port traits, the `./verify` runner with its
quick tier, the requirement registry, and the pre-push hook.

Exit: `./verify quick` runs and passes. `./verify spec-check` confirms every registry quote
appears in `SPEC.md`. The traceability matrix reports every requirement as deferred. No
requirement is active yet.

### M1 Walking skeleton

Goal: prove every seam before any of them carries weight.

Work: one thin path end to end. A Rust test client pairs with the desktop, sends one file, and
the desktop verifies it, stages it, commits it by rename, and records it. The same core runs
under the simulator with fake ports and under the real shell with sockets and SQLite.

Exit: one acceptance scenario passes in both simulation and real mode. The JSON report is
produced in its final shape. `R-PAIR-001`, `R-XFER-001`, and `R-COMMIT-001` become active.

### M2 Simulation depth

Goal: make the crash claims checkable.

Work: filesystem semantics with durability modelling, the SQLite VFS, crash and restart at any
effect boundary, the executable model with differential comparison, fault injection, proptest
strategies, the seed corpus, and the first sabotage set.

Exit: `./verify self-test` catches at least four known-bad implementations and names the test
that caught each. A 256-seed campaign runs clean. Recovery requirements move from deferred to
active as their tests land.

### M3 Catalog, diff, and transfer

Goal: the desktop can receive a real library.

Work: catalog intake, the three-way diff, concurrent uploads over several channels, streaming
hash verification, staging manifest entries, partial files, watermarks, and resume.

Exit: `R-CATALOG-*`, `R-DIFF-*`, `R-XFER-*`, and `R-STAGE-*` are active and verified. A scenario
covers an interrupted transfer resuming from the desktop watermark.

### M4 Commit, recovery, and the index

Goal: the durability core.

Work: the write-log, the name map, dedup under the commit lock, directory fsync barriers,
done-marks, the staging clear order, replay after a crash, and the SQLite index.

Exit: `R-COMMIT-*`, `R-RECOVER-*`, and `R-INDEX-*` active and verified. The campaign includes
crash points across the whole commit sequence. The sabotage set covers every ordering rule in
`SPEC.md` §7.3.

### M5 Deletion protocol

Goal: the phone can free space safely.

Work: nomination with vault-presence checks, the two verification gates, the candidate stream,
result reporting, and the stale-row escape hatch.

Exit: `R-DELETE-*` active and verified, including a scenario proving a curated vault copy is
never nominated.

### M6 Sessions, discovery, and pairing

Goal: real session behavior between real peers.

Work: mDNS advertising and discovery, the pairing exchange with the short authentication string,
reconnection with the frozen catalog, concurrent devices, and the per-device commit lock under
contention.

Exit: `R-SESSION-*`, `R-DISCOVER-*`, and the rest of `R-PAIR-*` active and verified. The
campaign includes multi-device interleavings.

### M7 Desktop application

Goal: the desktop is usable by the family.

Work: Slint window and views, the tray item, suspend inhibition, autostart, configuration file,
logging, and translations.

Exit: `R-UI-*` active for the behavior that has state behind it. Rendering is reviewed by hand.

### M8 Android application

Goal: the phone half.

Work: the session state machine as a plain Kotlin module with no framework types, then the
Android module around it. MediaStore enumeration, permissions, the foreground service and its
keep-alive stack, deletion requests, Compose screens, and translations.

Depends on an Android SDK being available. The Kotlin session core can be written and tested on
the JVM before the SDK exists.

Exit: JVM tests green against the Kotlin fake desktop, managed-device tests green for the
platform surface, and the conformance vectors passing on both ends.

### M9 Packaging and release

Goal: something installable.

Work: PKGBUILD, the F-Droid repository, install and pairing instructions, and the manual OEM
matrix for the target phones.

Exit: both halves install on a clean machine and phone, and one real session runs end to end.

## Environment

`docs/DEVELOPMENT.md` records where the work runs and how to set a machine up. Two milestones
need something beyond the defaults.

**M6** tests the discovery code against the Rust test client inside the development machine.
Discovery between the desktop and a real phone needs the machine bridged onto the local network,
and that case runs on the desktop instead.

**M8** needs the Android SDK with its platform and emulator images. The emulator is accelerated
there, because the processor is passed through and the host allows nested virtualisation. The
Kotlin session module needs none of this and can be written and tested on any machine with a
JDK.
