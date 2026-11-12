Filed by the assistant from 6a6276fc3 and 3dbb73d6d, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# The module docs still say `git_stdalloc_init_allocator` "is compiled hidden"

- **finding**: `misc/stale-hidden-allocator-init-docs`
- **batch**: `wave-12/batch-3/run-1`, the review of wave 12 batches 1 (`c0c753430`) and 2
  (`1dc9911d7`), alloc.c's seven strategy items, as batch 2 left them
- **items** (the fix commit's): `fn::git__malloc`, `fn::git__calloc`, `fn::git__mallocarray`
- **affected commit**: `1dc9911d7`
- **source commits**: `6a6276fc3` (file), `3dbb73d6d` (fix)

## Finding

Batch 0's module docs still say `git_stdalloc_init_allocator` "is compiled hidden".

## Evidence

Since `d21c78c70` hidden functions link, and schedule.json plans it in
src/util/allocators/stdalloc.c's wave 181. No Rust stand-in of stdalloc exists.

The exact command to rerun: not recorded: the report was refused by the host; filed from
6a6276fc3 and 3dbb73d6d.

## Reproducer

None: docs only.

## Fix

In `3dbb73d6d`: the module docs now say `git_stdalloc_init_allocator` is
src/util/allocators/stdalloc.c's, wrapped with that module, rather than "compiled hidden".
