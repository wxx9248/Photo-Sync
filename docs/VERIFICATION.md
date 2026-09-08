# Photo Sync — Verification Architecture

Companion to `SPEC.md` (what the system does) and `STACK.md` (what it is built from). This
document defines the instrument that decides whether an implementation is correct, and the
feedback loop that guides development.

## 1. What this instrument is for

Implementation work on this project is performed by AI agents. That changes what a test suite
has to be. A human developer treats a failing test as a hint and reads the code; an agent needs
the failure itself to carry enough information to act on, and — critically — must not be able to
make the signal go away by weakening the thing producing it.

So the harness is designed around four properties, and every decision below traces back to one
of them:

1. **Reproducible from a token.** Every failure reduces to a seed or a scenario ID that
   reproduces it exactly, on any machine, with one command.
2. **Localized to a clause.** Every failure names the `SPEC.md` requirement it violates, not
   just the assertion that tripped.
3. **Minimizable.** Failures shrink automatically to the shortest trace that still fails.
4. **Resistant to weakening.** The artifacts that define correctness are separated from the
   artifacts that implement it, and the suite is itself tested for its ability to detect wrong
   implementations.

Explicit non-goals: line-coverage percentage is not a target and is not gated; UI rendering is
not verified automatically; the OEM keep-alive behavior of §3.3 cannot be simulated and is
handled by a manual matrix (§7.3).

## 2. Architectural precondition — a sans-I/O core

The harness cannot be more rigorous than the code allows, so the product architecture is
constrained first:

```
shell (adapters)   tokio · tonic · rustls · rusqlite · slint · ksni · mdns-sd
      ↕            events in, effects out — no I/O crosses this line
core  (sans-I/O)   session scheduler · diff · staging manifest · commit state machine ·
                   recovery · dedup · naming · deletion nomination
```

The core consumes `Event` values (bytes arrived, fsync completed, connection dropped, timer
fired, query returned) and emits `Effect` values (write these bytes, fsync this directory,
rename A→B, insert these rows, send this message). It never awaits, never spawns, never reads a
clock, and never opens a file. All of `SPEC.md` §6, §7 and §8 lives there; the shell is a thin
adapter in both directions.

This is the single most important rule in the project, and the one an agent will violate by
default — asked to "write the commit path", a model reaches for `tokio::fs`. It is therefore
enforced mechanically rather than by review:

* the `core` crate's `Cargo.toml` does not depend on `tokio`, `rusqlite`, `tonic`, or any I/O
  crate — the dependency graph is the primary enforcement;
* `clippy.toml` adds `disallowed-types` for `std::fs::*`, `std::time::SystemTime`,
  `std::time::Instant`, and `std::thread`, denied in the core crate;
* determinism rules on top: no iteration over `HashMap`/`HashSet` where order is observable
  (use `BTreeMap`/`BTreeSet` or sort explicitly), no floating point in decisions, no ambient
  randomness. The core reaches no port, so the values it has to invent come from counters it
  can justify instead: a staging identifier continues above the highest the manifest holds,
  read once at startup, and a collision suffix counts up as `SPEC.md` §7.2 describes. Both
  replay from a trace without a seed. The `Rng` port stays for the shell.

The `Effect` enum is not an implementation detail; it is the port surface, and adding a variant
is how a new kind of side effect becomes visible to the simulator, the model, and the fault
injector at once.

## 3. Layers

```
L6  runner · reports · gates · negative controls
L5  acceptance scenarios (TOML)
L4  cross-language conformance (vectors · fake peers · real-bytes E2E)
L3  deterministic simulator (clock · network · filesystem · SQLite VFS · crashes)
L2  executable model (the oracle)
L1  ports and fault injection
L0  requirement registry
```

### L0 — Requirement registry

`verification/requirements.toml` is the machine-readable projection of `SPEC.md`. One entry per
testable clause:

```toml
[[requirement]]
id      = "R-COMMIT-014"
section = "§7.3 step 2"
area    = "COMMIT"
status  = "active"          # active | deferred
quote   = "After each group, fsync the vault and staging directories, then append flushed done-marks for the group."
note    = "The directory fsync must be durable before any done-mark becomes durable."
```

The `quote` field is load-bearing: `verify spec-check` asserts that every quote still appears
**verbatim** in `SPEC.md`, comparing after collapsing runs of whitespace so that a quoted
sentence may span the wrapped lines of the document. Rewording a requirement therefore breaks the build until someone
confirms whether the meaning changed and updates both files together. This is what keeps the
registry an actual projection of the spec rather than a parallel document that drifts. `SPEC.md`
itself stays clean prose with no identifiers embedded in it.

ID scheme: `R-<AREA>-<NNN>` with areas `CATALOG`, `DIFF`, `XFER`, `STAGE`, `NAME`, `COMMIT`,
`RECOVER`, `INDEX`, `DELETE`, `PAIR`, `DISCOVER`, `SESSION`, `UI`, `PERF`. Numbers are never
reused.

Coverage is declared at the test site and extracted statically:

* Rust: `covers!("R-COMMIT-014");` as the first statement of a test body (the macro expands to
  nothing; a small parser in `xtask` associates it with the enclosing `#[test] fn`);
* Kotlin: `@Covers("R-XFER-003")` on the test method;
* Scenarios: a `covers = [...]` key in the TOML.

The runner merges those declarations with the test results and emits the traceability matrix.
**A requirement with `status = "active"` and no passing covering test fails `verify full`.**
Deferring a requirement is allowed but requires changing `status` in the registry, which shows up
in the diff — the point is that not verifying something is a visible act, not an omission.

### L1 — Ports and fault injection

Every side effect the core can emit is served by a port trait in the shell:

| Port | Methods | Injected faults |
|---|---|---|
| `FileOps` | `create`, `write_at`, `sync_data`, `sync_dir`, `rename`, `remove`, `set_len`, `metadata` | fail at operation *N*, `ENOSPC`, `EIO`, short write, silent no-op fsync |
| `Db` | `begin`, `execute`, `query`, `commit` | commit failure, crash mid-transaction (via the VFS, §L3) |
| `Clock` | `now`, `sleep_until` | arbitrary wall-clock values, skew, non-monotonic jumps |
| `Rng` | `next_u64` | seeded, replayable |
| `Net` | `send`, `recv`, `close` | drop, delay, reorder, disconnect mid-stream |
| `Env` | `free_space`, `paths` | disk near-full, path missing |

The silent no-op fsync deserves a note: it is a **negative control**. A suite that still passes
against a filesystem that only pretends to fsync is a suite that is not actually testing
durability, so specific tests are asserted to *fail* under that mode (§L6).

### L2 — Executable model (the oracle)

A pure, I/O-free model of the observable system state:

* vault: multiset of `(vault_name → content_hash)`;
* index: `content` set keyed by hash, `device_file` map keyed by `(device_id, device_path)`;
* staging, per device: manifest entries with watermarks, plus write-log state;
* per-session: the frozen catalog and the diff classification.

The model consumes the same `Event` stream as the core and computes the expected state
independently. The simulator compares a canonical projection of both after every commit, every
recovery, and at session end. A mismatch is reported as a structural diff — "model expects 3
`device_file` rows for device A, implementation produced 2" — which is directly actionable in a
way a failed boolean is not.

The model exists to catch semantic drift that nobody thought to assert: dedup that forgets a
device row (§7.3 step 1), a nomination that offers a curated file (§8), a re-import that appends
instead of replacing (§7.5). It is deliberately kept small — a few hundred lines, no
optimization, written for legibility — because its own correctness is reviewed by reading it.

**The model is a protected artifact** (§L6): changing it requires a `SPEC.md` change in the
same commit. A model edited to agree with the implementation is not an oracle.

### L3 — Deterministic simulator

A single-threaded, seeded simulation of the entire desktop plus any number of phones. One seed
plus one step budget reproduces a run exactly.

**Simulated filesystem.** Durability semantics are modeled to match what §7 depends on:

| Operation | Modeled behavior |
|---|---|
| `write_at` | lands in a page cache; not durable |
| `sync_data` | makes that file's written ranges and its size durable |
| `rename` | immediately visible; **durable only after the parent directory is fsynced** |
| `create` | same — the directory entry needs a directory fsync |
| crash | all non-durable state is discarded; the tail of the last written page may be torn or zero-filled; unsynced writes may be dropped out of order |
| `ENOSPC` | injectable at any write or rename |

**SQLite lives inside the simulation.** A custom VFS (`xOpen`/`xRead`/`xWrite`/`xSync`/
`xTruncate`/`xDelete`/`xAccess`/`xLock`) is registered against the simulated filesystem, so the
staging manifest and the commit write-log — the two artifacts carrying the §7.3/§7.6 durability
claims — crash the way real storage crashes, and SQLite's own WAL recovery runs for real inside
the simulator. Without this, the most load-bearing claims in the spec would be the only ones the
simulator could not see.

**Simulated network and lifecycle.** Connection drops mid-stream, delayed and reordered
delivery, a phone that reconnects during a commit, two phones committing concurrently, a phone
that never returns (exercising the manual "Commit now" path), process crash and restart at any
effect boundary.

**Scenario generation.** `proptest` strategies generate operation sequences over the alphabet
{start session, send file, drop connection, crash desktop, restart, finish, commit, delete,
edit photo on phone, curate vault copy, fill disk}. Shrinking reduces a failure to a minimal
trace automatically.

**Campaigns.** `full` runs 256 seeds with a 5,000-step budget; `nightly` runs until a time
budget (default six hours) or 50,000 seeds. Any seed that has ever failed is committed to
`verification/corpus/` and replayed by `verify quick` forever after — the same discipline as a
fuzzing corpus, and the mechanism that makes fixed bugs stay fixed.

### L4 — Cross-language conformance

The protocol has two implementations in two languages, so it gets its own layer:

* **Golden vectors.** `verification/vectors/` holds encoded protobuf messages with their
  expected decoded interpretation. Both the Rust and Kotlin suites assert against the same
  files, so a field-numbering mistake or a default-value assumption fails on both ends at once.
  Vectors are a protected artifact.
* **Fake peers.** A fake phone (Rust) drives the desktop core inside the simulator; a fake
  desktop (Kotlin) drives the phone's session state machine in JVM tests. Both are generated
  from the same `.proto`, so neither can drift from the contract.
* **Descriptor check.** A `FileDescriptorSet` is committed; CI fails when the schema changes
  without it, making every wire-format change explicit in review.
* **Real-bytes end-to-end.** A small set of scenarios runs the real binary over loopback TLS
  with real files and real SQLite, proving the adapters the simulator replaces. Deliberately
  few: this layer verifies the shell, not the logic.

### L5 — Acceptance scenarios

Declarative TOML, one file per scenario, in `verification/scenarios/`. They are the readable
face of the harness: a human can review one against the spec without reading Rust, and an agent
can author one from a spec clause.

```toml
id          = "S-DELETE-004"
covers      = ["R-DELETE-002", "R-DELETE-011"]
description = "A vault copy the user deleted is never nominated, even though the index still covers it"

[[phone.library]]
path = "DCIM/Camera/IMG_0001.jpg"
size = 2400000
mtime = 1756000000
content = "alpha"
exif_datetime = "2026-08-01 12:34:56"

[[desktop.index.device_file]]
device_path = "DCIM/Camera/IMG_0001.jpg"
size = 2400000
mtime = 1756000000
content = "alpha"
vault_name = "2026-08-01_123456.jpg"

[desktop.vault]
files = []                      # the user curated this photo away

[[events]]
kind = "run_session"

[expect]
deletion_candidates = []
uploaded            = []
vault               = []
phone_kept          = ["DCIM/Camera/IMG_0001.jpg"]
```

The same file runs in the simulator by default (milliseconds) and against the real stack with
`--real` (seconds). Deserialization is strict: an unknown key or a misspelled event kind is an
error naming the offending line, never a silently ignored field.

### L6 — Runner, reports, and gates

**Entry point.** `cargo xtask verify <tier>`, wrapped by a `./verify` shim so the command an
agent runs is stable. Pure Rust orchestration; it shells out to Gradle for the Kotlin tiers and
needs nothing installed beyond the toolchains in `STACK.md`.

| Tier | Contents | Budget |
|---|---|---|
| `quick` | core unit tests, model differential on the committed corpus, all scenarios in sim, `spec-check`, clippy, fmt | < 30 s |
| `full` | + 256-seed campaign, real-bytes E2E, Kotlin JVM tests, conformance vectors, descriptor check, requirement matrix gate | < 5 min |
| `nightly` | + long campaign, `cargo-mutants`, Android managed-device suite, `cargo-deny`, `cargo-audit`, performance metrics | hours |

**Reports.** Every run writes `verification/reports/latest.json` (plus a timestamped copy):

```json
{
  "tier": "full", "started": "...", "duration_s": 143, "result": "failed",
  "requirements": { "active": 118, "verified": 117, "unverified": ["R-RECOVER-009"] },
  "failures": [{
    "test": "sim::commit::donemark_ordering",
    "requirement": "R-COMMIT-014",
    "spec": "§7.3 step 2",
    "seed": "0x8f31c0a94b2e",
    "repro": "./verify replay 0x8f31c0a94b2e",
    "trace": "verification/reports/traces/0x8f31c0a94b2e.jsonl",
    "diff": "model expects vault={2026-08-01_123456.jpg}, implementation produced {}"
  }],
  "campaign": { "seeds": 256, "steps_total": 1180432, "failed_seeds": ["0x8f31c0a94b2e"] },
  "mutation": { "module": "core::commit", "caught": 91, "missed": 4, "score": 0.958 }
}
```

A human-readable summary prints alongside. The JSON is what an agent reads to decide its next
action, so it always names a requirement, a spec section, and a single reproduction command. The
shape grows as the tiers land: today it carries the result of each check, and the campaign,
mutation, and failure sections appear with the milestones that produce them.

**The done gate.** A change is complete when all of the following hold:

1. every requirement ID touched by the change has at least one passing covering test, and no
   `active` requirement is unverified;
2. `verify quick` and `verify full` are green, corpus replay included;
3. `cargo-mutants` on the changed core modules scores at or above **0.85**, with every surviving
   mutant either killed or listed in `verification/mutants-allow.toml` with a one-line
   justification;
4. no protected artifact was modified without an accompanying `SPEC.md` change.

**Protected artifacts.** `verification/requirements.toml`, `verification/vectors/`, the model
crate, and the `[expect]` blocks of existing scenarios define correctness. A pre-push hook and
the nightly audit flag any commit that touches them without touching `SPEC.md`. This is the
structural answer to the failure mode where an agent, unable to make the implementation right,
makes the oracle wrong instead.

**Negative controls.** `verify self-test` builds the core with a `sabotage` feature that injects
known-bad implementations one at a time — done-mark written before the directory fsync, watermark
taken from file size instead of the durable counter, dedup that skips the device row, nomination
that trusts the index without stat-ing the vault — and asserts that each is caught, by which
test, within a bounded number of seeds. A harness that cannot demonstrate it detects these has
no standing to certify anything. The sabotage set grows with every real bug found.

## 4. Android verification

* **JVM (fast, hermetic):** session state machine, catalog freezing, diff handling, the
  size+mtime versus re-hash deletion gates, and the reconnect path, all driven against the
  Kotlin fake desktop plus the shared conformance vectors.
* **Managed devices (nightly):** Gradle Managed Devices with a headless ATD image cover what
  only Android can answer — MediaStore enumeration including the pending/trashed exclusions,
  `RELATIVE_PATH` semantics, `DATE_MODIFIED` precision, the `createDeleteRequest` flow with and
  without `MANAGE_MEDIA`, and foreground-service lifecycle. Reproducible image versions, no
  physical phone in the loop.
* **Manual matrix (pre-release):** the hostile-OEM behaviors of §3.3 — autostart permission,
  battery-saver exemption, lock-in-recents, screen-off transfers, and real Wi-Fi roaming — are
  checked by hand on each target brand and recorded in `verification/manual/<brand>.md`. This is
  stated as a limit of the instrument, not hidden inside it.

## 5. Performance guardrails

Two kinds, kept apart because they fail differently:

* **Deterministic assertions (gated).** The HTTP/2 flow-control window, channel-pool size, and
  chunk size are asserted against the values `STACK.md` §3.6 fixes, including a check on the
  actual `SETTINGS` frame observed by the real-bytes test. A regression here is a silent
  throughput collapse, and this catches it without depending on machine speed.
* **Tracked metrics (not gated).** Loopback throughput for a synthetic 2 GB transfer with
  hashing in the path, catalog diff time for a 50,000-entry library, and commit time for a
  10,000-entry batch (which the spec claims is pure metadata work). Recorded per nightly run and
  compared against a rolling baseline; a regression opens a report entry rather than failing a
  build, because timing on a shared machine is noisy.

## 6. Where it runs

Every push and pull request runs the quick and full tiers on a hosted runner, which installs the
toolchain, calls `./verify full`, and keeps the JSON report as an artifact. The same command runs
locally, so a check behaves the same way in both places. The pre-push hook runs the quick tier
and the protected-artifact comparison before a push leaves the machine.

The nightly tier runs on a schedule, sharded across hosted jobs so each stays inside the limit
that applies to one job. Seed campaigns and mutation testing both divide cleanly, so parallel
jobs finish sooner than one long run. The development machine runs the same tier directly when
someone wants a deep campaign, and it is where the emulator suite runs, since its processor is
passed through and nested virtualisation gives it a working `/dev/kvm`.

One kind of test stays outside it. A session with a real phone needs multicast on the local
network, which the machine's NAT interface does not carry, so acceptance runs on the desktop
that is the deployment target anyway. `docs/DEVELOPMENT.md` separates that from the two cases
which look similar and are not: the discovery code is tested against the Rust test client, and
emulator sessions do not use discovery at all.

## 7. Limits of the instrument

1. **SQLite's own correctness is trusted.** The VFS simulates its storage faithfully; it does
   not verify SQLite's implementation of WAL.
2. **The emulator is not a phone.** OEM-modified frameworks, aggressive task killers, and real
   radio behavior are outside it (§4, manual matrix).
3. **Rendering is unverified.** Slint layout and visual correctness are reviewed by hand; only
   the view-model state feeding the UI is tested.
4. **The model is a second implementation.** It can be wrong in the same way twice if a
   requirement is misread. The registry quotes and human review of scenario `[expect]` blocks
   are the mitigations; nothing removes this risk entirely.

---

## Appendix A — Decision log

| # | Decision |
|---|---|
| V1 | Sans-I/O core, all I/O in the shell; enforced by the dependency graph and clippy `disallowed-types` |
| V2 | Full deterministic simulation: clock, network, filesystem, process crashes, seeded and replayable |
| V3 | SQLite runs inside the simulation through a custom VFS, so the manifest and write-log crash realistically |
| V4 | An executable model is the primary oracle; differential comparison after every commit and recovery |
| V5 | Requirement registry with verbatim quote anchoring; `SPEC.md` stays free of identifiers |
| V6 | Coverage is declared at the test site (`covers!`, `@Covers`, scenario key) and extracted statically |
| V7 | An `active` requirement with no passing test fails the build; deferral is an explicit, visible edit |
| V8 | Acceptance scenarios are declarative TOML, runnable in simulation or against the real stack |
| V9 | Every failing seed is committed to a corpus and replayed by the fastest tier forever |
| V10 | Done gate = requirement matrix + green campaign + mutation score ≥ 0.85 on changed core modules |
| V11 | Protected artifacts (registry, vectors, model, existing expectations) cannot change without a `SPEC.md` change |
| V12 | Negative controls: sabotage builds prove the harness detects known-bad implementations |
| V13 | Android split: JVM tests with a fake desktop, managed-device suite nightly, OEM behavior manual |
| V14 | Performance split: configuration assertions are gated, wall-clock metrics are tracked only |
| V15 | Hosted runners gate every push and run the sharded nightly tier; no self-hosted runner |
| V16 | Work runs in an x86_64 Fedora KDE virtual machine, including the emulator; sessions with a real phone run on the desktop |
