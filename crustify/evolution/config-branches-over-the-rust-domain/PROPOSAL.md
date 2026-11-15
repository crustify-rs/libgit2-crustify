# A macro's configured branches are compared over the arguments its Rust form takes

Status: `open`.

Goal: Idiomatic Rust, and Least constraint: the shim the rule keeps for branches that differ
would bind libc where the Rust form can state the one result its callers receive.

## Signal

A landed row's recurrence signal: `config-branches-of-one-result` (crustify `769d102`),
"a task message or plan correction that settles again whether a configuration-dependent
macro is ported or shimmed". `wave-220/batch-0/run-1`'s task message settled it again:
"Where the header has several configured branches, port what this build defines, as
`porting-symbols`' Macros section and `strnlen_h.rs` do."

## Evidence

`wave-220/batch-0/run-1` ported `src/util/ctype_compat.h`'s eight macros, `git__isalnum`,
`git__isalpha`, `git__isdigit`, `git__isprint`, `git__isspace`, `git__isxdigit`,
`git__tolower` and `git__toupper`. The header has two branches:

- this build's: `# define git__tolower(a) tolower((unsigned char)(a))`, and
  `(!!isalpha((unsigned char)(a)))` for each class, libc's `<ctype.h>`; and
- `GIT_WIN32`'s: `GIT_INLINE` functions comparing the `int` with ASCII ranges,
  `return (c >= 'A' && c <= 'Z') ? (c + 32) : c;`.

`porting-symbols`' Macros section ports such a macro once "where every configuration's
expansion computes the same result for every input", and keeps the shim for one "whose
branches differ in what they compute". Read over the C's inputs, the `int` each macro
takes, these branches differ: `git__tolower(-1)` is `-1` under `GIT_WIN32` and `255`
elsewhere, `git__tolower(0x141)` is `0x141` and `'a'`. Read over the byte the expansion's own
cast leaves, the argument every other build classes, they agree for each of the 256: the
run checked it in glibc's `C`, `POSIX` and `C.UTF-8` locales against the `GIT_WIN32`
comparisons (`branches.c` beside this proposal, run as `cc branches.c && ./a.out <locale>`: 0 bytes differing in each),
and the batch's equivalence tests compare the ports with glibc's functions for every `int`
from -300 to 600. The Rust form takes that byte, a `u8`, and its C callers' cast is
written where they call it, `c as u8`. Which reading the rule means, the rule does not say,
so the task message had to.

The process locale, which `<ctype.h>` reads and the Rust does not, is a second difference the
rule's "configuration" leaves open: it is no build configuration, but the state of an
out-of-tree library's entity, which `dependencies.json` already records in the entity's
`match` and note ("libgit2 runs in the C locale, where glibc's tables agree") and which its
route's confirmation checks.

## Destination

The spec: `porting-symbols`' Macros section.

## Change

In its second paragraph, replace "is ported once where every configuration's expansion
computes the same result for every input, and its documentation names the branches the
build does not take" with:

> is ported once where every configuration's expansion computes the same result for every
> input its Rust form takes: the arguments the expansion's own conversions leave, such as a
> cast to `unsigned char`, which its Rust callers make where they call it. Its documentation
> names the branches the build does not take, and the arguments outside that domain on
> which they part. The runtime state an out-of-tree entity of the expansion reads, such as
> the locale `<ctype.h>` follows, is no configuration: the entity's route and its
> confirmation settle it, as `dependencies`' Confirmation section describes.

## Validation

`wave-220/batch-0`'s port and tests, as the evidence describes: 11 `unit_pub` tests check
every byte against the C standard's C-locale classes, and 5 `equiv` tests compare the ports
with glibc's `<ctype.h>` for every `int` in -300..=600 and through three of the C's
expansion sites. A mutant `is_space` without the vertical tab fails three of them.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a configuration-dependent macro's branches are compared over the arguments its Rust form takes, after the expansion's own conversions; an out-of-tree entity's runtime state, such as the locale, is its route's, not a configuration | `libgit2-crustify`, `config-branches-over-the-rust-domain` | — | `config-branches-of-one-result`'s recurrence: a task message settled again whether `ctype_compat.h`'s macros, whose branches part only on `int`s outside a byte, are ported or shimmed | a task message or plan correction that settles again whether a configuration-dependent macro is ported or shimmed, or a review finding of a shim over branches that agree on the Rust form's domain | — |

## Review

None yet.
