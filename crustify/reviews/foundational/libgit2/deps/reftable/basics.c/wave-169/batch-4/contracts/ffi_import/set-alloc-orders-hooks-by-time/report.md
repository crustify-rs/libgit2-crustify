Filed by the assistant from d04070734 and 9aa6520a5, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `set_alloc`'s contract orders the hooks' change by time, not by happens-before

- **finding**: `contracts/ffi_import/set-alloc-orders-hooks-by-time`
- **batch**: `wave-169/batch-4/run-1` of `libgit2/deps/reftable/basics.c`
- **item**: `fn::reftable_set_alloc`
- **affected commit**: `bbaafcd09`
- **source commits**: `d04070734` (file), `9aa6520a5` (fix)

## Finding

`set_alloc`'s first `# Safety` bullet asked that no other thread allocate "during the call".
That window is measured in time, but a race is decided by happens-before:
`reftable_set_alloc` stores the hooks and `reftable_malloc` loads them, both without
synchronisation. `set_default_alloc`'s contract ("As set_alloc") had the same gap.

## Evidence

The reproducer honours all three bullets as written. Its worker allocates 500 ms after the
call, no allocation is live when the hooks change, and the runtime is kept. Under the tsan
variant it reports a data race and exits 66:

```
WARNING: ThreadSanitizer: data race
  Read of size 8 by thread T1: reftable_malloc basics.c:22 <- reftable_strdup
    basics.c:65 <- ReftableStrFree::c_dup basics.rs:1285 <- main.rs:52
  Previous write of size 8 by main thread: reftable_set_alloc basics.c:75
    <- set_alloc::<Libc> basics.rs:1500 <- main.rs:58
```

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. From the reproducer's directory (`$W` is
not defined in `d04070734`; `7db163587`, the next batch's file commit, uses W for the
repository root):

```
CARGO_TARGET_DIR=$W/crustify/tmp/repro-tsan CRUSTIFY_BUILD_VARIANT=tsan \
RUSTFLAGS=-Zsanitizer=thread \
TSAN_OPTIONS="halt_on_error=1:suppressions=$W/crustify/scripts/tsan.supp" \
  cargo +nightly-2026-10-08 run -Zbuild-std --target x86_64-unknown-linux-gnu
```

## Fix

In `9aa6520a5`: `set_alloc`'s first `# Safety` bullet now asks that every use of reftable's
allocator on another thread be ordered before or after the call by synchronisation (a join,
a lock, a channel), and not merely run at another time. The module docs and the SAFETY
comments of `set_alloc` and `set_default_alloc` say the same. The UB regression test is
ub_unsafe's `set_alloc_ordered_by_a_channel_lets_other_threads_allocate`: a worker that
allocates through `Counted` only after a channel orders it after the call. It runs clean
under TSan and ASan.

`set_alloc` and `set_default_alloc` stay unsafe. Two of their obligations are not
expressible in types. The ordering of the hooks' unsynchronised C statics against every
other thread's use of them, the remaining C's included, is one. The ban on live allocations
crossing allocator families is the other. The obligation that libgit2's runtime stay
initialised guards against A being replaced silently, not against UB: every owner pins the
runtime before it allocates, and the pin is never released. It was kept as information.
