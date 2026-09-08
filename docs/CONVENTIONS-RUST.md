# Rust conventions

Idioms for the desktop code. The general rules in `CONVENTIONS.md` apply here too, and this
document only covers what the language decides for us. Lint configuration lives in
`clippy.toml` and the workspace lint table.

## Types

R1 **Wrap domain identifiers in newtypes.** `DeviceId(String)` and `Sha256([u8; 32])` cannot be
swapped at a call site the way two `String` arguments can. See 2.4.

R2 **Derive deliberately.** `Debug` on public types helps every failure message. `Clone` derived
by reflex hides that a value is being duplicated on a hot path.

R3 **Use enums for closed sets and match without a catch-all arm.** Adding a variant then fails
to compile everywhere it matters, which is the point of the type.

R4 **Prefer borrowed parameters and owned returns.** Taking `&str` and returning `String` lets
callers decide about allocation.

R5 **Mark a function `#[must_use]` when ignoring its result is a bug.** The compiler is a better
reviewer than a comment.

## Errors

R6 **Use concrete error enums in the core, built with `thiserror`.** The caller can match on the
case and the harness can assert on it.

R7 **Keep `anyhow` in the shell.** Context chaining is right at the top level and wrong in a
library where the caller needs to decide.

R8 **No `unwrap` or `expect` in the core without a comment naming the invariant.** An
unexplained unwrap is a panic waiting for a message nobody can act on.

R9 **Reserve `panic!` for broken invariants.** Anything a peer, a file, or a user can cause is a
`Result`. See 3.5.

## Structure

R10 **Default to private, then `pub(crate)`, then `pub`.** Each widening is a commitment to keep
that item working.

R11 **Use `foo.rs` beside `foo/` rather than `mod.rs`.** The file name tells you which module you
are reading.

R12 **Keep `async` out of the core crate.** The core is driven by events and returns effects, so
it can run inside the simulator. `VERIFICATION.md` explains why this matters.

R13 **Write `let else` and early `return` instead of nested `if let`.** The happy path stays at
one indentation level. See 1.3.

R14 **Use iterator chains while they read as a sentence.** Past that, a loop with a name for the
intermediate value is clearer.

## Determinism

R15 **Use ordered collections wherever iteration order is observable.** `HashMap` iteration order
varies between runs and breaks replay from a seed.

R16 **Take time and randomness from injected ports.** A direct call to the system clock makes a
test pass today and fail on a leap second.

## Unsafe

R17 **No `unsafe` outside the SQLite VFS shim.** That shim exists because the C API requires it,
and nothing else in the project does.

R18 **Give every `unsafe` block a safety comment stating the invariant it relies on.** The
comment is the only review material for code the compiler cannot check.

## Tests

R19 **Put unit tests in `#[cfg(test)] mod tests` beside the code, and cross-module tests in
`tests/`.** Placement tells the reader the scope of what is being checked.

R20 **Prefer `proptest` over a hand written table when the input space is large.** Shrinking
turns a failure into the smallest example by itself.
