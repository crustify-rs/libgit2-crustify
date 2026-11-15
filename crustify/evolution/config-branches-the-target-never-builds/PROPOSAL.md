# A macro's branches for compilers and platforms the crate never targets do not shim it

Status: `open`.

Goal: Idiomatic Rust, and Least constraint: a shim over a branch the crate cannot be built
in constrains the port without a reason. Fitness: the rules a run worked around, per module
(Least constraint's measure), here one task message's override.

## Signal

A rule an agent works around, met as the recurrence signal of the record's row
`config-branches-of-one-result` (crustify `769d102`, `watched`): "a task message ... that
settles again whether a configuration-dependent macro is ported or shimmed".

## Evidence

`wave-223/batch-0/run-1` ported `src/util/integer.h`'s six overflow macros. The header
chooses each by compiler: clang's and gcc's `__builtin_*_overflow` intrinsics, Microsoft's
`<intsafe.h>` functions under `_MSC_VER`, and `GIT_INLINE` fallbacks for a compiler with
neither; `git__multiply_int64_overflow` also takes its fallback under clang on a 32-bit
target (`GIT_ARCH_32`). Five of the six compute one result in every branch, as the record's
row has it. The sixth does not: the fallback of `git__multiply_int64_overflow` reports
`INT64_MIN * -1` and `-1 * INT64_MIN` as fitting and `INT64_MIN * 1` and `1 * INT64_MIN` as
overflowing, the opposite of the intrinsic, and `tests/util/integer.c`'s edge-case test
asserts the fallback's answers (its body compiles empty on every variant, which defines the
macro as the intrinsic).

`porting-symbols`' Macros section, as `config-branches-of-one-result` landed it, says: "Only
one whose branches differ in what they compute keeps the shim `binding-symbols` gives it".
Read literally, `git__multiply_int64_overflow` would be bound through
`crustify_git__multiply_int64_overflow` rather than ported, though no build of the crate can
take the branch that differs: the bindings are generated for the host target alone and
`src/sys.rs` fails to compile on any other (`bindgen`'s Generation section), every variant
compiles the C with clang for x86-64 (`crustify/artifacts/build.json`), and the bundled build
uses the `plain` variant's configuration. The task message had to settle it again: "Where the
header has several configured branches, port what this build defines, as porting-symbols'
Macros section and strnlen_h.rs do." The batch ported `checked_mul_i64` over `i64::checked_mul`
and recorded the fallback's four inputs in the module's docs and in a carried-over unit test.

The section does not say which configurations its exception means. A configuration the crate
can be built in, a build option such as the SHA-1 backend or threads, which a dependent's
bundled build could select, is one a port must answer for; a compiler or a platform the crate
cannot be built for is not.

## Destination

The spec: `porting-symbols`' Macros section.

## Change

Replace the second paragraph's first two sentences with:

> A macro whose expansion depends on the build's configuration is ported once, from the
> branch the crate's builds take, where every configuration the crate can be built in
> computes the same result for every input: a branch only a compiler or a platform the crate
> is not built for takes is no such configuration. Its documentation names the branches the
> build does not take, and where one of them computes another result, the inputs on which
> it does. Only one whose branches differ in what they compute among the configurations the
> crate can be built in, such as a build option a bundled build may select, keeps the shim
> `binding-symbols` gives it: a port reimplements the body, and a shim over the C's
> expansion would make it a binding.

## Validation

Not yet validated: the rerun is the item that exposed it, `mac::git__multiply_int64_overflow`
of `wave-223/batch-0`, under the change and without the task message's sentence, expected to
port it as this run did.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a configuration-dependent macro is ported from the branch the crate's builds take, where every configuration it can be built in computes one result; a branch only an untargeted compiler or platform takes holds no shim, and its differing inputs are documented | `libgit2-crustify`, `config-branches-the-target-never-builds` | — | the landed rule's shim exception, read literally, bound a macro whose only differing branch no build of the crate can take; a task message had to settle it again | a task message or plan correction that settles again whether a configuration-dependent macro is ported or shimmed | — |

## Review

None yet.
