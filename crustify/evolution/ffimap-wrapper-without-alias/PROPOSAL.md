# ffimap does not count a wrapper that keeps its C function's name

Status: `open`: the change is ffimap `2a2aa1e` on `libgit2-crustify/evolution/ffimap-wrapper-without-alias`,
made on 2026-11-14 at the user's word ("fix now"), awaiting its review.

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

None yet.
