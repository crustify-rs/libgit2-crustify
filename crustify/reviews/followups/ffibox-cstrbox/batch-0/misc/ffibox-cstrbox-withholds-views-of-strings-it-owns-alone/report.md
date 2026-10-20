# ffibox's `CStrBox` withholds borrowed views on a premise its own contract excludes

- **where**: ffibox, `src/refs.rs`. The `CStrBox` type docs (lines 572-588),
  `as_handles` (649), `as_ptr` (622-625), and the README's `CStrBox` rows. This is the
  uncommitted change in `/opt/venv/share/ffibox` on top of `4722185` (diff
  `sha256sum` `802b9393…a97ec`).
- **reached from this crate through**: every `OdbDataStr` reader in commit
  `ac7dd20e5`: `raw::data_str_bytes`, `OdbDataStrVec`'s `Debug`,
  `worktree::list`, and the tests that compare or decode strings.
- **class**: misc, a defect in ffibox: a design premise that contradicts the type's
  safety contract, and a stale doc line. Filed here and not fixed, as this batch's
  instructions ask. It is not undefined behaviour either way. It is a cost every
  consumer pays, plus documentation that misstates when that cost buys anything.
- **reproducer**: `src/main.rs`. Run
  `LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run` from this directory. It
  exits 101 on `ac7dd20e5` with the ffibox change (`run-affected.log`).

## Finding

The change removes `as_c_str`, `as_bytes` and `to_str` from `CStrBox`, on this ground:

> C keeps its pointer to this string, so a reference over the bytes would assert
> `noalias` / `readonly` over memory C may write

But the only way to make a `CStrBox` is `from_raw` / `from_raw_with`, whose `# Safety`
requires "a valid, NUL-terminated, **uniquely-owned** string that `D::c_drop`
releases". It hands out no `&mut` path to the bytes (`into_raw` consumes it). So under
its contract, no C code keeps a pointer it may write through while a `&self` borrow
lives. A caller who let C keep one would already break `from_raw`. Then the
element-wise reads would not help either: `CStrBox` is `Sync`, so a concurrent C write
would race the plain reads in `len()` and `CSlice::elem` as much as it races a `&CStr`.

ffibox makes the opposite call for the same ownership elsewhere. `CVec::as_slice`
hands out a real `&[T]` "because the buffer is owned exclusively", and the `CSlice`
docs say "`CVec` earns those by owning its buffer exclusively. A run inside a C object
does not". A `CStrBox` owns its string as exclusively as a `CVec` owns its buffer.

In libgit2 every `OdbDataStr` is owned that way, as the wrappers that make one show:
`BufMut::take_str` (the buffer is left empty), `StrArrayMut::take_strings` (the array
is overwritten with `EMPTY` first), `replace_string` in `core/types.rs` (the field is
swapped), and `OdbDataStrFree::dup` (fresh storage). Wavefront's records agree:
`git_strarray.strings` on output is owned by the struct, libgit2 "copies what it
keeps"; `git_signature.name` is owned on every public path. So every copy commit
`ac7dd20e5` introduces is paid where C cannot write. Decoding a string, or making a
path of it (which needs contiguous bytes), now takes a heap copy. The same bytes in an
`OdbDataVec<u8>` (`CVec`) under the same ownership take none:

```
decode + path, CVec<u8> (borrowed &[u8]):      0 Rust allocations
decode + path, CStrBox (copied out of a CSlice): 1 Rust allocations

thread 'main' panicked at src/main.rs:67:5:
assertion `left == right` failed: a string the CStrBox owns alone is copied to be read, where a CVec under the same ownership lends its bytes
  left: 1
 right: 0
```

`as_ptr`'s doc still gives the removed views as its reason: "`*const`, like
`CStr::as_ptr`, because the views are read-only".

## Suggested resolution (in ffibox, not applied; the design is the user's call)

Do one of these:

- keep `from_raw`'s unique-ownership contract and restore the borrowed views
  (`as_c_str`, `as_bytes`, `to_str`), bound to `&self`, with the `CVec::as_slice`
  argument as their justification; or
- if `CStrBox` is meant to also cover strings C keeps pointing at, relax `from_raw`'s
  contract to say so. In that case, also reconsider `Sync`: concurrent C writes race
  the element-wise reads.

In either case, fix `as_ptr`'s doc. A `CStrBox` that lives inside a C object (a
`char *` field viewed in place) is a case for the handle types, not for an owner whose
contract says it owns the string alone.
