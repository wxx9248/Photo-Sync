# Bootstrap

Read this first. It gives you what you need to begin work, in the order you need it, and points
at the document that answers each question properly.

## 1. Check the machine

```sh
./verify doctor
```

It lists every tool, says which tiers can run here, and gives the exact command for anything
missing. A missing tool is skipped rather than failed, so a partial setup still works. If the
machine is new, `docs/DEVELOPMENT.md` has the full setup and explains what runs where.

Configure the hooks once per clone, as `doctor` will remind you:

```sh
git config core.hooksPath hooks
```

## 2. What this project is

Two applications. An Android app pushes the camera roll to a Linux desktop over the home
network. The desktop receives, verifies, deduplicates, renames by capture time, archives into a
vault, and owns all the state. Once the desktop can prove it holds a file, the phone is offered
the chance to delete it and free space.

The first design priority is that no photo is ever lost, and it outranks everything else. Most
of the difficulty in this codebase comes from that one sentence.

## 3. Read in this order

| Document | What it decides | When to read it |
|---|---|---|
| `docs/SPEC.md` | What the system does | The sections your task touches, before you write anything |
| `docs/ROADMAP.md` | Which milestone is current and what it must prove | At the start of a task |
| `AGENTS.md` | The rules your change has to satisfy | In full. It is short |
| `docs/VERIFICATION.md` | How correctness is decided | Before writing tests |
| `docs/CONVENTIONS.md` | How code and documents are written | In full, plus the file for your language |
| `docs/STACK.md` | What it is built from and why | When adding a dependency or touching an adapter |
| `docs/DEVELOPMENT.md` | Where each kind of work runs | When something cannot be tested here |

`SPEC.md` is the authority. If the code disagrees with it, the code is wrong. If the
specification itself is wrong, say so and change it in its own commit rather than working
around it.

## 4. Where the work is now

Milestone M0 is complete, and M1 is in progress. `docs/ROADMAP.md` defines both.

What exists: the protobuf schema, the Rust workspace, the `Event` and `Effect` vocabulary with
the port traits and typed store requests in `photo-sync-core`, the `./verify` runner with its
quick tier, the requirement registry with seventy four entries, and skeletons for the model, the
simulator, the shell, and the Android project.

What does not exist yet: the session state machine, the simulator and its filesystem, the model
transition function, the gRPC server, storage adapters, the user interface, and the Android
application. M1 builds the thinnest path through all of it.

## 5. The working loop

1. Read `verification/reports/latest.json`. It is the state of the world when you arrive.
2. Take the task from the current milestone in `docs/ROADMAP.md`.
3. Find the requirement identifiers it touches in `verification/requirements.toml`. If none
   covers it, add one with a verbatim quote from `SPEC.md`. If no clause in `SPEC.md` supports
   the behavior, stop: the specification needs changing first, and that is a separate commit.
4. Write the code and its tests. Declare coverage at the test site with `covers!` in Rust,
   `@Covers` in Kotlin, or a `covers` key in a scenario.
5. Run `./verify quick` constantly while working. It takes seconds.
6. Run `./verify full` before any commit that touches the core.
7. Commit one logical change at a time, with the reason in the message.

## 6. Rules that are not negotiable

`AGENTS.md` has all of them. These four are the ones that get broken first.

The core crate performs no input or output. It consumes events and emits effects, and the shell
does the rest. This is what lets the whole core run inside a deterministic simulator.

Never edit a protected artifact to make a test pass. The requirement registry, the golden
vectors, the model crate, and the expectations of existing scenarios define what correct means.
Changing one requires a matching `SPEC.md` change in the same commit.

Every behavior change references a requirement identifier.

Every failing seed gets committed to the corpus with its fix.

## 7. When you are stuck

If two readings of the specification would produce different code, stop and ask. Guessing
produces plausible code, and plausible code is the hardest kind of wrong to catch in review.

If the model or a scenario expectation looks wrong, say so and leave it alone. An oracle edited
to agree with the implementation has stopped being an oracle.

If a tool is missing, `./verify doctor` names the command that installs it.

If something cannot be tested on this machine, such as the graphical shell or a session with a
real phone, `docs/DEVELOPMENT.md` says where it runs instead.

## 8. Before you say you are done

`AGENTS.md` states the gate in full. In short: every requirement you touched has a passing test
that declares it, `./verify quick` and `./verify full` are green, mutation testing on the core
modules you changed scores at or above 0.85, and no protected artifact changed without a
specification change.

Report the result honestly. A partly finished change described accurately is useful. A green
summary that does not match the report is not.
