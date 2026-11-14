# ffimap does not count a wrapper that keeps its C function's name

Status: `open`. No change is made yet.

Goal: Safety, the unsafe surface's measures: the share of C functions with a wrapper.

## Signal

A tool that reports what the source contradicts, once `doc-alias-equal-to-the-name` lands:
`conventions` then gives an item whose name is its C name no `doc(alias)`, and the driver's
`fn.ffibox.wrapper` counters read a wrapper from its alias alone (`docs/schema.md`,
"The annotation is the only record of what a wrapper wraps").

## Evidence

Read from the driver, `src/ffimap/driver/src/main.rs` at `9aa2ef1`: the counters loop over
`doc_aliases` of each function. No campaign has an affected function yet: glibc-crustify's
`stderr` is a global, and libgit2-crustify's helpers were renamed. The first wrapper of a C
function that keeps its name would go uncounted.

## Destination

A tool: ffimap's driver, with a fixture and a test.

## Change

Not made. The item's name cannot stand for the alias on its own: a port's export has its C
function's name too, and is no wrapper. The driver would count a function whose name is a
`-sys` function's and whose doc comment carries the anchor that names it.

## Validation

None yet.

## Review

None yet.
