Filed by the assistant from 7db163587 and 81dede2b1, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# The equivalence suite's C side reads reftable's hooks before anything pins libgit2's runtime

- **finding**: `contracts/ffi_import/equiv-c-side-races-the-first-init`
- **batch**: `wave-169/batch-5/run-1`, the module pass of `libgit2/deps/reftable/basics.c`,
  over wave 169's batches 0 to 4 as the wave tip `9aa6520a5` holds them
- **item**: `fn::reftable_buf_add`
- **affected commit**: `9aa6520a5`
- **source commits**: `7db163587` (file), `81dede2b1` (fix)

## Finding

Found by the wave's TSan gate on the tip. The C side of tests/equiv/deps/reftable/basics.rs
called `reftable_buf_add` (`RawBuf::new`, then `apply`'s `Step::Add`, and `pair`) before
anything pinned libgit2's runtime. `reftable_buf_add` reads reftable's allocator hooks
(`reftable_realloc`), and the process's first `git_libgit2_init` writes them
(`git_reftable_global_init` -> `reftable_set_alloc`), both without synchronisation. The
suite's other tests make that first init on the harness's other threads. The unsafe blocks'
"// SAFETY: `c` is a live buffer and `data` readable" leaves out the ordering, which the
safe API keeps by pinning first.

## Evidence

The gate's report:

```
WARNING: ThreadSanitizer: data race
  Write of size 8 by thread T1: reftable_set_alloc basics.c:76
    <- git_reftable_global_init refdb_reftable.c:1893 <- git_libgit2_init
    <- runtime_pinned errors.rs:559 <- ReftableStrFree::c_dup
    basics.rs:1318 <- equiv allocator_routines_match_c basics.rs:417
  Previous read of size 8 by thread T4: reftable_realloc basics.c:34
    <- reftable_alloc_grow basics.h:229 <- reftable_buf_add basics.c:127
    <- equiv apply basics.rs:175 <- buffer_routines_match_c_step_by_step
```

TSan then halted the suite, so the rest of equiv never ran under TSan.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. The reproducer replays the suite's C side
on one thread while another makes the first safe allocation. Run from its own directory,
with W the repository root:

```
CARGO_TARGET_DIR=$W/crustify/tmp/repro-tsan CRUSTIFY_BUILD_VARIANT=tsan \
RUSTFLAGS=-Zsanitizer=thread \
TSAN_OPTIONS="halt_on_error=1:suppressions=$W/crustify/scripts/tsan.supp" \
  cargo +nightly-2026-10-08 run -Zbuild-std --target x86_64-unknown-linux-gnu
```

```
WARNING: ThreadSanitizer: data race
  Write of size 8 by thread T1: reftable_set_alloc basics.c:76 <- ...
    <- runtime_pinned errors.rs:559
  Previous read of size 8 by main thread: reftable_realloc basics.c:34
    <- reftable_alloc_grow basics.h:229 <- reftable_buf_add basics.c:127
    <- main main.rs:35
SUMMARY: ThreadSanitizer: data race basics.c:76 in reftable_set_alloc
(exit 66)
```

It replays the old sequence, so it keeps racing on any tree: on a later tree the verdict is
the equiv suite under TSan.

## Fix

In `81dede2b1`: the equivalence suite's C side pins libgit2's runtime before its first call
that reads reftable's hooks: `pin_runtime()`, called by `RawBuf::new` and `c_parse_names`,
makes a first safe allocation, after which `OnceLock` orders the thread's reads after the
init's write; `allocator_routines_match_c` uses it in place of its inline pin. The SAFETY
comments of the raw add, release and parse_names calls state the ordering, and the suite's
doc says why. All of equiv (506 tests) now runs clean under TSan, which the race had halted.

Proposal filed with `7db163587`: crustify/evolution/equiv-c-side-meets-the-c-preconditions/:
the testing skill's Equivalence tests section gains the rule this finding needed.
