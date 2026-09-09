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

M0 through M7 are complete. The work in progress is **M8**.

M2 closed with the SQLite virtual file system outstanding: the crate `STACK.md` §6 chose does
not support the journal mode §3.5 requires. `docs/HANDOFF.md` records what was tried and the
three ways forward.

M3 closed with four of the catalog requirements still deferred. `R-CATALOG-001`, `-002`, `-004`
and `-005` are `SPEC.md` §3.2 — the `DCIM/Camera` bucket, `IS_PENDING`, the frozen snapshot,
and no upfront hashing — which are things the phone does and nothing on the desktop can
observe. They close with M8, where the Kotlin session core is written. `R-CATALOG-003`, the
entry identity the desktop diffs against, is active and verified here.

M4 closed with one ordering rule of `SPEC.md` §7.3 tested but not sabotaged. Index rows go in
before the commit lock is released, which is what lets the next commit's map see them; two
phones that staged one photograph before either committed prove the behaviour. Inverting the
rule needs two commits actually running against one another. M6 closed that: a campaign runs
two phones now, and the control exists.

M5 closed with two of the deletion requirements still deferred. `R-DELETE-005` is the single
prompt the phone shows and `R-DELETE-009` is `MediaStore.createDeleteRequest`; both are things
the phone does, and nothing on this end can see either. They close with M8. The desktop's half
of both — the split between what this session committed and what an earlier one did, which the
prompt is built from, and the candidate list the delete request works through — is active and
verified.

M6 closed with four requirements still deferred, all of them things the phone does and nothing
here can see: `R-SESSION-001` (the Start tap), `R-SESSION-003` (the phone persists nothing about
transfer progress), `R-PAIR-002` (the phone stores exactly one paired desktop), and
`R-DELETE-005`/`R-DELETE-009` from M5. They close with M8.

M7 closed with every `R-UI-*` active. What it cannot close from inside this repository is the
rendering: there is no display on the development machine, so the window has been compiled and
its contents decided in testable Rust, but nobody has looked at it. `docs/HANDOFF.md` §12 says
what to look at and how to run it.

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
