Filed by the assistant from 7db163587 and 81dede2b1, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `Names`, `NamesVec` and `NamesRef` are neither `Send` nor `Sync`

- **finding**: `ergonomics/names-confined-to-their-thread`
- **batch**: `wave-169/batch-5/run-1`, the module pass of `libgit2/deps/reftable/basics.c`,
  over wave 169's batches 0 to 4 as the wave tip `9aa6520a5` holds them
- **items**: `fn::parse_names`, `fn::names_length`, `fn::names_equal`
- **affected commit**: `9aa6520a5`
- **source commits**: `7db163587` (file), `81dede2b1` (fix)

## Finding

`Names` (`parse_names`' result), `NamesVec<'_>` and `NamesRef<'_>` are neither Send nor
Sync: their raw pointers withhold both, and no impl states why. Their standard counterparts,
`Vec<CString>`, `Vec<&CStr>` and `&[&CStr]`, cross threads, and ffibox's G1 asks for the
counterpart's traits, each departure stated. Nothing needs the departure. A `Names` owns its
array and strings in reftable's allocator, which any thread may release (the module's Alloc
contract, libc's and libgit2's allocators), made only after the runtime is pinned. A
`NamesVec` holds `&CStr` borrows and the array of their pointers. Everything a `NamesRef`
reaches reads: its iterator, `names_length` and `names_equal`. Batch 3 made `Buf` Send and
Sync on the same grounds.

## Evidence

On `9aa6520a5` the reproducer does not compile:

```
error[E0277]: `NonNull<*const i8>` cannot be sent between threads safely
  (NamesRef, basics.rs:730)
error[E0277]: `*mut i8` cannot be sent between threads safely
  (Names, basics.rs:1362)
error[E0277]: `*const i8` cannot be shared between threads safely
  (NamesVec, basics.rs:777)
error: could not compile `names-confined-to-their-thread` due to 3
  previous errors
```

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. The reproducer
(`#![forbid(unsafe_code)]`) moves a parsed list to another thread and shares a built list
and its view with two more. Run from its own directory, with W the repository root:

```
CARGO_TARGET_DIR=$W/crustify/tmp/repro-plain CRUSTIFY_BUILD_VARIANT=plain cargo run
```

## Fix

In `81dede2b1`: in ffi_import, `Names`, `NamesVec<'_>` and `NamesRef<'_>` are Send and Sync,
each impl with its proof; the module documentation says so. Tests: unit_pub
`name_lists_cross_threads_as_a_vec_of_strings_does` (compile time, runs under Miri) and
ub_safe `name_lists_move_to_and_are_read_by_other_threads` (TSan). The reproducer now
compiles and prints "names crossed threads".
