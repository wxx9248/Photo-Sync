# Handoff

What an agent could not check from inside this repository, and what is worth a person's
attention before the next milestone. This is a working note rather than an authority: nothing
here overrides `SPEC.md`, and anything that becomes a rule belongs in the documents
`AGENTS.md` lists.

Written at the close of milestone M1 and added to as each one went. Every milestone is
closed. **Section 0 is the list to work through**; the rest is the reasoning behind each item,
kept in the order it was written.

## 0. Everything waiting on you

One list, gathered from every milestone. The sections after it explain each item where it came
up. Nothing here is blocked on me; all of it needs a machine, a phone, or a person's eyes.

### Look at it

- [ ] **The desktop window has never been rendered.** No display on the development machine.
      `cd desktop && cargo run --bin photo-sync`. Worth your eyes: whether the stock
      `camera-photo` tray icon reads as this application on Plasma; whether the progress line
      is legible during a real transfer; and the sentence under the vault box, which is the one
      place the interface has to talk somebody out of an assumption. (§12)
- [ ] **The phone's screens have never been rendered either**, for the same reason plus the
      emulator below. (§13)
- [ ] **Both Simplified Chinese translations were written without a native reader** — the
      desktop window and the phone's strings. `LANG=zh_CN.UTF-8 cargo run --bin photo-sync`.
      (§12, §13)

### Run it somewhere this machine cannot

- [ ] **The Android instrumented tests.** They compile and have never executed. With the SDK,
      the emulator and a system image all installed here, the guest now boots properly --- init
      reaches `surfaceflinger`, `adbd` and `bootanim`, and `adb` sees the device --- and then the
      emulator process on the host exits with status 1 and no message, a minute or two in.
      Renderer, memory and core count make no difference, and Gradle's managed-device path
      fails the same way. This development machine is itself a virtual machine, so the
      emulator runs nested, and the kernel log shows its vcpu being created and torn down on
      each attempt. That is the likeliest reason and it is not the project's. On a machine
      where an emulator stays up: `cd android && ./gradlew :app:phoneDebugAndroidTest`. Four
      tests, all about what MediaStore does rather than what this code does with the answer.
      When they pass, `R-CATALOG-001` and `R-CATALOG-002` can move to active. (§13, §16)
- [ ] **Discovery against a real phone.** The responder is checked against another Rust daemon
      on this machine; multicast across a home router and `NsdManager`'s reading of the records
      are what only a real network answers. The phone's side of it was rewritten in §15 and has
      never run. Worth doing on an Android 12 or 13 phone in particular: that is where §16's
      lint error would have thrown. (§11, §15, §16)
- [ ] **A video, across the wire.** The phone used to gather a whole file in memory before
      sending it, so anything larger than free RAM could not cross. It streams now, and a test
      holds it to that. What the test cannot show is the number: send a photograph and then a
      long video, and watch the phone's memory while it goes. (§15, §16)
- [ ] **Closing the window leaves the desktop running.** It used to stop everything --- tray,
      advertisement and listener --- so a phone could not find the desktop until somebody opened
      it again. Fixed, and confirmed here by closing the window from a KWin script and watching
      the process stay up. `R-UI-004` is deferred because nothing in the suite can close a
      window. Close it with the mouse, then check the tray icon is still there and a phone
      still syncs. (§17)
- [ ] **Restart the desktop after pairing.** It read its paired phones from a path nothing
      wrote to, so it woke up knowing nobody. Fixed, and unreachable by any test, because the
      defect was in `main`. Pair a phone, quit the desktop, start it again, and check the phone
      still syncs without showing a code. (§15)
- [ ] **One real session, end to end.** Desktop and phone, same network, a photograph across
      and freed. This is M9's exit and the only thing that exercises every piece at once. (§14)
- [ ] **Throughput.** The real server takes its lock per message rather than per file, which is
      what makes streaming and concurrent transfers work. Nothing has measured the cost on a
      real network, and it is the one change whose price is not visible from inside the
      repository. (§8, §9)

### Decide

- [ ] **§7.4's parenthetical.** It justifies recovery by an inference the desktop does not
      make, and is safer for not making it. Reword the specification, or leave it. (§9)
- [ ] **A vault with more than one writer.** Today a file that appears at a reserved name is
      renamed over and the photograph wins. If a vault might ever sit on a synced folder or a
      network share, that trade is worth confirming. (§9)
- [ ] **The SQLite virtual file system.** Outstanding since M2 with three ways forward written
      out. Nothing since has needed it, and a crash inside SQLite's own writing is still not
      modelled. (§5)
- [ ] **Pairing has no deadline.** A window a person opens stays open until a phone pairs or
      somebody closes it. (§4)
- [ ] **The F-Droid signing keystore.** Make it, and back it up off the machine, before the
      first release. Losing it re-identifies every phone in the house. `packaging/fdroid/`
      says why at length. (§14)

### Confirm once on a real machine

- [ ] **The private key's permissions.** Written owner-only and asserted by a test, but a umask
      is the sort of thing that differs. (§4)
- [ ] **The OEM keep-alive steps.** `docs/INSTALL.md` has a table per brand, written from the
      specification rather than from a phone in hand. Each row is a guess until somebody
      follows it on that brand. (§14)

### Closed since these were written

- **The JDK.** You chose 25; `DEVELOPMENT.md` says so, both halves target it, and the setup
  command it documents now works. Kotlin is 2.4.20 and Gradle 9.7.1, both current, with the
  wrapper checked in.
- **The pairing code across languages.** `verification/vectors/pairing.tsv` states the six
  digits for a set of key pairs, computed from the written derivation rather than from either
  implementation, and both ends answer it. That was the vector M8 owed.

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

## 5. M2 is closed, with one work item outstanding

Its exit criteria are met. `./verify self-test` notices five wrong worlds and names the test
that caught each, a 256-seed campaign runs clean in the full tier, and R-RECOVER-001 through
R-RECOVER-004 are active with passing tests. Startup recovery and write-log replay landed with
them, which `docs/ROADMAP.md` had listed under M4. The fault injection of `VERIFICATION.md`
§L1 is complete but for one entry, noted at the end of this section.

### The SQLite virtual file system, and why it is not built

`STACK.md` §6 chose `sqlite-vfs` so the manifest and the write-log would crash the way real
storage crashes. I tried it rather than assuming, and it does not reach as far as the product
needs.

* The crate registers against `rusqlite` 0.39 and a database opens through it. That much works.
* Its WAL index, the shared memory SQLite needs for write-ahead logging, lives in a module the
  crate names `wip`, and the only handle that compiles without implementing it is one that
  declares WAL unavailable.
* `STACK.md` §3.5 runs **both** databases in WAL mode. A simulator whose SQLite ran in a
  different journal mode from production would be testing storage the product never uses,
  which is most of the reason for doing this at all.

Three ways forward, and the choice is yours.

1. **Leave it.** A crash currently treats both databases as intact. That is sound for WAL plus
   `synchronous=FULL`: a transaction that returned survived, and one that did not never
   existed. `VERIFICATION.md` §7 already says SQLite's own correctness is trusted and that the
   VFS does not verify WAL, so what this would add is narrower than it first appears.
2. **Implement the WAL index** on top of that `wip` module. The largest of the three, and the
   part the crate itself calls unfinished.
3. **Run the simulator's SQLite without WAL**, accepting a journal mode that differs from
   production, which trades one kind of fidelity for another.

One thing applies whichever you pick: the simulator would need the real `rusqlite` store, which
lives in the shell today. Sharing it means lifting it into a crate of its own rather than
having the simulator depend on the product.

### A short write is not modelled

Deliberately. The shell writes with `write_all`, which loops until the bytes are gone or an
error is returned, so a short count cannot reach the core to be mishandled. Modelling it would
be modelling something that cannot happen.

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
* A commit that halts part-way leaves the device locked until recovery replays its write-log,
  and recovery is M4. Nothing is lost, but that phone cannot sync again until then.

## 8. What M3 changed, and what it leaves for a person

M3 closed. `R-CATALOG-003`, every `R-DIFF-*`, every `R-XFER-*` and every `R-STAGE-*` are active
and verified, and scenario `S-STAGE-001` covers an interrupted transfer carrying on from the
desktop's watermark.

Two of the changes were product defects rather than missing features, and both are worth
knowing about.

**A transfer never really resumed after a power loss.** The desktop kept each partial's digest
in memory only, so a machine that came back had the bytes and no way to verify them: §7.6's
resume offset was worth nothing and every interrupted file started again. The desktop now reads
each partial back off its own disk, a megabyte at a time, before it answers a diff. **This costs
one pass over every unverified partial at startup** — bounded by what was in flight when the
power went, not by the size of the library. A partial that reads back short, or not at all,
starts from zero rather than resuming onto bytes nothing verified.

**The real server gathered a whole upload in memory** and handed it to the desktop only when
the stream ended. Watermarks, partial files and resume all worked in the simulator and none of
them had ever happened over a socket; a video larger than free memory would have taken the
desktop down. Each message now goes across as it arrives. Nothing measures the cost of taking
the desk lock per message rather than per file, and **a throughput measurement over a real
network is worth doing before this ships** — it is the one change here whose price is not
visible from inside the repository.

### Left for M8, not skipped

`R-CATALOG-001`, `-002`, `-004` and `-005` stay deferred. They are `SPEC.md` §3.2 — the
`DCIM/Camera` bucket, `IS_PENDING` exclusion, the once-only frozen enumeration, and no upfront
hashing — and they describe what the phone does. Nothing on the desktop can observe any of them,
so activating them here would mean claiming a check that does not exist. They close with the
Kotlin session core in M8. `R-CATALOG-003`, the `(path, size, mtime)` identity the desktop
diffs against, is active and verified.

### Still open from earlier milestones

* The SQLite virtual file system of §5 above. Nothing changed there.
* Capture-time extraction still needs `nom-exif` and a timezone database, so a vault name still
  falls back to the modification time in UTC.
* `S-STAGE-001` is checked in simulation only, because it takes the power away mid-transfer.
  The real stack proves the half of it that a socket can: `a_transfer_cut_short_carries_on_from
  _the_watermark_over_a_socket` sends part of a file, drops the stream, and finds those bytes on
  the desktop's disk and offered back on the next diff.

## 9. What M4 changed, and one question for you

M4 closed. Every `R-COMMIT-*`, `R-RECOVER-*` and `R-INDEX-*` is active and verified, a commit
can now be crashed anywhere in §7.3 rather than at one chosen point, and the sabotage set has
grown from five wrong desktops to nine.

**Crashes are placed by counting now.** The campaign used to stop a commit just before the
first rename, which is one power-off point out of the twenty-odd that §7.4 promises to survive.
A seed now says how many of the commit's steps happened before the power went, which reaches
places no named effect can: between two store writes of the same kind, part-way through a group
of renames, and between the two halves of a clear. It found a disagreement on its first run and
the desktop was right; the model was insisting a commit had happened when §7.4 allows it not to
have. Nothing in the product changed.

**Four more rules can now be seen breaking.** Renaming before the write-log is sealed, clearing
staging before the index rows are durable, clearing the manifest before the write-log, and
carrying on past a rename that failed. Each is a real desktop built behind a feature flag, and
each is caught. Two of them needed tests that did not exist, so a commit whose vault refuses
the rename is now known to stop where it stands and be finished by recovery afterwards.

### The question

`SPEC.md` §7.4 ends with an assumption: recovery assumes the vault has a single writer, because
"already present at the target ⇒ our own completed rename" is sound only if nothing else creates
files there. **The desktop never makes that inference.** It decides from the staged file rather
than from the target, so a file that appeared at a reserved name while the machine was down is
renamed over, and the photograph wins. The test
`a_photograph_is_not_given_up_for_whatever_sits_at_its_name` pins that.

This is safer than the specification's own reasoning in the direction that matters, and it costs
the intruding file instead. Two things follow, and both are yours to decide:

1. Whether §7.4's parenthetical should be reworded, since it justifies the design by an
   inference the code does not make. Nothing behaves wrongly either way; the prose is a
   rationale, not a rule, which is why it has not been touched.
2. Whether overwriting a stranger's file is what you want if a vault ever ends up somewhere with
   more than one writer — a synced folder, a network share. Today it is, and the alternative
   (stop and ask) is a feature nobody has specified.

### Still open

* The SQLite virtual file system of §5. Nothing changed there.
* Capture-time extraction still needs `nom-exif` and a timezone database.
* Noticing *which* files are stranded is the other half of §7.5's escape hatch and belongs to
  the interface in M7. The action it will call — dropping the row so the next diff re-sends the
  file — is built and tested.
* The throughput question from §8 above still stands, and now matters slightly more: crash
  recovery reads every unverified partial back at startup.

## 10. What M5 changed

M5 closed. Eight of the ten `R-DELETE-*` requirements are active and verified, and scenario
`S-DELETE-001` — a curated vault copy is never nominated — runs in simulation and against the
real stack, which is the exit M5 names.

Most of §8 was already built and already checked. What was missing was the moment the protocol
exists for: a photograph changing between the desktop offering it and the phone letting go of
it. The simulated phone can now be told to write over a file at exactly that point, so both
gates are exercised rather than assumed:

* a photograph committed this session, whose size or time changed, is kept and reported;
* one from an earlier session whose **bytes** changed while its size and time stayed the same
  is kept, which only the re-hash can catch. That is the case the cheap gate would wave
  through, and it is now the test that says the expensive one is worth its cost.

The campaign can also write over a photograph before the prompt, so those gates meet crashes
and restarts rather than only the two arrangements above.

`into_chunks`, which cuts the diff and the candidate list into messages, now has unit tests.
It carries one bit of real meaning — which message is the last — and the reader on the other
end waits for it, so an off-by-one there is a session that never finishes. Reaching that path
end to end would need a thousand-photograph library; the function is pure, so it is tested
directly instead.

### Left for M8

`R-DELETE-005` (the phone shows one prompt) and `R-DELETE-009` (`MediaStore.createDeleteRequest`,
silent under `MANAGE_MEDIA`) are the phone's own doing. Nothing on the desktop can observe
either, so they stay deferred rather than being claimed here.

## 11. What M6 changed

M6 closed. `R-DISCOVER-001`, `R-PAIR-003`, `R-PAIR-004` and four of the five `R-SESSION-*` are
active and verified; sixty requirements are active now, with none unverified.

**The desktop can be found.** An mDNS responder advertises `_photosync._tcp` with the protocol
version and the display name in its TXT record, which is what lets a phone tell an incompatible
desktop from an absent one before opening a connection. Given no addresses it tracks the
machine's own, since a laptop that moves between wifi and a cable is the ordinary case. The test
browses from a second, independent daemon standing in for `NsdManager`, so what passes is a real
multicast round trip rather than a responder hearing itself.

**A campaign runs two phones.** One phone left two of §7.3's claims to hand-written tests:
that commits serialize across devices, and that one phone's content dedups against another's
through the index. That also made the rule M4 could not sabotage reachable — index rows going in
after the commit lock is released, so the next commit builds against an index that has never
heard of the batch and stores a second copy. Every ordering rule in §7.3 now has a control, ten
in all.

**Two things are checked by lying.** A phone that claims another device's identity in its
handshake is still known by the key it holds, so nothing in a message body decides whose session
this is. And a phone whose remembered desktop key does not match the one answering never opens
the connection at all — the refusal happens in TLS, before a byte of the protocol.

### Worth a person's attention

* **Discovery has never met a real phone.** The responder is checked against another Rust daemon
  on this machine. Multicast across a home router, and `NsdManager`'s own view of these records,
  are the two things only a real network can answer. `docs/DEVELOPMENT.md` already says the
  machine needs bridging for that.
* Nothing calls the responder yet. It is a module with tests and no caller until M7 assembles
  the application.

## 12. What M7 changed, and the part only you can finish

M7 closed. All three `R-UI-*` are active and verified, and there is an application rather than a
placeholder: settings, a window, a tray item, sleep inhibition, autostart, logging, and both
languages.

The three requirements are the ones with state behind them, and each is checked where it lives:

* **The vault path is a plain setting.** Changing it leaves the photographs where they were and
  does not create the new directory behind anyone's back. `R-UI-003`.
* **Suspend inhibition.** Which phones are busy is bookkeeping — the first to arrive takes the
  inhibitor, the last to leave releases it. Whether logind will actually hold off sleeping is a
  question only a real bus can answer, and this machine has one, so the test asks it for real
  and refuses to skip quietly anywhere a bus exists. `R-UI-002`.
* **"Commit now"** needed no new code; the event was already in the vocabulary. What it needed
  was the case it exists for: staging left by a phone that never came back. `R-UI-001`.

The window's contents are decided in plain Rust that runs without a screen — progress, the
wording of each line, how a byte count reads — so the markup decides only how things look. That
split is deliberate: the less that lives only in the markup, the less rides on somebody's eyes.

### Please look at the window

**Nobody has seen it.** There is no display on the development machine. It compiles, and the
application starts and stays running — the smoke run reaches `listening port=…`, writes its
rolling log, creates the identity and the index, announces itself, and takes a tray slot — but
what the window *looks like* has never been rendered once.

```sh
cd desktop && cargo run --bin photo-sync
```

Worth your eyes in particular:

1. Whether the tray icon (`camera-photo`, a stock name so it follows your theme) reads as this
   application on Plasma.
2. Whether the progress line is legible while a phone is actually transferring — the wording is
   `3 of 4 photographs, 2.0 MB`, and only a real transfer shows whether that updates at a
   sensible pace.
3. The sentence under the vault box: "Changing this moves nothing. Move the photographs yourself
   if you want them moved." That is the one place the interface has to talk somebody out of an
   assumption, and it is worth reading aloud.
4. The Simplified Chinese. Every string is translated and a test insists the catalogue and the
   markup describe the same window, but the translations were written without a native reader.
   `LANG=zh_CN.UTF-8 cargo run --bin photo-sync`.

### Not built, and deliberately so

* **Pairing from the window.** The pairing code has a place in the markup and nothing drives it:
  the pairing window is opened programmatically today, and a person-facing "pair a new phone"
  flow needs a decision about how it is started that nobody has made.
* **Staged sessions are listed empty.** The "Commit now" button and the event behind it work and
  are tested; what does not exist is the query that lists which devices have staging worth
  committing. It is a store read away.
* `.po` files ship as source. Compiling them to `.mo` at install time belongs with packaging in
  M9.

## 13. What M8 changed

M8 closed. The phone has a session state machine that plays `SPEC.md` §6 from the handshake to
the summary, holds no Android type, and is checked by fifteen tests on a plain JVM against a
fake desktop — a resumed transfer that hashes the photograph it did not send, a catalog frozen
while the camera keeps working, a phone told to wait for a commit that is still running, and
the deletion gates.

Around it: MediaStore enumeration narrowed to what §3.2 allows, the deletion request of §8, the
foreground service that is the floor of §3.3's keep-alive stack, the two taps of §3.4 in
Compose, both languages, and a gRPC client that pins the desktop's key and presents one the
Android Keystore will not hand out.

`./verify` runs the phone's tests too, and the traceability matrix reads Kotlin `@Covers`. Half
the specification is the phone's; leaving it out would have shown those rules unverified for as
long as the Android half existed.

### Three things that had to be worked out

**The protobuf Gradle plugin does not understand AGP 9.** It reaches for an extension type that
version no longer has. Both are current and neither is wrong; they simply do not compose yet.
The wire stubs are generated in a plain JVM module and depended on, which costs nothing.

**AGP 9 compiles Kotlin itself**, so the separate Kotlin plugin the M0 skeleton applied is now
an error rather than a redundancy.

**The emulator does not survive here.** Gradle's managed device fails creating its snapshot;
launched by hand the emulator boots and then exits when its launching command does. The
instrumented tests are written and compile, and they are the item in §0 that needs a machine.

### What is active, and what is not

`R-CATALOG-004`, `R-CATALOG-005` and `R-PAIR-002` are active on the strength of tests that
actually run. `R-CATALOG-001` and `R-CATALOG-002` have tests that have never executed, so they
stay deferred: activating them would claim a check that has not happened. `R-SESSION-001`,
`R-SESSION-003`, `R-DELETE-005` and `R-DELETE-009` need a screen or a device and stay deferred
with them.

## 14. What M9 changed, and how to release

M9 closed. There is a `packaging/` directory with everything an installation needs.

**The computer.** A PKGBUILD that builds the binary, installs a desktop entry and an icon, and
compiles the translation catalogues. No service is enabled and nothing runs as root: the
application needs the graphical session for its tray and its sleep inhibitor, so it is started
by the session. The release binary builds and the desktop entry validates clean.

**The phones.** An F-Droid repository configuration, metadata, and the procedure in
`packaging/fdroid/README.md`. The release APK builds unsigned here, which is as far as this can
go without the keystore — and the keystore is the one thing in this project that must be kept
forever, because `ANDROID_ID` derives from it and losing it re-identifies every phone in the
house.

**The instructions.** `docs/INSTALL.md` covers installing both halves, pairing, the first
session, the per-brand keep-alive steps, and what to do when something is wrong. The brand
table is written from the specification rather than from a phone in hand, so each row is a
guess until somebody follows it.

What M9 cannot close from here is its own exit: both halves installing on a clean machine and
phone, and one real session running end to end. That is the last item in §0.

## 15. What an inspection against the conventions turned up

A sweep of both halves against `CONVENTIONS.md` and the two language documents, after every
milestone had closed. Five defects and four tidying changes; each is its own commit.

**The phone held whole files in memory.** `GrpcDesktop` gathered every chunk into a list and
opened the upload only when the digest arrived, so a phone needed as much free memory as its
largest photograph and a video simply could not cross. `STACK.md` §5.3 says chunking bounds
memory on both ends, and it did not on this end. This is the same defect the desktop had in
M6, on the other side of the same wire. The request now opens when the upload does and the
pieces travel along it, over a channel with no buffer, so the phone holds one 512 KB piece at
a time.

**The phone never gave up looking for a desktop.** `Discovery.find` declared a ten-second
timeout and never read it, so a phone whose computer was off waited forever and the "is the
computer on?" screen of §5.1 was unreachable. Discovery was also left running after an answer
came back, which keeps the radio busy for the life of the process.

**The desktop forgot every phone when it restarted.** `main` asked `Paired::load` for a
directory and gave it `<data>/paired.json`, a name nothing else uses, so the loader looked
inside a file that is not a directory and found nothing. It then served through a `listen`
whose hidden default wrote new pairings into the working directory. Pairing is once-ever in
§5.2 and had become once-per-start. The default is gone: there is one `listen` and it asks
where the pairings live.

**Nothing bounded what a phone could make the desktop hold.** The catalog and the deletion
results were each gathered into a vector with no ceiling, which conventions §12.1 and §12.2
rule out for anything a peer drives. `SPEC.md` §6.1 now states the ceiling and R-CATALOG-006
covers it.

**Cancellation was reported as a refusal.** Every call in `GrpcDesktop` was wrapped in
`runCatching`, which also catches the `CancellationException` a cancelled session throws. The
state machine was told the desktop had refused, and the coroutine carried on past its scope.
Rule K8.

The rest was tidying: one reader for the conformance vectors instead of one per Rust suite,
one name for the keystore alias, `internal` across the application module, and
`unreachable_pub` on the verification runner so R10 is checked by the compiler rather than at
review.

### Left alone, and why

- **`MediaStoreLibrary.read` reopens the file for every chunk**, re-querying MediaStore and
  skipping to the offset each time, which is quadratic in the length of a video. Rule 11.1
  says measure first, and measuring this needs a phone. It belongs with the throughput item
  in §0.
- **`Desktop::showing` in the pairing test polls with `Thread::sleep`**, which rule 6.4 rules
  out. It has a two-second budget and would go flaky on a slow machine. Fixing it means giving
  `PairingWindow` something to wait on, which is more than the sweep should carry.
- **No ktlint or detekt is configured**, though `CONVENTIONS-KOTLIN.md` says the lint
  configuration lives there. Adding one needs the plugin, which needs a network this machine
  does not have.
- **The application module has no plain JVM tests.** `GrpcDesktop` and `Discovery` hold no
  Android type and could be tested against an in-process server, but `./verify` runs only
  `:session:test`, and reaching the application module needs the Android SDK on the build
  server. That is why two of the fixes above are in §0 rather than in the suite.

## 16. What the application module's checks turned up

The harness reached the session module and stopped. Nothing built the Android application, ran
a test against it, or pointed Android Lint at it, so a change that broke the application passed
every check. Three checks now run in the full tier, one Gradle task each: the module builds,
its unit tests pass, and lint is content. They need the Android SDK, which nothing else here
does, so a machine without one skips them and says so.

**Lint found a defect on the first run.** `NsdServiceInfo.hostAddresses` arrived in Android 14,
and on Android 13 only with the seventh Tiramisu extension. `SPEC.md` §3.1 starts this
application at Android 12, so on the floor of its own supported range discovery reached for a
method that is not there. It compiles, and it would have thrown the moment a desktop answered.

**The application module now has unit tests.** `GrpcDesktop` holds no Android type, so all of
it runs against a gRPC server in this process; `FilePairingStorage` is ordinary file I/O. Nine
tests, and both fixes from §15 are among them.

Each was checked by putting the old behaviour back rather than by trusting that it passes.
Buffering the file again fails two of them, and restoring `runCatching` fails a third. A test
that passes either way is not evidence, and these two mattered: they are the fixes nothing
could confirm when they were written.

### Left alone, and why

- **Eight lint warnings.** Two ask for plurals rather than `%d` in the strings, which matters
  in both languages and is a translation change rather than a code one. Two are about Selected
  Photos Access on Android 14, which is a product decision about partial media grants and not
  something to settle in an inspection. The rest are backup and data-extraction rules. Lint
  fails on errors only; making warnings fail would mean settling all eight first.
- **The emulator.** See §0. The guest boots; the emulator process does not stay up, most
  likely because this machine is a virtual machine and the emulator is a second one inside it.
