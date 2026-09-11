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
    java-25-openjdk-devel gettext

rustup-init -y
rustup default stable
rustup component add rustfmt clippy

cargo install cargo-nextest cargo-mutants cargo-deny cargo-audit
```

JDK 25 is the current long-term release and is what both halves target: the Kotlin modules set
`jvmToolchain(25)` and the Android Gradle plugin compiles against it. The document used to pin
21 on the assumption that the plugin lagged; it does not, and Fedora 44 has no 21 package
anyway, so the setup command failed as written.

Gradle itself is not installed: `android/gradlew` fetches the version the project uses, so a
clean machine needs only the JDK.

The graphical shell needs a few more development packages:

```sh
sudo dnf install -y fontconfig-devel libxkbcommon-devel wayland-devel \
    libX11-devel libXcursor-devel libXrandr-devel libXi-devel \
    mesa-libEGL-devel mesa-libGL-devel
```

The Android SDK is not packaged. Install the command-line tools somewhere of your choosing and
point `ANDROID_HOME` at it:

```sh
mkdir -p ~/Android/sdk/cmdline-tools
# unpack commandlinetools-linux-*.zip so that its `cmdline-tools` becomes `latest`
export ANDROID_HOME=~/Android/sdk
yes | $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --licenses
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --install \
    "platform-tools" "platforms;android-37.0" "build-tools;37.0.0"
```

Gradle finds the SDK through `ANDROID_HOME`, or through `android/local.properties` if you would
rather not export it in every shell:

```sh
echo "sdk.dir=$HOME/Android/sdk" > android/local.properties
```

That file is per-machine and is not committed. Without either, `./verify` skips the three
application-module checks and says so.

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

The nightly tier will run as a scheduled workflow on hosted runners, sharded across jobs so that
each stays inside the six hour limit that applies to a single job. Seed campaigns and mutation
testing both divide cleanly, and running them in parallel jobs finishes sooner than running them
in sequence on one machine.

No self-hosted runner is planned. It would only buy one uninterrupted long run, which sharding
gives another way, and a self-hosted runner on a public repository executes code from pull
requests, including from forks. The development machine still runs `./verify nightly` directly
whenever someone wants a deep campaign under their own eye.

## Running the tiers

```sh
./verify quick      # constantly, while working
./verify full       # before a commit that touches the core
./verify nightly    # on the development machine, unattended
```

See `docs/VERIFICATION.md` for what each tier contains and `AGENTS.md` for the rules that apply
to a change.
