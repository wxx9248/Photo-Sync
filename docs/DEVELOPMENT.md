# Development environment

Where each part of the work runs, how to set the machine up, and what a few milestones need
beyond the defaults.

## The development machine

A local x86_64 virtual machine running Fedora 44 KDE, with 16 processors and 16 GiB of memory.
Three details of its configuration decide what can be tested inside it:

* The processor is passed through and the host allows nested virtualisation, so the guest has a
  working `/dev/kvm`. The Android emulator runs accelerated.
* The display adapter is `virtio-vga-gl` with 3D acceleration, so the graphical shell renders
  through real OpenGL rather than falling back to software.
* The network interface is on a libvirt NAT network. Outbound access works. Multicast does not
  reach the local network, which matters only for a real phone and is covered below.

Almost everything happens there: the desktop application, the simulator and its campaigns,
mutation testing, the Kotlin session module, the Android application, and instrumented tests on
the emulator.

Two things happen elsewhere. Acceptance with a real phone belongs on your desktop, which is the
deployment target and is already on the same network as the phones. The Arch package build in
milestone M9 also belongs there, or in an Arch container, because Fedora cannot exercise a
PKGBUILD.

## Setup

```sh
sudo dnf install -y git rustup protobuf-compiler gcc gcc-c++ pkgconf-pkg-config \
    java-21-openjdk-devel

rustup-init -y
rustup default stable
rustup component add rustfmt clippy

cargo install cargo-nextest cargo-mutants cargo-deny cargo-audit
```

Install JDK 21 rather than a newer one, because the Android Gradle plugin supports it and later
versions lag behind.

The graphical shell arrives in milestone M7 and needs a few more development packages:

```sh
sudo dnf install -y fontconfig-devel libxkbcommon-devel wayland-devel \
    mesa-libEGL-devel mesa-libGL-devel
```

Then clone and check the setup:

```sh
git clone git@github.com:wxx9248/Photo-Sync.git
cd Photo-Sync
git config core.hooksPath hooks
./verify quick
```

Give the machine a deploy key scoped to this repository rather than a personal key, so that
nothing running there can reach your other repositories.

The runner reports a missing tool and skips that check rather than failing, so a partial setup
still gives useful output.

## Testing discovery

Three cases look similar and need different things.

**The discovery code itself**, meaning the mDNS responder and the browse path, is tested inside
the virtual machine with the Rust test client as the peer. Nothing special is required.

**Emulator sessions do not need discovery.** Point the application at an address, or use
`adb forward`, and the emulator covers what only Android can answer: MediaStore, permissions,
the foreground service, and the delete request. Exercising `NsdManager` against the emulator is
a separate matter, because the emulator gives its guest a user-mode network that does not carry
multicast. That needs the emulator started with TAP networking, not a change to the virtual
machine.

**A real phone on your network** is the only case that needs the virtual machine bridged onto
the local network with macvtap. This is acceptance work, and it runs on your desktop instead,
along with the manual matrix for the OEM keep-alive behavior that no emulator can reproduce.

## Continuous integration

`.github/workflows/verify.yml` runs the quick and full tiers on every push and pull request. It
installs the Rust toolchain, `protoc`, and `cargo-nextest`, then calls `./verify full` and keeps
the JSON report as an artifact. Everything CI does goes through `./verify`, so a check that
passes locally passes there for the same reasons.

Nightly campaigns and mutation testing want the virtual machine registered as a self-hosted
runner. That job is added once the runner exists, because a scheduled job with no runner
produces queued builds rather than results.

**A self-hosted runner on a public repository runs code from pull requests, including from
forks.** Restrict the self-hosted job to `push` on `main` and to `schedule`. Never let
`pull_request` reach it.

## Running the tiers

```sh
./verify quick      # constantly, while working
./verify full       # before a commit that touches the core
./verify nightly    # on the development machine, unattended
```

See `docs/VERIFICATION.md` for what each tier contains and `AGENTS.md` for the rules that apply
to a change.
