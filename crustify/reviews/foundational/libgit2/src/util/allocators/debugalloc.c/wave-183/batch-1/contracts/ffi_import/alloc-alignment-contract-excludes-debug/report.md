Filed by the assistant from 801c9ddbd and 007596d40, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `Alloc`'s alignment contract asks for 16 where nothing needs more than 8, excluding debugalloc

- **finding**: `contracts/ffi_import/alloc-alignment-contract-excludes-debug`
- **batch**: `wave-183/batch-1/run-1`, the module pass of
  `libgit2/src/util/allocators/debugalloc.c`, over batch 0 (`fd3e6b8e1`)
- **item**: `fn::git_debugalloc_init_allocator`, bound as `Allocator::debug`
- **affected commit**: `fd3e6b8e1`
- **source commits**: `801c9ddbd` (file), `007596d40` (fix)

## Finding

`Allocator::debug`'s documentation said the family departs from `Alloc`'s contract, which
`set_allocator`'s `# Safety` requires, in its alignment: `Alloc` asked for malloc's
(max_align_t, 16), debugalloc gives a word's (8). So the value had no installation its own
documentation allowed. `Alloc`'s contract asked for 16 where nothing needs more than 8.

## Evidence

What the crate relies on is alloc.rs's `MAX_ALIGN`, max(align usize, align u64), which wave
12's review lowered to (LESSONS.md), and odb.rs's `DATA_ALIGN`; libgit2's C runs on
debugalloc in its CI.

The exact command to rerun: not recorded: the report was refused by the host; filed from
801c9ddbd and 007596d40.

## Reproducer

None: an obligation that asks more than the implementation needs.

## Fix

In `007596d40`: `Alloc`'s contract (alloc.rs) now asks for alignment to the largest power of
two not above len, up to `MAX_ALIGN` (the wider of usize's and u64's), which is what
libgit2's C and this crate's owners rely on, instead of max_align_t. Every implementation
meeting the old contract meets this one. With both of the batch's fixes `Allocator::debug`
meets `Alloc`'s contract, so installing it honours `set_allocator`'s `# Safety`, whose first
bullet now names it.
