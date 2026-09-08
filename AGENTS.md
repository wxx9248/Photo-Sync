# Operating rules

Read this before changing anything in this repository.

## Documents

| File | Authority |
|---|---|
| `docs/SPEC.md` | What the system does. The authority on behavior. Nothing overrides it. |
| `docs/STACK.md` | What it is built from. Libraries, versions, architecture decisions. |
| `docs/VERIFICATION.md` | How correctness is decided. Read before writing tests. |
| `docs/CONVENTIONS.md` | How code and documents are written. Read before writing either. |
| `docs/CONVENTIONS-RUST.md`, `docs/CONVENTIONS-KOTLIN.md` | Language idioms for each end. |
| `docs/ROADMAP.md` | The order work happens in, and what each milestone must prove. |
| `docs/DEVELOPMENT.md` | Where each part of the work runs, and how to set a machine up. |
| `verification/requirements.toml` | Machine-readable projection of `SPEC.md`. Protected. |

If the code and `SPEC.md` disagree, the code is wrong. If `SPEC.md` is wrong, say so and change
it in its own commit. Do not work around it silently.

## Hard rules

1. **The core crate performs no I/O.** No `std::fs`, no `tokio`, no clock, no RNG, no threads.
   It consumes `Event` and emits `Effect`. All I/O lives in the shell. Clippy enforces this;
   do not add an allow.
2. **No ambient nondeterminism.** Time comes from the `Clock` port, randomness from `Rng`, and
   observable iteration is over ordered collections. A test that passes on rerun but not on
   replay is a bug in the code, not a flake.
3. **Never edit a protected artifact to make a test pass.** Protected artifacts are
   `verification/requirements.toml`, `verification/vectors/`, the model crate, and the
   `[expect]` blocks of existing scenarios. They define correctness. Editing one requires a
   matching `SPEC.md` change in the same commit, and the pre-push hook checks this.
4. **Every behavior change references a requirement ID.** If no ID covers it, add one to the
   registry with a verbatim quote from `SPEC.md`. If no clause in `SPEC.md` supports it, stop.
   The spec needs changing first.
5. **Every failing seed gets committed.** Drop it in `verification/corpus/` with the fix, in
   the same commit.

## Commands

```
./verify quick            # < 30 s,  run constantly while working
./verify full             # < 5 min, required before any commit touching core
./verify nightly          # hours,   runs on a timer, you normally only read its report
./verify replay <seed>    # reproduce one simulator failure exactly
./verify scenario <id>    # run one acceptance scenario (add --real for the real stack)
./verify spec-check       # registry quotes still match SPEC.md verbatim
./verify mutants <module> # does the suite actually detect wrong implementations here?
./verify self-test        # negative controls: prove the harness catches known-bad code
```

Start every session by reading `verification/reports/latest.json`.

Continuous integration runs `./verify full` on every push, so a change that passes locally passes
there for the same reasons.

## Reading a failure

The JSON report names, for each failure: the requirement ID, the `SPEC.md` section, a seed, a
one-line reproduction command, a trace file, and a structural diff of expected versus actual
state when the model disagrees.

Work in this order:

1. Run the reproduction command. It is deterministic; if it does not reproduce, that is the bug.
2. Read the `SPEC.md` section named in the failure. Decide what the correct behavior is from the
   spec, not from the current code.
3. Read the minimized trace. It is already the shortest failing sequence, so read it before
   adding any logging.
4. Fix the implementation. If you conclude the oracle is wrong, stop and say so explicitly
   rather than editing it.

## Definition of done

A change is complete when all four hold:

- every requirement ID it touches has a passing covering test, and no `active` requirement is
  unverified;
- `./verify quick` and `./verify full` are green, including corpus replay;
- `./verify mutants` on the changed core modules scores ≥ 0.85, with any surviving mutant killed
  or justified in one line in `verification/mutants-allow.toml`;
- no protected artifact changed without a `SPEC.md` change.

Report the result honestly. A partially finished change described accurately is useful; a green
summary that does not match the report is not.

## Writing tests

- Declare coverage at the test site: `covers!("R-COMMIT-014");` in Rust, `@Covers("...")` in
  Kotlin, `covers = [...]` in a scenario.
- Prefer an acceptance scenario (`verification/scenarios/*.toml`) when the behavior is
  observable at session level. It is reviewable by a human and runs in both simulation and the
  real stack.
- Prefer a property in the simulator when the behavior is about *any* interleaving of crashes,
  drops, or concurrent devices. Do not enumerate cases the simulator would generate.
- Assertions on internal state are a last resort. The oracle is the model; if the model cannot
  see the property, ask whether the property belongs in the `Effect` enum.

## Conventions

`docs/CONVENTIONS.md` holds the full list. These are the ones that go wrong most often, so they
are repeated here.

- Search for an existing component before writing a new one. The second implementation of a
  thing is what causes drift.
- Verify an API exists before calling it. Do not write library signatures from memory.
- Reproduce a failure before fixing it.
- Keep each change small enough to verify in one step.
- State an assumption out loud rather than encoding a guess in code, and stop and ask when two
  readings of the spec would produce different code.
- Do not expand scope. Unrequested work still costs review and maintenance.
- Write plainly. Short sentences, no unnecessary asides.

## Commits

One logical change per commit. Spec changes, oracle changes, and implementation changes are
separate commits. Never commit generated protobuf sources, report files, or emulator artifacts.
