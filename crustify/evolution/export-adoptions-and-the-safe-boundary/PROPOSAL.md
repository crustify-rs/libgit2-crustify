# What an `ffi_import` region may expose to its module's `ffi_export`

Status: `open`.

## Signal

A rule an agent had to work out by hand that a skill should have stated: two rules read
against each other.

## Evidence

`wave-186/batch-4/run-1` (`git2/src/cli/opt.c`) wrote the module's first exports while its
structs stay wrapped. `cli_opt_status_fprint`'s gateway receives an outcome whose spec C
reads forward to the end of its choice, so it must adopt C's spec table, a run ended by a
zeroed spec, which only `ffi_import`'s private `SpecTable::from_c` builds. `binding`'s Safe
boundary section lets a region expose "no `unsafe fn` beyond the public ones whose obligation
no type expresses", so a `pub(super) unsafe fn` for the gateway is excluded, while
`binding-symbols`' Function pointers section has the C half of a callable type made "only
inside the crate, through an `unsafe` constructor", a crate-private `unsafe fn`, which this
batch also writes (`ForeachCallbackFn::from_c`, for batch 5's `cli_opt_foreach` export).

The batch read the general rule as binding: `SpecTable::from_ptr` is a public `unsafe fn`, an
adoption beside ffibox's `CRefSeam::from_ptr`, with its `# Safety` and its `ub_unsafe` suite,
and the callable's constructor stays `pub(crate)`, as the specific rule asks. Every later
export of a module whose types keep their C layout meets the same choice: batch 5's
`cli_opt_parse` and `cli_opt_foreach` adopt the same tables.

## Destination

`binding`'s Safe boundary section (`rust/binding`), which the translator does not evolve, or
`porting`'s FFI boundary section, which it does, as the place a port's exports are described.

## Change

Not made, pending which reading is meant. Proposed: a region may expose to its module's other
region, `ffi_export`, the `unsafe` adoptions its gateway needs, crate-private, each with the
`# Safety` a public one would carry; or, if every adoption is to be public, the Function
pointers section says its constructor is public too.

## Validation

None: the batch's code follows the stricter reading and passes its gates either way.

## Review

None yet.
