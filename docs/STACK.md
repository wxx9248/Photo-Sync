# Photo Sync — Technology Stack

Companion to `SPEC.md`. The spec defines *what* the two apps do; this document fixes *what
they are built from*, and records why each component was chosen over the alternatives that
were considered.

## 1. Selection criteria

Ranked, and derived from the design priorities in `SPEC.md` §1:

1. **Syscall-level control over durability.** §7.3's fsync-before-done-mark ordering, §7.6's
   watermark truncation, and commit-by-`rename()` are only as correct as the API that
   expresses them. The stack must offer `fdatasync`, directory `fsync`, `ftruncate`, and
   `rename` directly — not through a runtime that buffers or reorders them.
2. **Testability of the failure paths.** "Idempotent at any power-off point" is a claim that
   has to be executed, not asserted, so file I/O must be interposable behind a seam.
3. **A typed contract across a two-language boundary.** The desktop and the phone are written
   in different languages; the protocol is the one place a silent divergence causes data loss.
4. **Self-contained deployment, small runtime surface.** The desktop app is one binary that
   starts at login. It must not require system daemons, an interpreter, or a bundled VM.
5. **Throughput.** Concurrent streams must saturate the Wi-Fi link with hashing in the path.
6. **Native-feeling Plasma integration.** A real StatusNotifierItem tray, Wayland rendering,
   and system suspend inhibition.

## 2. At a glance

| Concern | Desktop | Mobile |
|---|---|---|
| Language | Rust 2024 edition | Kotlin |
| UI | Slint (winit + FemtoVG) | Jetpack Compose + Material 3 |
| Async | Tokio (multi-thread) | Coroutines + Flow |
| Transport | tonic gRPC server (HTTP/2) | grpc-okhttp + grpc-kotlin stubs |
| Serialization | prost | protobuf-javalite |
| TLS | rustls, custom SPKI verifier | Conscrypt via custom `X509TrustManager` |
| Identity key | rcgen self-signed P-256, file 0600 | P-256 in Android Keystore (StrongBox if present) |
| Discovery | `mdns-sd` responder | `NsdManager` |
| Persistence | rusqlite (bundled SQLite) + refinery | none beyond the pairing record (DataStore) |
| Hashing | `sha2` (SHA-NI accelerated) | `MessageDigest` (ARMv8 crypto extensions) |
| Capture time | `nom-exif` | — |
| Tray / D-Bus | `ksni`, `zbus` | — |
| Logs | `tracing` | Logcat + in-app diagnostics |
| Packaging | PKGBUILD → single binary | self-hosted F-Droid repo |
| License | GPL-3.0 | GPL-3.0 |

Rust crate versions below were the current stable releases when this document was written. Each
is pinned exactly in `Cargo.toml` as the milestone that needs it adds it, so a crate named here
is a decision rather than a dependency the workspace already carries. Android artifact versions live in the Gradle version
catalog (`gradle/libs.versions.toml`) and are pinned at project initialization.

## 3. Desktop

### 3.1 Language and runtime — Rust, one static binary

Rust satisfies criteria 1, 2, 4 and 5 simultaneously, which nothing else on the shortlist did.
The durability rules of §7 map one-to-one onto `std::fs` and `rustix` calls with no runtime
between the code and the kernel; memory safety covers the concurrent-session paths that a
manual-lifetime language would leave exposed; and the result is a ~25 MB binary whose only
runtime requirements are glibc and the D-Bus session bus that Plasma already runs.

The honest cost is the build graph: Tokio, tonic and Slint pull several hundred crates to
compile. That is a *source* dependency count, not a *runtime* one — nothing has to be
installed on the target machine, and `cargo-deny` in the nightly tier (§6) keeps the graph
auditable.

Alternatives considered:

* **Kotlin/JVM + Compose Desktop** would have shared protocol code with the phone via a
  Kotlin Multiplatform module — the strongest anti-divergence story available. Rejected on
  criterion 4: it ships a JVM, idles around 200 MB resident for an app that autostarts at
  login, reaches the Plasma tray only through the legacy `xembedsniproxy` X11 shim, and
  renders through XWayland. Its directory-fsync path also depends on the non-obvious trick of
  opening a directory as a `FileChannel`. Protobuf (§5) recovers most of the code-sharing
  benefit without those costs.
* **C++/Qt 6 + KDE Frameworks** produces the most native Plasma application and the fewest
  distinct upstream projects. Rejected on criteria 1/4: manual lifetime management sits
  directly in the no-data-loss path, and self-containment fails from the other end — Qt has no
  mDNS responder (KDNSSD requires the `avahi-daemon` service) and no media metadata reader, so
  the install grows Qt, KF6, exiv2 and ffmpeg as runtime dependencies.
* **Go** matches the single-binary goal and covers more of the app from its standard library
  than anything else, but its GUI options are the weakest of the field: Gio is Wayland-native
  but niche, Fyne is X11-first, and neither reads as a desktop application on Plasma.
* **Python + PySide6** builds fastest and gets the Qt tray for free, but concurrent TLS
  streams with hashing in the path is the one workload it is worst at, and deployment becomes
  an interpreter plus a virtualenv.

### 3.2 GUI and tray

* **Slint 1.17** with the `backend-winit` + `renderer-femtovg` feature pair. FemtoVG is a pure
  Rust OpenGL renderer, so the Skia C++ toolchain never enters the build; winit talks Wayland
  natively, which matters on a fractional-scaling Plasma session. The UI surface is small — a
  device list with progress, a staged-sessions pane with the manual **Commit now** action, a
  settings page, and a pairing dialog — and `.slint` markup expresses it in a fraction of the
  code an imperative toolkit needs.
* **ksni 0.3** for the tray. It implements the `StatusNotifierItem` specification that Plasma
  actually consumes, and it is built on **zbus 5** (pure Rust) with a Tokio integration
  feature, so the tray needs neither libdbus nor the XEmbed bridge. `iced` and `egui` were the
  permissive-license fallbacks; they are unnecessary because the project is GPL-3.0 (§7),
  which is exactly the license Slint offers for open-source use.
* **Translations** use Slint's `@tr(...)` macro backed by gettext `.po` files. Simplified
  Chinese and English ship from day one on this end too — the same family operates both halves,
  and `SPEC.md` §3 requires it of the phone. `libintl` lives inside glibc on Linux, so this
  adds no dependency.

### 3.3 Concurrency model

The desktop is split into a **sans-I/O core** and a thin **shell**, and the boundary is the
most important structural rule in the codebase:

* The **core** crate holds all of `SPEC.md` §6, §7 and §8 — session scheduling, diff, staging
  manifest, commit state machine, recovery, dedup, naming, deletion nomination. It consumes
  `Event` values and emits `Effect` values. It never awaits, never spawns, never reads a clock,
  and never opens a file. Its `Cargo.toml` does not depend on Tokio, rusqlite or tonic at all,
  so the dependency graph enforces the rule; `clippy.toml` adds `disallowed-types` for
  `std::fs`, `std::time` and `std::thread` on top.
* The **shell** turns effects into syscalls and syscalls into events. Tokio's multi-threaded
  runtime drives it, because tonic requires one.

This exists so that the entire core can run inside a deterministic simulator (`VERIFICATION.md`
§L3), which is what makes the durability claims of §7 verifiable rather than merely asserted.
It costs an indirection at the boundary and pays for it in every crash test.

Within the shell:

* Network streams and RPC handlers are async.
* Every file operation — write, `fdatasync`, `rename`, hash update — runs on
  `tokio::task::spawn_blocking`. Async filesystem APIs buy nothing here (the kernel offers no
  async `fsync`) and would obscure exactly the ordering the spec depends on.
* The global commit lock of §7.3 is a plain `tokio::sync::Mutex`; per-device locks are held
  from finish-signal to commit completion, as specified. Commits are pure metadata work, so
  serializing them costs nothing measurable.

### 3.4 Durability primitives

The spec's guarantees map to specific calls. This table is the contract the fault-injection
tests of `VERIFICATION.md` §L1 are written against:

| `SPEC.md` requirement | Implementation |
|---|---|
| Periodic `.part` durability (§7.6) | `File::sync_data()` every 16 MB — `fdatasync` also flushes the size change needed to read the data back |
| Watermark advance (§7.6) | Manifest transaction committed *after* the `sync_data` returns |
| Truncate to watermark on startup (§7.6) | `File::set_len(durable_bytes)` |
| Commit rename (§7.3) | `std::fs::rename` — staging lives on the vault filesystem, so it is atomic |
| Directory barrier before done-marks (§7.3) | `File::open(dir)?.sync_all()` on the vault and staging directories; opening a directory read-only and fsyncing its descriptor is supported on Linux |
| Write-log seal (§7.3) | A single SQLite transaction (§3.5) |
| Free-space check (§4) | `rustix::fs::statvfs` on the vault path |
| Vault-copy verification at nomination (§8) | `std::fs::metadata` on the recorded vault name |
| Applying the phone's mtime (§6.4) | `rustix::fs::utimensat` with the catalog's `DATE_MODIFIED` seconds |

`rustix` is used where `std` has no equivalent; it issues Linux syscalls directly without
going through libc wrappers.

### 3.5 Persistence

**rusqlite 0.40** with the `bundled` feature — SQLite is compiled into the binary, so there is
no `libsqlite3` runtime dependency and no version skew with the distribution. **refinery 0.9**
manages schema migrations as versioned `.sql` files. Both databases run `journal_mode=WAL`
and `synchronous=FULL`.

Two databases, mirroring the spec's separation of concerns:

* **`$XDG_DATA_HOME/photo-sync/index.db`** — the import index of §7.5 (`content` keyed by
  sha256, `device_file` keyed by `(device_id, device_path)`). Outside the vault, so vault
  relocation or curation never touches it.
* **`<vault>/.staging/<deviceID>/manifest.db`** — the staging manifest of §7.6 *and* the
  commit write-log of §7.3, as separate tables in one file on the vault filesystem.

Putting the write-log in SQLite strengthens §7.4 rather than merely implementing it: "write
the map, flush, seal with an end marker" collapses into one atomic transaction, so a
half-written log physically cannot exist — the recovery branch for an unsealed log is kept as
defensive code for a state the storage layer no longer produces. Done-marks are an `UPDATE`
committed *after* the directory fsyncs return, preserving the ordering §7.3 requires. The
staging clear order (write-log → manifest → files) becomes two committed transactions followed
by the unlinks, so the invariant that survives a crash between them is unchanged.

Watermark updates cost one transaction per 16 MB — about 64 commits per gigabyte, negligible
against the I/O they are protecting.

`sqlx` was considered for its compile-time-checked queries, which appeal to the same instinct
that chose protobuf. It was rejected because it couples an async runtime to code that already
runs under a global lock, and it requires either a live database or an offline query cache at
build time.

### 3.6 Networking

**tonic 0.14** (gRPC over HTTP/2) with **prost 0.14**, served over **rustls 0.23**:

* The server is constructed with `serve_with_incoming` over a custom rustls acceptor, rather
  than tonic's built-in TLS config. This is what makes §5.2's pinning exact: a custom
  `ClientCertVerifier` ignores X.509 chain semantics entirely and compares the SHA-256 of the
  peer's `SubjectPublicKeyInfo` against the pinned value for that device. The verifier has a
  second mode used only while pairing is open (§3.9), where it accepts any client certificate
  and records its SPKI.
* The verified device identity is attached to each connection via tonic's `Connected` trait and
  read from request extensions, so no RPC has to trust a device ID sent in a message body.
* **Flow control is set explicitly.** HTTP/2's default 65,535-byte window caps throughput at
  `window / RTT`, which on a multi-millisecond Wi-Fi RTT is far below link speed. The server
  sets an 8 MB initial stream window with adaptive sizing enabled; the phone sets the matching
  value on its channels.
* **Concurrency is N connections, not N streams on one connection.** A pool of four channels
  each carry one `UploadFile` at a time. A single TCP connection multiplexing all uploads would
  make one lost segment stall every file behind it, and would share one congestion window;
  separate connections confine both effects and are what §6.4 means by concurrent streams.
* Keepalive pings every 20 s with a 10 s timeout detect a dropped Wi-Fi association quickly
  enough for the reconnect path of §6 to feel instant.

HTTP/3 was investigated and rejected for v1: gRPC is specified over HTTP/2, grpc-java has no
HTTP/3 transport, and no maintained pure-JVM QUIC stack exists — the phone would need `quiche`
or MsQuic through JNI with NDK builds. What QUIC would provide (independent streams, survival
across network changes) is already covered by the N-connection model and by the
reconnect-and-re-diff design of §6.

### 3.7 Discovery

**`mdns-sd` 0.21**, a pure-Rust DNS-SD responder advertising `_photosync._tcp` with TXT
records carrying the protocol version and the desktop's display name. It binds port 5353 with
`SO_REUSEPORT`, so it coexists with Avahi if that daemon is ever enabled on the machine.

Registering through Avahi's D-Bus API was the battle-tested alternative and was rejected on
criterion 4: it makes `avahi-daemon` a hard runtime requirement for an application that
otherwise needs no system services.

### 3.8 Capture-time extraction

**`nom-exif` 3.7** reads both sources §7.2 names — EXIF `DateTimeOriginal` from JPEG/HEIF and
`creation_time` from MOV/MP4 containers — in one pure-Rust crate. Anything it cannot parse
falls through to the mtime and import-time fallbacks the spec already defines, so coverage gaps
degrade into cosmetic naming rather than errors. Shelling out to `ffprobe` or `exiftool` would
cover more formats at the cost of self-containment and a process spawn per file.

### 3.9 Desktop integration

* **Suspend inhibition** (§4) via `zbus` calling `org.freedesktop.login1.Manager.Inhibit`
  with `what="sleep"`, `mode="block"`, holding the returned file descriptor while any session
  is active.
* **Autostart** (§4) by writing `~/.config/autostart/photo-sync.desktop` when the setting is
  enabled. No systemd user unit: the app needs the graphical session for its tray and its
  inhibitor, so it is a session application, not a service.
* **Paths** follow the XDG base directory specification — config in
  `$XDG_CONFIG_HOME/photo-sync/config.toml` (serde + `toml`), data in `$XDG_DATA_HOME`, logs in
  `$XDG_STATE_HOME`. The vault path is one key in that config file, a plain setting exactly as
  §4 describes.
* **Logging** with `tracing` + `tracing-subscriber`: human-readable to stderr, plus a rolling
  file appender in `$XDG_STATE_HOME/photo-sync/`. Commit and recovery paths log at INFO with
  structured fields (device, batch size, entry counts) so that a post-incident reconstruction
  is possible from logs alone.

### 3.10 Packaging

A PKGBUILD producing one binary plus a `.desktop` entry and icons, published through the
existing self-hosted package repository. No post-install service enablement, no daemon.

## 4. Mobile

### 4.1 Baseline

Kotlin, Jetpack Compose with Material 3, `minSdk 31` (Android 12, per `SPEC.md` §3),
`targetSdk` at the current stable API level, Gradle Kotlin DSL with a version catalog.
Simplified Chinese and English string resources from the first commit; no hardcoded UI text.

### 4.2 Architecture

Single activity, Compose navigation, `ViewModel` + `StateFlow` for screen state. Dependencies
are wired by hand in a small `AppContainer` — the graph is roughly six objects (media store
reader, discovery client, channel pool, session state machine, pairing store, notification
controller), which is well under the threshold where a DI framework repays its build-time and
indirection costs. Hilt would add KSP to the build; Koin would move wiring errors to runtime,
which cuts against the type-safety reasoning behind the protobuf contract.

The session state machine is a plain Kotlin class owned by the foreground service, holding the
frozen catalog in memory (§3.5 of the spec). **There is no database on the phone** — the only
durable state is the pairing record in `DataStore<Preferences>`, with the private key held in
the Android Keystore where it cannot be extracted.

### 4.3 Media access

* Enumeration through `ContentResolver` over `MediaStore.Files` filtered to the
  `DCIM/Camera/` `RELATIVE_PATH` and to image/video media types, projecting
  `RELATIVE_PATH`, `DISPLAY_NAME`, `SIZE`, `DATE_MODIFIED`. MediaStore queries exclude
  trashed and pending items by default; the query passes the corresponding match arguments
  explicitly so the exclusion §3.2 requires is visible in the code.
* **`mtime` is defined as `DATE_MODIFIED`: whole seconds since the epoch.** That integer is
  what the catalog carries, what the index stores, and what the desktop applies to the vault
  copy — a single definition on both ends, so the `(path, size, mtime)` identity of §3.2 can
  never mismatch through a precision difference.
* File bytes are read via `openInputStream` on the content URI. Scoped storage means no raw
  filesystem paths are ever used.
* Permissions: `READ_MEDIA_IMAGES` + `READ_MEDIA_VIDEO` on API 33+, `READ_EXTERNAL_STORAGE` on
  API 31–32, plus `MANAGE_MEDIA` requested through `ACTION_REQUEST_MANAGE_MEDIA`.
* Deletion uses `MediaStore.createDeleteRequest` launched through
  `ActivityResultContracts.StartIntentSenderForResult`, batched at roughly 500 URIs per request
  to stay inside binder transaction limits. With `MANAGE_MEDIA` granted the request completes
  without a dialog; without it the system confirmation appears, which is the fallback §8
  describes.

### 4.4 Keep-alive

Foreground service with `android:foregroundServiceType="dataSync"` and the
`FOREGROUND_SERVICE_DATA_SYNC` permission, a `PARTIAL_WAKE_LOCK`, and a
`WifiManager.WifiLock` in `WIFI_MODE_FULL_LOW_LATENCY`, with a progress notification.

One platform constraint to design around: since Android 15, `dataSync` foreground services are
subject to a cumulative daily runtime cap. It does not threaten normal use — a 50 GB first-run
backlog is well under an hour at LAN speeds — and if it ever triggers, the consequence is
exactly a connection drop: staging and its manifest persist, and the next launch resumes from
the desktop's watermark. `WorkManager` is deliberately not used; sessions are user-initiated
and intentionally do not survive process death (§3.5).

### 4.5 Networking

**grpc-okhttp** (the transport intended for Android, considerably lighter than grpc-netty)
with **grpc-kotlin** coroutine stubs and **protobuf-javalite** codegen. Four `ManagedChannel`s
form the upload pool, each its own TCP connection, with the initial flow-control window raised
to match the server.

TLS goes through a custom `SSLSocketFactory` installed on the channel builder:

* a custom `X509TrustManager` that ignores chain validation and compares the SHA-256 of the
  server's SPKI against the pinned desktop key;
* a custom `KeyManager` returning the Keystore-resident P-256 private key, so client
  authentication is performed by hardware that never releases the key material.

Discovery uses the platform `NsdManager` — no library, and no multicast lock bookkeeping of our
own. jmDNS was considered so that both ends would run inspectable, identical implementations;
it was rejected as an APK dependency added to work around quirks that may not appear.

### 4.6 Pairing

Each side generates a long-lived self-signed P-256 certificate on first run (`rcgen` on the
desktop, `KeyGenParameterSpec` on the phone, StrongBox-backed where the device offers it). The
pairing code shown on the desktop and confirmed on the phone is a short digest over *both*
SPKIs. Because the code binds both keys, an active man-in-the-middle necessarily produces
different codes on the two screens, so a mismatch is visible rather than silent. Pairing mode
must be explicitly opened on the desktop; outside that window the verifier accepts pinned peers
only.

### 4.7 Hashing

`MessageDigest.getInstance("SHA-256")`, which Conscrypt backs with ARMv8 cryptographic
extensions on every target device — comfortably ahead of both flash read speed and the Wi-Fi
link, so the streaming hash of §6.4 and the re-hash gate of §8 are I/O-bound, not CPU-bound.

### 4.8 Distribution

A self-hosted F-Droid-format repository built with `fdroidserver`, giving the family automatic
updates without a Play Console account, without MANAGE_MEDIA review, and without Google's
`targetSdk` schedule. The Play-compliant permission model of `SPEC.md` §3.1 remains a
self-imposed design constraint.

**One operational rule follows from the stack:** the upstream signing keystore must be
preserved forever and backed up off-machine. `ANDROID_ID` — the device identity that keys
staging directories and index rows (§2 of the spec) — is derived per app *signing key*. Losing
the keystore re-identifies every phone, orphaning its staging directory and stranding its index
rows, which the spec's failure analysis treats as duplicate re-imports rather than data loss,
but it is entirely avoidable.

## 5. The shared contract

### 5.1 Repository layout

```
Photo-Sync/
├── proto/photosync/v1/*.proto     single source of truth for the wire format
├── desktop/                       cargo workspace (core, shell, model, sim, xtask)
├── android/                       gradle project
├── verification/                  requirements, scenarios, vectors, corpus, reports
├── docs/                          SPEC.md, STACK.md, VERIFICATION.md
└── AGENTS.md                      operating rules for implementation work
```

Names are fixed once and used everywhere: the desktop binary and package are `photo-sync`, the
Rust crates are `photo-sync-core`, `photo-sync-model`, `photo-sync-sim`, `photo-sync-protocol`,
and `photo-sync` for the shell, and the Android application identifier is
`top.wxx9248.photosync`.

One repository. A schema change breaks both builds in the same commit, which is the entire
point: the two ends are written in different languages, and the `.proto` is the only mechanism
that makes divergence a compile error instead of a runtime data-loss bug.

### 5.2 Code generation

Generated at build time on both ends — `tonic-build` in the desktop's `build.rs`, the
`protobuf-gradle-plugin` with the grpc-java and grpc-kotlin codegen plugins on Android. No
generated sources are committed. `prost-build` needs a `protoc`, which the distribution's
`protobuf` package supplies, and `protoc-bin-vendored` is available if a fully hermetic build is wanted later.

To keep compatibility auditable without adding tooling, CI checks in a
`FileDescriptorSet` produced from the current schema and fails the build when a change to it is
not accompanied by an updated descriptor — making every wire-format change explicit in review.

### 5.3 Service surface

Mapping directly onto `SPEC.md` §6:

```protobuf
service PhotoSync {
  rpc Handshake(HandshakeRequest) returns (HandshakeResponse);        // §5.2 version, device id, name
  rpc SubmitCatalog(stream CatalogChunk) returns (CatalogAck);        // §6.1 frozen catalog, 1000 entries/message
  rpc GetDiff(DiffRequest) returns (stream DiffChunk);                // §6.2 to-send list + resume offsets
  rpc UploadFile(stream FileChunk) returns (UploadResult);            // §6.4-6.5 header first, then 512 KB chunks
  rpc Finish(FinishRequest) returns (stream FinishChunk);             // §6.6 commit, then deletion candidates (§8)
  rpc ReportDeletions(stream DeletionReport) returns (SessionSummary); // §8 per-file results
}

service Pairing {
  rpc Pair(PairRequest) returns (PairResponse);                       // §5.2, reachable only while pairing is open
}
```

Catalog and diff are streamed because a 50,000-entry library exceeds gRPC's default 4 MB
message limit; splitting them into chunked RPCs is preferred over raising the limit, since it
bounds memory on both ends. Deletion candidates and deletion results stream for the same
reason. `UploadFile` sends a header message with the file id and resume offset, then the bytes,
then a trailer carrying the digest, which is computed while reading and is therefore not known
when the stream opens; §7.6 needs no in-band acknowledgements because the resume offset always
comes from the next diff, never from the phone's own counter.

### 5.4 Compatibility rules

* The protocol version appears in the mDNS TXT record *and* in `HandshakeRequest`; the desktop
  rejects an unknown major version with a plain-language message rather than negotiating.
* Field numbers are never reused or renumbered; removed fields are `reserved`.
* Every new field is optional with a safe default, so a phone that has not updated yet keeps
  working.

### 5.5 Error taxonomy

gRPC status codes carry the spec's outcomes: `FAILED_PRECONDITION` for the free-space shortfall
of §4, `DATA_LOSS` for a hash mismatch (§6.5), `ABORTED` for a device whose previous commit is
still running (§7.3's "finishing previous import…"), `UNAUTHENTICATED` for an unpinned peer.
The phone maps each to one of the user-facing messages the spec names; unknown codes surface as
a generic retryable error rather than a silent skip.

## 6. Verification tooling

The verification architecture — the sans-I/O core requirement, the deterministic simulator, the
executable model, the requirement registry, and the gates that define "done" — is specified in
`VERIFICATION.md`. What belongs here is the tooling it is built from:

| Tool | Role |
|---|---|
| `cargo-nextest` | test runner; per-test process isolation and machine-readable results |
| `proptest` | property generation and automatic shrinking of failing traces |
| `sqlite-vfs` / `libsqlite3-sys` | custom VFS so SQLite runs on the simulated filesystem |
| `cargo-mutants` | meta-oracle: does the suite detect a wrong implementation? |
| `cargo-deny`, `cargo-audit` | dependency licence policy and advisory audit |
| `insta` | snapshot assertions for report and scenario output |
| Gradle Managed Devices (ATD) | headless, reproducible Android instrumentation runs |
| `cargo xtask` | the `./verify` runner; pure Rust, no extra tooling to install |

Two consequences reach back into the stack itself. The core crate must not depend on any I/O
crate, which is why §3.3 splits core from shell. And SQLite is reached through rusqlite in a way
that permits registering a custom VFS, so the manifest and write-log of §3.5 can be crash-tested
inside the simulator rather than trusted.

## 7. License

GPL-3.0 for both applications. Slint is dual-licensed and is used under its GPL-3.0 option,
which requires no attribution bookkeeping and no royalty-free-license terms to track. Every
other dependency is permissively licensed; `cargo deny` enforces that in CI so a transitive
change cannot quietly introduce an incompatible license.

---

## Appendix A — Decision log

| # | Decision |
|---|---|
| T1 | Desktop in Rust: one static binary, direct syscall control over §7 durability, no runtime dependencies beyond glibc and the session D-Bus |
| T2 | Slint (winit + FemtoVG, pure Rust) for the desktop UI; GPL-3.0 option |
| T3 | `ksni` over zbus for a real StatusNotifierItem tray — no XEmbed shim, no libdbus |
| T4 | Tokio for async networking; all file I/O and hashing on `spawn_blocking` |
| T5 | gRPC over HTTP/2 (tonic + prost / grpc-okhttp + grpc-kotlin), one `.proto` as the cross-language contract |
| T6 | Bulk transfer over four independent channels, not multiplexed streams; HTTP/2 windows raised to 8 MB |
| T7 | HTTP/3 and QUIC rejected for v1: no viable Android client without JNI-bundled native QUIC |
| T8 | mTLS with self-signed P-256 certificates and mutual SPKI pinning via a custom rustls verifier; SAS pairing code binds both keys |
| T9 | Phone key in the Android Keystore (StrongBox where available), used through a custom `KeyManager` |
| T10 | `mdns-sd` in-process responder on the desktop, `NsdManager` on the phone; no Avahi dependency |
| T11 | rusqlite with bundled SQLite + refinery migrations; WAL, `synchronous=FULL` |
| T12 | Index DB in `$XDG_DATA_HOME`; staging manifest and commit write-log as tables in `<vault>/.staging/<deviceID>/manifest.db` |
| T13 | The §7.3 write-log seal is a single SQLite transaction, so an unsealed log cannot physically exist |
| T14 | `nom-exif` for EXIF and MOV/MP4 capture times; no ffmpeg or exiftool |
| T15 | Android: Compose + Material 3, manual `AppContainer` DI, no annotation processor, no database |
| T16 | `mtime` is defined as MediaStore `DATE_MODIFIED` in whole seconds on both ends |
| T17 | Foreground service (`dataSync`) + wake lock + Wi-Fi lock; no WorkManager, since sessions are not meant to survive process death |
| T18 | Monorepo with `/proto` as the source of truth; codegen at build time, generated sources not committed |
| T19 | A committed `FileDescriptorSet` makes every wire-format change explicit in review |
| T20 | Desktop split into a sans-I/O core and an adapter shell, so the core runs inside a deterministic simulator |
| T21 | Verification architecture, gates and agent workflow live in `VERIFICATION.md`; operating rules in `AGENTS.md` |
| T22 | Self-hosted F-Droid-format repository; the signing keystore is preserved permanently because `ANDROID_ID` derives from it |
| T23 | GPL-3.0 for both applications; `cargo deny` enforces dependency license policy |
