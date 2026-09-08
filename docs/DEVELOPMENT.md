# Development environment

What to install, where each part of the work runs, and why the two halves of the project do not
run in the same place.

## Where work runs

| Work | Where | Why |
|---|---|---|
| Desktop code, simulator, campaigns, mutation testing | Fedora aarch64 virtual machine | Architecture independent, and the long tiers want a machine that is not your desktop |
| Kotlin session module | Anywhere with a JDK | It holds no Android types, so it is an ordinary JVM library |
| Android application build | x86_64 only | Google publishes `aapt2` for `linux` but not `linux-arm64` |
| Android instrumented tests | x86_64 only | Google publishes no Android emulator for aarch64 Linux |
| Pushes and pull requests | GitHub hosted runners | Fast feedback on every change |

The split is not a preference. Building the app on aarch64 Linux needs an unofficial `aapt2`
through `android.aapt2FromMavenOverride`, and running the emulator there is not possible at all.
Use the x86_64 desktop or a hosted runner for anything Android, and treat the virtual machine as
the desktop and harness machine.

## Fedora aarch64 setup

```sh
sudo dnf install -y git rustup protobuf-compiler gcc gcc-c++ pkgconf-pkg-config \
    java-21-openjdk-devel

rustup-init -y
rustup default stable
rustup component add rustfmt clippy

cargo install cargo-nextest cargo-mutants cargo-deny cargo-audit
```

The graphical shell arrives in milestone M7 and needs a few more development packages before it
will build:

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

The runner reports a missing tool and skips that check rather than failing, so a partial setup
still gives useful output.

## Continuous integration

`.github/workflows/verify.yml` runs the quick and full tiers on every push and pull request. It
installs the Rust toolchain, `protoc`, and `cargo-nextest`, then calls `./verify full` and keeps
the JSON report as an artifact. Anything CI does goes through `./verify`, so a check that passes
locally passes the same way there.

Two tiers are not wired to CI yet.

**Nightly campaigns and mutation testing** want the virtual machine registered as a self-hosted
runner. The job is added once the runner exists, because a scheduled job with no runner produces
queued builds rather than results.

**Android instrumented tests** arrive with milestone M8 and need an x86_64 runner with KVM.

## Running the tiers

```sh
./verify quick      # constantly, while working
./verify full       # before a commit that touches the core
./verify nightly    # on the virtual machine, unattended
```

See `docs/VERIFICATION.md` for what each tier contains and `AGENTS.md` for the rules that apply
to a change.
