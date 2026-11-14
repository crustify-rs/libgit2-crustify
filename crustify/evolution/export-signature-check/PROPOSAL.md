# How a gateway checks an export against its raw binding

Status: `open`: made as crustify `dfd557c` on `libgit2-crustify/evolution/export-signature-check`,
awaiting its review.

## Signal

A rule an agent had to work out by hand that a skill should state. `testing-gates`'
Regressions section asks of a `port` batch that "each export is checked at compile time
against the raw binding of the C declaration it replaces, so that a signature that drifts
from C's fails to build"; neither it nor `porting-symbols`' Functions section, which
describes the gateway, says how, and the crate held no export to follow.

## Evidence

`wave-229/batch-0/run-1` wrote the crate's first `ffi_export` gateway
(`src/libgit2/src/util/utf8.rs`, for `git_utf8_iterate`, `git_utf8_char_length` and
`git_utf8_valid_buf_length`). It allowlisted the three functions, which the bindings
lacked, and checks each export with

```rust
const _: () = {
    let _ = [sys::git_utf8_iterate, git_utf8_iterate];
    // ...
};
```

the array's two function items coercing to one function-pointer type only where their
signatures agree. Changed by hand to take `str_len: u32`, the export fails `cargo check
--features ffi-export-src-util-utf8-c` with `E0308` at that line (`expected usize, found
u32`); restored, `clippy --all-targets` passes with the feature on and off.

## Destination

The spec: `porting-symbols`' Functions section, which the translator's role evolves.

## Change

crustify `dfd557c`: one sentence after the panic rule, naming the constant, why it fails on
drift, and that the binding is allowlisted for it where the bindings lack it; and its
`EVOLUTION.md` entry.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`dfd557c`, which also edited `EVOLUTION.md`; on 2026-11-14 it was put on the working branch's
tip as `27f80ba`, the same diff without that edit, since a change's row is drafted in its
proposal. Review-1 read `27f80ba`; the commit is now `c59838e`.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| how a gateway checks an export against its raw binding | `libgit2-crustify`, `export-signature-check` | crustify `c59838e` | the first export of a port campaign had to work out how the compile-time check `testing-gates` asks for is written | a gateway whose exports have no signature check against their raw bindings, or one written another way per module | watched |

## Validation

The batch above, which followed the rule before it was written: the mutation fails to
build, the gateway as landed builds and lints.

## Review

Pending.

`review-1` (session `b787f0b3-9a40-4bda-bcf9-060b5f2d8989`, Opus 5.5, `$1.32`, 5m07s), on `27f80ba`: **approved**, with no fix required. It reran the validation: a drifted parameter type fails the build with E0308 at the check line. Not required: globals and variadic exports are not covered. On 2026-11-14 the commit was put second in the stack, on `144259c`, as `c59838e`, its diff unchanged (the same `git patch-id`).
Its report is the longest `result` of `crustify/artifacts/evolution/export-signature-check/review-1/stream.jsonl`.
