# src/util/integer.h's standard-library equivalent named only `TryFrom`

## What the earlier plan decided

`modules.json`, module `src/util/integer.h` (libgit2, index 27):

```json
"std": {"library": ["core::convert::TryFrom"], "feature_parity": "full", "notes": ""}
```

## Evidence

The header defines two families (`c/src/util/integer.h`): range checks
(`git__is_sizet`, `git__is_ssizet`, `git__is_uint16`, `git__is_uint32`, `git__is_ulong`,
`git__is_int`), which `TryFrom` provides, and overflow-checked arithmetic
(`git__add_sizet_overflow`, `git__multiply_sizet_overflow`, `git__add_int_overflow`,
`git__sub_int_overflow`, `git__add_int64_overflow`, `git__multiply_int64_overflow`), which it
does not: those are the integer types' `checked_add`, `checked_sub` and `checked_mul`.

```sh
grep -n "GIT_INLINE" c/src/util/integer.h
```

## Edit

`library` also names `core::primitive::{usize,i32,i64}::checked_{add,sub,mul}`, the paths
the overflow family maps to. The parity stays `full` and the objective `wrap`; nothing moves.
