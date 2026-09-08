# Conventions

Rules for writing code and documents in this repository. They are language neutral. Idioms for
a specific language live in `CONVENTIONS-RUST.md` and `CONVENTIONS-KOTLIN.md`.

Each rule states what to do and why. The reason matters as much as the rule, because it tells
you when a rule genuinely does not apply. Automated checks live in the lint configuration, not
here.

## 1. Units and structure

1.1 **Keep units small.** A function or class that grew large is holding more than one
responsibility, and splitting it is the fix rather than a nicety.

1.2 **Each unit does one thing and does it well.** A single responsibility makes a unit easy to
name, test, and reuse.

1.3 **Avoid deep nesting. Return early.** Guard clauses put the exceptional cases at the top and
leave the main path at one level of indentation.

1.4 **Treat deep nesting as a signal to extract.** Nesting usually means several decisions have
collected in one place, and each of them wants its own unit.

1.5 **Do not repeat yourself.** Before adding a component, search for one that already does the
job. Duplicated logic diverges as soon as one copy is fixed.

1.6 **Generalize an existing component when it almost fits.** One component with a small
extension costs less to maintain than two that resemble each other.

1.7 **Do not generalize on speculation.** Wait for the second real caller. An abstraction built
for one caller is usually shaped for the wrong problem.

1.8 **Use a design pattern when there is a need for extensibility, reuse, or maintainability.**
Applied without that need, a pattern adds indirection and nothing else.

1.9 **Make things easy by default.** The simplest thing that works is also the easiest to change
when requirements move.

1.10 **Organize modules for high cohesion inside and low coupling outside.** A module you can
describe in one sentence is a module you can change without reading its neighbours.

1.11 **Avoid circular dependencies between modules.** A cycle usually means a third module is
missing and the shared part belongs there.

## 2. Types and interfaces

2.1 **Make illegal states unrepresentable.** Modelling the domain so invalid combinations cannot
be constructed removes a whole class of runtime checks.

2.2 **Parse at the boundary, then trust the type.** Converting untrusted input into a typed value
once means downstream code never has to re-check it.

2.3 **Do not contort types to satisfy the two rules above.** When the encoding is harder to read
than a runtime check with an assertion, use the check.

2.4 **Give a domain concept its own type.** A device identifier passed as a plain string can be
swapped with any other string at a call site.

2.5 **Keep argument lists short.** When a unit needs several related values, group them into a
named type so the relationship is visible.

2.6 **Avoid boolean parameters that select behavior.** Two named functions read correctly at the
call site, where a bare `true` does not.

2.7 **Separate commands from queries.** A unit that changes state and also returns an answer is
hard to call safely and hard to test.

2.8 **Keep the public surface small.** Start private and widen only when a caller needs access,
because every public item becomes something you must keep working.

## 3. Errors and failure

3.1 **Separate bugs from expected failures.** A bug should be loud and fail fast, while an
expected failure is a value the caller handles.

3.2 **Never swallow an error silently.** If ignoring one is correct, one comment saying why turns
a suspicious line into a reviewed decision.

3.3 **Say what was expected and what happened.** An error that carries context can be acted on
without reproducing the failure first.

3.4 **Handle an error where the code can decide what to do about it.** Passing an error upward
unchanged is a valid choice and better than a guess.

3.5 **Do not assert on input you do not control.** An assertion on peer input turns a malformed
message into a crash.

## 4. State and data flow

4.1 **Keep one source of truth for each fact.** Derived copies drift, and then two parts of the
system disagree about what is true.

4.2 **Prefer immutable values.** When something can change, the reader has to track where, and
that cost grows with the size of the unit.

4.3 **Keep decisions pure and push side effects to the edges.** Logic without I/O can be tested
by calling it.

4.4 **Avoid global mutable state.** Hidden shared state makes behavior depend on call order and
on tests that ran earlier.

4.5 **Prefer explicit arguments over ambient context.** A unit that declares what it needs can be
understood from its signature.

## 5. Boundaries and dependencies

5.1 **Point dependencies inward.** The core should not know which library serves it, so the
library can be replaced without touching the logic.

5.2 **Wrap third-party types at the boundary.** When a library type spreads through the codebase,
replacing that library becomes a rewrite.

5.3 **Prefer composition over inheritance.** Composition states the relationship explicitly and
does not couple a type to the internals of its parent.

5.4 **Keep a dependency out until it earns its place.** Every dependency is a build surface, a
runtime surface, and something to audit.

## 6. Concurrency

6.1 **Prefer sequential code with clear ownership.** Add concurrency when a requirement or a
measurement calls for it, because concurrent bugs are the expensive kind.

6.2 **Give every invariant one owner.** Write down which lock protects what, and the order locks
are taken, so the rule can be checked by reading.

6.3 **Keep blocking work off async executors.** A blocked executor thread stalls unrelated work
and the symptom appears far from the cause.

6.4 **Never synchronize with a sleep.** Wait on the event you actually care about, otherwise the
code is timing dependent and fails on a slower machine.

## 7. Naming

7.1 **Name a thing for what it is or does.** A name that describes the implementation goes stale
the first time the implementation changes.

7.2 **Spell words out.** Use only abbreviations the domain already uses, because the reader
should not have to expand them.

7.3 **Name booleans as predicates.** A name that reads as a claim makes the condition at the call
site obvious.

7.4 **Use one term per concept across the codebase.** Alternating synonyms makes a reader wonder
whether two things are different.

7.5 **Treat "and" in a name as a warning.** A unit named for two things usually does two things.

## 8. Comments and documentation

8.1 **Comment why, not what.** The code already says what it does, and a comment repeating it
becomes wrong after the next edit.

8.2 **Write a comment only where the code cannot explain itself.** Unusual constraints, ordering
requirements, and workarounds deserve one. Ordinary code does not.

8.3 **Document invariants and preconditions on the unit that owns them.** The reader needs them
at the point where breaking them is possible.

8.4 **Give each module a one sentence doc comment stating its responsibility.** If the sentence
needs an "and", the module needs splitting.

8.5 **Update documents in the commit that changes the behavior.** A document corrected later is a
document that was wrong in between.

## 9. Tests

9.1 **Test behavior, not implementation.** A refactor that preserves behavior should leave the
tests green, otherwise the tests block the work they were meant to protect.

9.2 **Give each test one reason to fail, and a name that states the property.** A failing name
should tell you what broke before you read the body.

9.3 **Keep logic out of tests.** A loop or a branch in a test means the test needs its own tests,
and usually means the case belongs in a property test.

9.4 **Build test data with builders and name the values that carry meaning.** A wall of literals
hides which value the test is actually about.

9.5 **Hold tests to the same conventions as production code.** Test suites decay first, and a
decayed suite stops being evidence.

## 10. Changes and commits

10.1 **One logical change per commit.** A commit that does two things cannot be reviewed as one
thing or reverted as one thing.

10.2 **Keep refactors separate from behavior changes.** Mixing them hides the behavior change
inside the noise of moved code.

10.3 **Delete dead code.** Version control remembers it, and commented out code tells the next
reader nothing about whether it still works.

10.4 **Give every TODO an owner and a reference.** A TODO without one is a note that will never
be actioned.

10.5 **Fix causes, not symptoms.** When a workaround is unavoidable, record what blocks the real
fix so the next person can finish it.

10.6 **Keep opportunistic cleanup small.** Improve what you touch, and leave the rest for its own
commit so the review stays about one thing.

## 11. Performance

11.1 **Measure before optimizing.** Intuition about hot paths is wrong often enough that the
measurement is cheaper than the wasted work.

11.2 **Improve the algorithm before the constant factor.** Tuning a loop that runs too many times
buys little.

11.3 **Record the measurement next to the optimization.** Without it, the next reader cannot tell
whether the complexity is still earning its place.

11.4 **Set budgets for the paths that matter and leave the rest simple.** Optimizing code nobody
waits on adds risk for no return.

## 12. Untrusted input

12.1 **Treat everything from the network as hostile, including a paired peer.** A peer can be
compromised, and pairing proves identity rather than good behavior.

12.2 **Put explicit limits on every size, count, and duration a peer influences.** Without a
limit, a malformed message becomes memory exhaustion.

12.3 **Keep secrets and user content out of logs and error messages.** Logs are copied into bug
reports and read by people who should not see either.

## 13. Working habits

13.1 **Read before writing.** Search for a component that already does the job, or nearly does
it, because the second implementation is the one that causes drift.

13.2 **Verify an API exists before calling it.** A signature written from memory compiles in your
head and fails in the build.

13.3 **Reproduce a failure before fixing it.** A fix for a failure you never saw is a guess.

13.4 **Keep each change small enough to verify in one step.** Large changes hide which part broke
the tests.

13.5 **State assumptions instead of encoding them silently.** An assumption written down can be
corrected, while one buried in code is found by a user.

13.6 **Stop and ask when two readings of the spec lead to different code.** Guessing produces
plausible code, which is the hardest kind of wrong to catch in review.

13.7 **Do not expand scope.** Unrequested work still costs review, maintenance, and risk.

13.8 **Prefer deleting code to adding code when both solve the problem.** Deleted code needs no
tests and cannot break.

13.9 **Match the surrounding style.** Consistency across a codebase is worth more than any
individual preference.

## 14. Writing

14.1 **Use simple, natural English.** These documents are read by reviewers and by agents, and
complex sentences slow both of them down.

14.2 **Keep sentences short and coherent, and be concise.** A long sentence usually holds two
claims that should be separated.

14.3 **Use a contrast only to correct a wrong assumption.** Write "it is not X, but Y" when a
reader would otherwise assume X. If Y is already the obvious reading, state Y directly.

14.4 **Do not overuse parenthetical structures.** This covers both parentheses and asides after
em dashes. An aside interrupts the sentence a reader is holding in their head.

14.5 **Explain in coherent sentences rather than in asides.** If a point is worth making, it is
worth a sentence of its own.

14.6 **Write for a reviewer who reads once.** Anything that needs a second pass to parse should
be rewritten.

## When a rule does not fit

A rule that does not fit a situation is a decision to record, not a rule to ignore quietly. Say
why in the code comment or the commit message. Report the conflict when the same rule keeps
getting in the way, because that means the rule needs changing.
