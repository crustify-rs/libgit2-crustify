Filed by the assistant from 6a6276fc3 and 3dbb73d6d, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# ffibox's run constructors pass a size past `isize::MAX` to libgit2's allocator

- **finding**: `misc/oversized-ffibox-runs-reach-libgit2`
- **batch**: `wave-12/batch-3/run-1`, the review of wave 12 batches 1 (`c0c753430`) and 2
  (`1dc9911d7`), alloc.c's seven strategy items, as batch 2 left them
- **items** (the fix commit's): `fn::git__malloc`, `fn::git__calloc`, `fn::git__mallocarray`
- **affected commit**: `1dc9911d7`
- **source commits**: `6a6276fc3` (file), `3dbb73d6d` (fix)

## Finding

The module docs say a run past isize::MAX bytes "is refused before C is asked", and that
ffibox's own run constructors "ask libgit2 nothing". Only `ArrayFree::new_uninit`/`new_zeroed`
check it; `AllocVec::new_uninit` and `from_fn` reach `ArrayFree::c_alloc_len`, which passes
2^63 to `git__malloc`.

## Evidence

The reproducer installs a recording `Alloc` through `set_allocator` and exits 101 on
`1dc9911d7`: "AllocVec::new_uninit: libgit2's allocator was asked for 9223372036854775808
bytes" (`run-affected.log`, beside this report).

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `1dc9911d7`. The exact
command: not recorded: the report was refused by the host; filed from 6a6276fc3 and
3dbb73d6d.

## Fix

In `3dbb73d6d`: `ArrayFree::c_alloc_len` refuses a size past isize::MAX with
`git_error_set_oom` before asking C, as `alloc_run` does, so ffibox's `AllocVec::new_uninit`
and `from_fn` make the module's one size check too; the module docs say so. The unit test
`overflowing_sizes_report_out_of_memory_unasked` and ub_safe's
`vec_rejects_oversized_requests_without_panicking` cover both constructors; the reproducer
exits 0 (`run-fixed.log`).

Revisit (`e455870fc`, wave-177/batch-1): the reproducer rerun on that tree still does not
reproduce: it exits 0 (each constructor refused before libgit2 is asked).
