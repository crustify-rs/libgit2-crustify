# A macro whose configured branches compute one result is ported once

Status: `open`.

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

## Validation

`wave-225/batch-0`'s tests: its unit tests check every NUL position in every bound up to 70
bytes against the standard library's scan, and its equivalence tests compare the port with
the C's expansion, glibc's `strnlen`, through `git__strndup`. The `NO_STRNLEN` branch is
not built on this host; its body is the same scan by reading.
