# A macro whose configured branches compute one result is ported once

Status: `landed` as crustify `769d102` (record `36f9c7f`) on `libgit2-crustify/devel`, on 2026-11-14, once its
review approved it.

Goal: Idiomatic Rust, and Least constraint: a shim where the Rust can state the result is a
constraint without a reason.

## Signal

A rule an agent had to work out by hand that a skill should have stated.

## Evidence

`wave-225/batch-0/run-1` ported `mac::p_strnlen` (`src/util/strnlen.h`). The header defines
it by platform: `#define p_strnlen strnlen` (libc's), or, under `NO_STRNLEN` (MinGW, Solaris,
Apple, MidnightBSD, old MSVC), a `GIT_INLINE` body over `memchr(s, 0, maxlen)`. Both give
the offset of the first NUL within `maxlen`, or `maxlen`, for every input.

`binding-symbols`' Macros section reaches "a macro whose expansion depends on the build's
configuration" through a shim, `crustify_<NAME>`, and binds a symbol alias "through the safe
wrapper of the symbol it names". `porting-symbols`' Macros section defers to it. Read
literally, the port would be a shim over libc's `strnlen`, or a wrapper of it, though
`dependencies.json` gives libc's `strnlen` the `std` route and nothing of its state is
shared with C. The task message had to settle it: "Port what this build defines, and say in
its docs what the other branch is." The batch ported a native `const fn strnlen(&[u8])`, by
`CStr::from_bytes_until_nul`, and documented the `NO_STRNLEN` branch on it.

The module pass, `wave-225/batch-1/run-1`, judged the port against `porting-symbols`'
Macros section and filed no finding: the body computes on its arguments alone, so it takes
the `const` function form, and libc's `strnlen` is reached by its `std` route. The reading
the shim exception invites is unclear twice over: what selects the branch here is the
target platform (`__APPLE__`, `__sun`, `__MINGW32__`, `__MidnightBSD__`, `_MSC_VER`), which
Rust's own `cfg(target_os, target_env)` does see, and a port reimplements the body instead
of reaching the C's expansion, so a shim over the C would make the port a binding. The
section should say which configurations the exception means, and that it applies only
where the branches differ in what they compute.

## Destination

The spec: `porting-symbols`' Macros section.

## Change

Add, after its first paragraph:

> A macro whose expansion depends on the build's configuration is ported once when every
> configuration's expansion computes the same result for every input, its documentation
> naming the branches the build does not take; only one whose branches differ in what they
> compute keeps `binding-symbols`' shim. A symbol alias of an out-of-tree library's entity
> is ported as that entity's route reaches it, as `dependencies`' Confirmation section
> checks it: through the standard library, it is a native function.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro whose configured branches compute one result for every input is ported once, its documentation naming the branches not taken; only one whose branches differ keeps the shim | `libgit2-crustify`, `config-branches-of-one-result` | crustify `769d102` | the shim exception, read literally, made the port of a platform-selected macro a binding of libc; a task message had to settle it | a task message or plan correction that settles again whether a configuration-dependent macro is ported or shimmed, or a review finding of a shim over branches that compute the same | watched |

## Validation

`wave-225/batch-0`'s tests: its unit tests check every NUL position in every bound up to 70
bytes against the standard library's scan, and its equivalence tests compare the port with
the C's expansion, glibc's `strnlen`, through `git__strndup`. The `NO_STRNLEN` branch is
not built on this host; its body is the same scan by reading.

## Review

`review-1` (Opus 5.5, `$1.07`), on `769d102`: **approved**, no fix required; it weakens no check. The built
configuration's equivalence tests still compare the port with the C's expansion; on a
configuration the campaign never builds, that the branches agree is a judgement made by
reading, the porter's, which the module pass reads. Listed apart: the rule does not say which
kinds of configuration the shim exception means; its reason clause sits beside the wrong
half of the sentence.
