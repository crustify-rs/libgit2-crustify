# The standard library's own macro expansions count as unsafe code outside the regions

Status: `open`.

## Signal

A rule agents work around, at a cost to the code (`evolution` §2). The conventions' Unsafe
regions section counts "every invocation of another crate's macro that expands to" an
`unsafe` block as unsafe code, so native code may not invoke one, and ffimap's
`code.unsafe_block.of_which.outside_regions` reports such an expansion. That includes the
standard library's `format_args!` (behind `write!`, `writeln!` and `format!`) and
`thread_local!`, whose unsafety is std's own, sound for every input, and which no
convention asks a crate to justify.

## Evidence

Three workarounds in one module:

- `wave-194/batch-0/run-1`: the four `Unnamed*` errors' `Display` used `write!`; the first
  scan found four unsafe blocks outside the regions, and the impls were rewritten to write
  their parts one by one (dashboard Notes, batch 0).
- `wave-194/batch-1/run-1`: a `thread_local!` in native code was moved into `ffi_import`
  for the same count, though it touches no C.
- `wave-194/batch-3/run-1`: `ParserRef::debug_trace`'s line, built with `writeln!` from
  `{:p}`, `{}` and `{:02x}` arguments, gave two blocks outside the regions
  (`src/libgit2/deps/llhttp/api.rs`, the boundary scan's `--sites
  code.unsafe_block.of_which.outside_regions`); it was rewritten without `format_args!`, a
  hand-written hexadecimal writer (`push_hex`) in place of `{:x}`.

## Destination

The spec, the conventions' Unsafe regions section (not the translator's: proposed), and
ffimap, whose count follows it: an option naming the crates whose macro expansions are
trusted (`--trusted-macro-crates core,alloc,std`), their blocks reported under a share of
their own rather than `outside_regions`.

## Change

Not made. In the conventions' § 2.1, after "such as one that mints a type over a C layout":
"The standard library's own macros, `format_args!` and those built on it, and
`thread_local!`, are not unsafe code of the crate: their expansions are std's, sound for
every input."

## Validation

None run. The rerun that would validate it: scan `wave-194/batch-3`'s tree before its
`push_hex` rewrite with the option, and see the two sites leave `outside_regions`.

## Review

None yet.
