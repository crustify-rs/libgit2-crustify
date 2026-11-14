# ffimap does not count a wrapper that keeps its C function's name

Status: `open`. It was deferred on 2026-11-14 until the first wrapper that keeps its C function's
name; a review found that day that the campaign has two already, so the deferral is put back
to the user.

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

Not made. The item's name cannot stand for the alias on its own: a port's export has its C
function's name too, and is no wrapper. The driver would count a function whose name is a
`-sys` function's and whose doc comment carries the anchor that names it, as a fallback
where no alias does. The anchor's grammar would then live in two parsers, `api`'s and the
driver's: they share one fixture, so that a drift fails a test.

## Validation

None yet.

## Review

None yet.
