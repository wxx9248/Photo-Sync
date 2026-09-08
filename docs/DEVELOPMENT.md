# Development environment

What to install, where each part of the work runs, and the container settings each milestone
needs.

## Where work runs

| Work | Where |
|---|---|
| Desktop code, simulator, campaigns, mutation testing | Development container |
| Kotlin session module, Android application, instrumented tests | Development container |
| Desktop user interface, tray, suspend inhibition | Your Plasma session |
| Discovery and sessions with a real phone | Your Plasma session, or the container on a bridged interface |
| Every push and pull request | GitHub hosted runners |

The container is an Arch system container under libvirt with systemd as its init and a
directory-backed root on the host's ext4. That matters in two ways. Real filesystem semantics
apply, so the tests that use real files behave the way the specification assumes, and the vault
and its staging directory land on one filesystem, which is what makes commit by `rename()`
atomic.

Two things do not belong in the container. The graphical shell needs a running Plasma session
for its tray item and its suspend inhibitor. Discovery needs multicast to reach the phone, which
the libvirt NAT network does not carry.

## Container setup

```sh
pacman -S --needed base-devel git rust protobuf jdk21-openjdk android-tools
cargo install cargo-nextest cargo-mutants cargo-deny cargo-audit
```

Arch's `rust` package includes `rustfmt` and `clippy`, so no separate components are needed.
Install JDK 21 rather than a newer one, because the Android Gradle plugin supports it and later
versions lag.

The graphical shell arrives in milestone M7 and needs a few more packages before it builds:

```sh
pacman -S --needed fontconfig libxkbcommon wayland mesa
```

Then clone and check the setup:

```sh
git clone git@github.com:wxx9248/Photo-Sync.git
cd Photo-Sync
git config core.hooksPath hooks
./verify quick
```

The runner reports a missing tool and skips that check rather than failing, so a partial setup
still gives useful output.

## Container settings each milestone needs

**M6, discovery and real sessions.** The default libvirt NAT interface does not carry multicast
to the local network, so `_photosync._tcp` is neither advertised to nor visible from a phone.
Give the container a bridged or macvtap interface on the same network as the phones before this
milestone, or run those tests from your desktop.

**M8, Android instrumented tests.** The emulator needs `/dev/kvm`, which the container does not
currently have. Pass the device through, or run the emulator on the host and point the tests at
it over adb.

## Continuous integration

`.github/workflows/verify.yml` runs the quick and full tiers on every push and pull request. It
installs the Rust toolchain, `protoc`, and `cargo-nextest`, then calls `./verify full` and keeps
the JSON report as an artifact. Anything CI does goes through `./verify`, so a check that passes
locally passes the same way there.

Nightly campaigns and mutation testing want the container registered as a self-hosted runner.
That job is added once the runner exists, because a scheduled job with no runner produces queued
builds rather than results.

**A self-hosted runner on a public repository runs code from pull requests, including from
forks.** Restrict the self-hosted job to `push` on `main` and to `schedule`. Never let
`pull_request` reach it.

## Running the tiers

```sh
./verify quick      # constantly, while working
./verify full       # before a commit that touches the core
./verify nightly    # in the container, unattended
```

See `docs/VERIFICATION.md` for what each tier contains and `AGENTS.md` for the rules that apply
to a change.
