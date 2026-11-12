Filed by the assistant from 1b6fb129c and ab780bd47, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `contains_key` finds a name at the start of a longer one

- **finding**: `logic/contains-key-matches-longer-names`
- **batch**: `wave-184/batch-1/run-1`, the module pass of `libgit2/src/util/strlist.c`, over
  wave 184 batch 0 (`aba03725e`), its six items
- **item**: `fn::git_strlist_contains_key`
- **affected commit**: `aba03725e`
- **source commits**: `1b6fb129c` (file), `ab780bd47` (fix)

## Finding

`contains_key` called `git_strlist_contains_key`, which compares only the bytes of key
before its delimiter, so a name is found at the start of a longer one:
`contains_key(["FOOBAR=2"], "FOO=1", '=') == true`, and an empty name ("=x") is found in
every non-empty list. Its one use, `merge_env` in src/util/unix/process.c (and
win32/process.c), then drops an inherited FOO when the caller sets FOOBAR, against process.h
("will add the given environment variables ... to the current environment") and POSIX's
naming of a variable by the bytes before its first '='. The wrapper documented C's answer as
kept.

## Evidence

The reproducer (`#![forbid(unsafe_code)]`) fails: "FOOBAR=2 sets FOOBAR, not FOO", exit 101
(`run-affected.log`, beside this report). `disclosure.md` sits beside it.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `aba03725e`. Run from its
directory with `cargo run` (plain variant).

## Fix

In `ab780bd47`: `contains_key` is native Rust: it finds the key's first delimiter and asks
`contains_prefix` (`git_strlist_contains_prefix`) for the name with its delimiter, so FOO=1
is no longer found in ["FOOBAR=2"], nor "=x" in every list; a key without the delimiter, or
a NUL delimiter, is still false without reading the strings. Replaces
`fn::git_strlist_contains_key`. The equivalence tests compare it with C's
`git_strlist_contains_prefix` of the name and delimiter, and with `git_strlist_contains_key`
wherever C does not diverge, and pin the three diverging keys of their table; the unit test
`contains_key_compares_the_delimiter` holds the corrected answers. The reproducer exits 0
(`run-fixed.log`).
