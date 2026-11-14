# ffimap does not count a wrapper that keeps its C function's name

Status: `deferred`, on 2026-11-14, at the user's word: it waits for `ffimap-sys-inventory-in-crate`,
without which the counters see none of this campaign's C functions. The change so far is
ffimap `2a2aa1e`, kept on `libgit2-crustify/evolution/ffimap-wrapper-without-alias`, rejected
by review-1 with three fixes still to make (2 to 4 below).

Goal: Safety, the unsafe surface's measures: the share of C functions with a wrapper.

## Signal

A tool that reports what the source contradicts, once `doc-alias-equal-to-the-name` lands:
`conventions` then gives an item whose name is its C name no `doc(alias)`, and the driver's
`fn.ffibox.wrapper` counters read a wrapper from its alias alone (`docs/schema.md`,
"The annotation is the only record of what a wrapper wraps").

## Evidence

Read from the driver, `src/ffimap/driver/src/main.rs` at `9aa2ef1`: the counters loop over
`doc_aliases` of each function. This campaign has two affected functions: `binsearch` and `parse_names` in
`src/libgit2/deps/reftable/basics.rs` (`wave-169/batch-2`, `2032628fd`), each `Wraps
fn::<name>` with no alias, and both declared by the raw bindings. Whether the counters read
low by them depends on `--sys-crates` reaching bindings that are a module of the crate, which
is not yet run.

## Destination

A tool: ffimap's driver, with a fixture and a test.

## Change

In the driver, `count_sys_types`: a function whose name is a `-sys` function's, with no alias
naming it, counts as that function's wrapper where its documentation carries its anchor, a
doc line of its own that is exactly ``Wraps `fn::<name>` `` or ``Replaces `fn::<name>` ``
(`anchors_fn`). The name alone does not count: a port's export has its C function's name too.
`docs/schema.md` says so. The `cells` fixture gains one such wrapper, `kind_of`, and two
functions named as C functions that are none: one with no anchor, one whose lines only hold
an anchor, in a sentence and with a trailing period, the forms `ffimap api` reports as
malformed.

The anchor's form now lives in two parsers, `api.py`'s `anchors` and the driver's
`anchors_fn`. Each has a fixture with the well-formed and the malformed forms, so a drift in
either fails its test; one fixture for both is not made.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `fn.ffibox.wrapper` counts a function that keeps its C function's name, and so carries no alias, by its anchor | `libgit2-crustify`, `ffimap-wrapper-without-alias` | ffimap `2a2aa1e` | once an item named as its C item carries no alias, the counters, which read aliases, missed two landed wrappers | test | |

## Validation

- ffimap's suite: 145 tests pass. `test_the_sys_fns_and_the_wrappers_annotated_for_them` expects
  five wrappers of four functions, where the old driver counts four of three.
- On real code, glibc-crustify at `0c718fb`, `--ffibox --sys-crates libc_sys`: the old and the
  new driver give the same 740 values but the scanner's version (`fn.ffibox.wrapper.total` 2 of
  `fn.ffibox.sys.total` 23). It has no wrapper named as its C function, so nothing moves.
- Not run on libgit2-crustify: its raw bindings are a module of the crate, not a `-sys` crate
  `--sys-crates` can name, so whether its two wrappers, `binsearch` and `parse_names`, enter
  the count at all is not shown here.

## Review

`review-1` (Opus 5.5, `$2.65`), on ffimap `2a2aa1e`: **rejected**, four fixes required; it weakens no
check. It ran the suite (145 pass; the changed test fails on the old driver, `4 != 5`) and
probed 19 forms of the anchor against `api.py`'s `anchors`.

1. The change moves nothing on any real crate today. The driver takes its `-sys` inventory from
   extern crates alone, and libgit2-crustify's raw bindings are `crate::sys`, a module of the
   crate: neither driver can count `binsearch` or `parse_names`. The row's "missed two landed
   wrappers" overstates, and the signal that `--sys-crates` cannot name a module of the scanned
   crate was unfiled: it is now `ffimap-sys-inventory-in-crate`.
2. The row's `test` is wrong: the fixture checks `Wraps` on a free function only, and dropping
   `Replaces` still passes.
3. The driver does not read the form `ffimap api` reads for a block doc comment, whose ` * `
   prefix rustdoc strips and the driver keeps.
4. The driver now keys on Crustify's anchor grammar, a departure from ffimap's G8 that
   `goals.md` records for `api` alone, and G10 asks the commit to say what moved on `examples/`.

The user's "fix now" rested on the count reading low by two, which fix 1 shows it does not.
The change is not landed, and whether to finish it is put back to the user.
