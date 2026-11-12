Filed by the assistant from 1b6fb129c and ab780bd47, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# The lists' adoption contracts admit an array holding one string twice

- **finding**: `contracts/ffi_import/adoptions-admit-repeated-strings`
- **batch**: `wave-184/batch-1/run-1`, the module pass of `libgit2/src/util/strlist.c`, over
  wave 184 batch 0 (`aba03725e`), its six items
- **items**: `fn::git_strlist_free`, `fn::git_strlist_free_with_null`
- **affected commit**: `aba03725e`
- **source commits**: `1b6fb129c` (file), `ab780bd47` (fix)

## Finding

`StrList::from_raw_parts` and `NullStrList::from_raw` ask for an array of libgit2's
allocator holding NUL-terminated strings of it and for unique ownership of "those strings",
but not for distinct ones. An array holding one string twice honours every word, and
`git_strlist_free` / `git_strlist_free_with_null` then free it twice.

## Evidence

The reproducer gives up a list of ["a", "b"], points slot 1 at slot 0's string (releasing
its own) and adopts it: "AddressSanitizer: attempting double-free ... #2 git_strlist_free
src/util/strlist.c:92:3 #3 <StrListFree as CLenDrop>::c_drop_len strlist.rs:178:22", exit
134 (and `git_strlist_free_with_null`, strlist.c:105, with `null`). `run-affected.log` and
`disclosure.md` sit beside this report.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `aba03725e`. From its
directory, built with the asan-ubsan variant's flags:

```
CRUSTIFY_BUILD_VARIANT=asan-ubsan \
RUSTFLAGS="-Zsanitizer=address -Cforce-frame-pointers=yes" \
cargo +nightly-2026-10-08 build --target x86_64-unknown-linux-gnu
```

and run with and without the argument `null`.

## Fix

In `ab780bd47`: `StrList::from_raw_parts`' and `NullStrList::from_raw`'s `# Safety` now ask
that no two elements be the same string. ub_unsafe's `adopts_equal_but_distinct_strings`
(both suites) adopts equal strings in distinct allocations.

A run of the reproducer on the fixed tree: not recorded: the report was refused by the host;
filed from 1b6fb129c and ab780bd47 (the directory holds no run-fixed.log).
