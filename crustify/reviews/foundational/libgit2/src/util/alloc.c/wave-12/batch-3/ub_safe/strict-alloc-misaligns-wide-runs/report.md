Filed by the assistant from 6a6276fc3 and 3dbb73d6d, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# Under `DEBUG_STRICT_ALLOC`, safe code forms a misaligned `&[u128]` over libgit2's storage

- **finding**: `ub_safe/strict-alloc-misaligns-wide-runs`
- **batch**: `wave-12/batch-3/run-1`, the review of wave 12 batches 1 (`c0c753430`) and 2
  (`1dc9911d7`), alloc.c's seven strategy items, as batch 2 left them
- **items** (the fix commit's): `fn::git__malloc`, `fn::git__calloc`, `fn::git__mallocarray`
- **affected commit**: `1dc9911d7`
- **source commits**: `6a6276fc3` (file), `3dbb73d6d` (fix)

## Finding

alloc.rs's `MAX_ALIGN` is `align_of::<max_align_t>()` (16), which `ArrayFree` vouches to
ffibox as `CLenAlloc`/`CLenClone::ALIGN` and which bounds `Free`'s `CAlloc`/`CAllocZeroed`
and `ArrayFree::alloc_run`. libgit2 configured with `-DDEBUG_STRICT_ALLOC=ON` (its own CI's
configuration) runs on debugalloc, which returns `malloc(len + 8) + 8`: 8-aligned storage.
Safe code then forms a misaligned `&[u128]`.

## Evidence

odb.rs's `DATA_ALIGN` already used max(align usize, align u64).

On the affected commit the reproducer aborts (exit 134): addresses `0x...c58` / `0x...c28`,
"unsafe precondition(s) violated: slice::from_raw_parts requires the pointer to be aligned"
(`run-affected.log`, beside this report). On the plain variant it passes, 16-aligned
(`run-affected-default-alloc.log`).

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `1dc9911d7`.

Rerun: build libgit2 privately with the plain variant's configure plus
`-DDEBUG_STRICT_ALLOC=ON`, then `cmake --build .` and
`static-archive.sh libgit2 crustify/libgit2.a`; from the reproducer dir:

```
CRUSTIFY_BUILD_VARIANT=plain CRUSTIFY_BUILD_DIR=<build> cargo run
```

## Fix

In `3dbb73d6d`: `MAX_ALIGN`, which `ArrayFree` vouches to ffibox and which bounds `Free` and
`ArrayFree::alloc_run`, is now the wider of usize's and u64's alignment (8 on x86-64), as
odb.rs's `DATA_ALIGN` is, not max_align_t's (16). libgit2 configured with
`DEBUG_STRICT_ALLOC` runs on debugalloc, which returns malloc's block offset by a size_t. A
16-aligned element or layout type is refused at compile time: compile-fail doctests for
`AllocVec::<u128>::from_slice`, `from_fn`, `ArrayFree::new_zeroed::<u128>` and an align(16)
`AllocBox`. The reproducer now fails to build with E0080 ("cannot align T"), the regression
form (`run-fixed.log`). ub_safe's `vec_storage_aligns_wide_elements` takes an i64 run in
place of its u128 one.

`3dbb73d6d` also records the alignment rule in LESSONS.md.

Revisit (`e455870fc`, wave-177/batch-1): the reproducer rerun on that tree still does not
reproduce: it fails to compile with E0080, as `run-fixed.log` records.
