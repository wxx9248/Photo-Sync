# Kotlin conventions

Idioms for the Android app. The general rules in `CONVENTIONS.md` apply here too, and this
document only covers what the language and the platform decide for us. Lint configuration lives
in the ktlint and Android Lint setup.

## Types

K1 **Use value classes for domain identifiers.** A `DevicePath` costs nothing at runtime and
cannot be passed where a display name is expected. See 2.4.

K2 **Model closed sets as sealed interfaces and match with an exhaustive `when`.** A new case
then becomes a compile error rather than a silent fallthrough.

K3 **Use data classes for values, not for entities with identity.** Generated equality compares
fields, which is wrong for anything identified by a key.

K4 **Declare `val` by default and expose read-only collection types.** A caller that cannot
mutate your state cannot surprise you.

K5 **Avoid `!!` and platform types crossing into your own code.** Convert at the boundary where
the platform hands you a nullable value. See 2.2.

## Errors

K6 **Return a sealed result type for expected failures.** Exceptions are for bugs, and a typed
result forces the caller to consider the failure.

K7 **Never catch a broad exception type without handling or rethrowing it.** A swallowed
exception turns a failure into wrong behavior later. See 3.2.

K8 **Let coroutine cancellation propagate.** Catching `CancellationException` breaks structured
concurrency and leaves work running after its scope ends.

## Coroutines

K9 **Pass a `CoroutineScope` in and never use `GlobalScope`.** Work that outlives its owner is
work nobody cancels.

K10 **Inject dispatchers.** A hardcoded dispatcher cannot be replaced by a test dispatcher, and
the test then depends on real time. See 6.4.

K11 **Make suspending functions main-safe by choosing the dispatcher inside them.** The caller
should not have to know where the work belongs.

## Structure

K12 **Declare `internal` by default inside a module.** Public is a commitment to other modules.
See 2.8.

K13 **Keep Android framework types out of the session state machine.** Logic free of the
framework runs in a plain JVM test, which is the fast half of the harness.

K14 **Use extension functions to adapt types you do not own.** Hiding domain logic in an
extension makes it hard to find.

K15 **Prefer constructor injection through the container over service location.** A dependency in
the signature is visible. See 4.5.

## Compose

K16 **Hoist state and keep composables stateless where possible.** A composable that only renders
its parameters can be previewed and tested directly.

K17 **Keep side effects in the effect APIs.** Work started during composition runs again on every
recomposition.

K18 **Expose one immutable UI state object per screen.** Several independent state holders let
the screen render a combination that never occurs.

## Tests

K19 **Never use `Thread.sleep` in a test.** Use a test dispatcher and virtual time so the test
waits on the event rather than on the clock.

K20 **Name a test for the property it checks.** The name is what a failure report shows first.
