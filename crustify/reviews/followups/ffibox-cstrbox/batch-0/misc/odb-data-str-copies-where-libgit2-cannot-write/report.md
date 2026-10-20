# Commit `ac7dd20e5` copies strings libgit2 cannot write, and justifies it with a pointer libgit2 does not keep

- **items**: `raw::data_str_bytes` (`odb/sys/odb_backend.rs`), `worktree::list`
  (`git_worktree_list`, `repository/worktree.rs`), and the two sort comparators the
  commit rewrote: `core::strarray::unit_tests::owned_arrays_copy_edit_and_release` and
  `ub_safe::sys_odb_backend::str_vec_elements_replace_and_move_in_place`.
- **affected commit**: `ac7dd20e5`
- **class**: misc. Inaccurate documentation, a needless copy on a public path, and two
  `clippy::unnecessary_sort_by` warnings. No behaviour difference: `list`'s result is
  unchanged.
- **fixed**: yes, in this batch's second commit.

## Finding

1. `data_str_bytes`'s doc gives its reason as "libgit2 keeps its own pointer to the
   string". libgit2 keeps no pointer to any `OdbDataStr`. Each is sole-owned: fresh
   from `OdbDataStrFree::dup`, or detached by `BufMut::take_str`,
   `StrArrayMut::take_strings` or `replace_string` before it is adopted. That is what
   `CStrBox::from_raw`'s contract requires. The copy is the price of ffibox's API (see
   `../ffibox-cstrbox-withholds-views-of-strings-it-owns-alone`), not of anything
   libgit2 does, and the doc sent readers to the copy even for comparisons, which need
   none.
2. `worktree::list` copied every name into a fresh `Vec<u8>` (`data_str_bytes`) only to
   join it onto a path, which copies again. The names belong to the `StrArrayVal` the
   call just filled. `git_worktree_list` keeps nothing (`worktree.c:36-75` hands its
   detached vector over). `StrArrayRef::iter` already lends them as `&CStr` while the
   array owns them, on the handles' invariant that only `StrArrayMut` writes.
3. The commit turned two `sort_by` comparators into
   `data_str_bytes(a).cmp(&data_str_bytes(b))`. That is two heap copies per comparison,
   and `cargo clippy --workspace --all-targets` reports `unnecessary_sort_by` on both
   (`core/strarray.rs:814`, `tests/ub_safe/sys_odb_backend.rs:293`). They were the only
   clippy warnings in the workspace.

Evidence: read against `src/libgit2/worktree.c:36-75` and Wavefront's
`git_strarray.strings` record ("libgit2 … copies what it keeps"). From
`crustify/rust`, `scripts/cargo-variant.sh plain clippy --workspace --all-targets` on
`ac7dd20e5` shows:

```
warning: consider using `sort_by_key`
   --> libgit2/libgit2/core/strarray.rs:812:9
...
warning: consider using `sort_by_key`
   --> libgit2/tests/ub_safe/sys_odb_backend.rs:293:5
```

## Fix

- `data_str_bytes`'s doc now says why a copy is needed: a path or a decode needs
  contiguous bytes, and `CStrBox` lends none. It also says that libgit2 keeps no
  pointer, and that a comparison goes through `as_handles().elems()` with each
  `c_char` read as `u8` (as `CStr` compares; a raw `c_char` comparison is signed).
- `list` decides which entries to keep from `names.as_ref().iter()` before it takes the
  strings out, then swaps them as before. There is no per-name copy, and the result
  and its order are unchanged (`equiv::worktree::routines::*`,
  `ub_safe::worktree::lists_*`, and the new
  `equiv::worktree::routines::lists_of_names_that_are_not_utf8_match_c`).
- Both comparators compare the bytes in place, through a small `bytes()` helper
  (`as_handles().elems()` as `u8`). Clippy is clean on the landed tree.
