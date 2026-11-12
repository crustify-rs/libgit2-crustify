Filed by the assistant from e455870fc and 1a34b9caf, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# The `grealloc` trampoline rests on a zero-size rule only C's calls can keep

- **finding**: `contracts/ffi_import/realloc-trampoline-leaves-zero-size-to-c`
- **batch**: `wave-177/batch-1/run-1` of `libgit2/src/util/alloc.c`
- **item**: `fnp::git_allocator.grealloc`
- **affected commit**: not recorded: the report was refused by the host; filed from
  e455870fc and 1a34b9caf (the batch follows wave 177 batch 0, `fce21ef4e`)
- **source commits**: `e455870fc` (file), `1a34b9caf` (fix)

## Finding

`realloc_with`, the grealloc trampoline `Allocator::new` stores, passed C's arguments
straight to `Alloc::realloc`, justified by `ReallocFn`'s rule that size is non-zero when ptr
is not NULL; `Alloc`'s contract and its `C_ALLOCATOR` bullet rested on the same rule.
Libgit2, not a Rust caller, chooses those arguments once the table is installed, and C's
declaration (git2/sys/alloc.h) states no such rule: an obligation only C's calls can keep is
no contract (reviewing, Contracts, Crustify 57f14ff).

## Evidence

No libgit2 path makes the call today (`resize_vector` returns early on 0,
`reftable_realloc` frees on 0). `run-affected.log`, beside this report, shows what the guard
prevents: with it removed, the new ub_unsafe test
`realloc_member_refuses_a_zero_byte_resize_of_a_live_block` reports a heap-use-after-free
under ASan, glibc having released the block and returned NULL.

The exact command to rerun: not recorded: the report was refused by the host; filed from
e455870fc and 1a34b9caf.

## Reproducer

None: no libgit2 path makes the call today, so there is no reproducer. `run-affected.log`
and `run-fixed.log` sit beside this report.

## Fix

In `1a34b9caf`: the grealloc trampoline `Allocator::new` stores refuses a zero-byte resize
of a live allocation before `Alloc::realloc` is reached, returning NULL with the allocation
left live, C's failed-resize convention. `ReallocFn`'s contract says the trampolines relax
its zero-size rule, `Alloc`'s that the trampoline keeps it, and the `C_ALLOCATOR` bullet
asks a C family's realloc to answer that request without releasing the block and returning
NULL. Tests: lib's `zero_byte_resizes_of_live_blocks_are_refused` (Miri runs it) and
ub_unsafe's `realloc_member_refuses_a_zero_byte_resize_of_a_live_block`
(heap-use-after-free under ASan without the guard).
