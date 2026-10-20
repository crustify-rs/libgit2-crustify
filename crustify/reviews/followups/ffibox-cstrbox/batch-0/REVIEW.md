# Review: follow-up `ffibox-cstrbox` / batch 0 (the ffibox `CStrBox` integration update)

Reviewed commit `ac7dd20e5` ("follow ffibox's CStrBox changes in the bound string
wrappers") against the uncommitted ffibox change in `/opt/venv/share/ffibox`: `4722185`
plus `git diff` (`README.md`, `src/refs.rs`, `tests/macros.rs`; diff `sha256sum`
`802b9393…a97ec`). The commit has no `Batch:` trailer. Its items are the ones whose
wrappers it rewrites:

| home | items | what the commit changed |
|---|---|---|
| `odb/sys/odb_backend.rs`, `raw.rs` | the `data_str` strategies (`git_odb_backend_data_alloc` / `_free` for strings) | new `pub fn data_str_bytes`; `OdbDataStrVec::{dup_all, try_clone}` through a dup closure; `Debug` through a `CString` copy |
| `repository/worktree.rs` | `git_worktree_list` | `list` copied each name through `data_str_bytes` |
| `core/buffer.rs`, `core/strarray.rs`, `core/types.rs` | `git_buf`, `git_strarray`, `git_signature` | unit tests only |
| `tests/{equiv,ub_safe,ub_unsafe}` | as above | assertions through `data_str_bytes`; three `compile_fail` cases |

Every rewritten API was re-read against the C it wraps, and against the C paths that
store, free or keep what it hands out.

| objective | outcome |
|---|---|
| no new reference over C-held string memory | Holds. The commit forms no `&CStr` / `&[u8]` over an `OdbDataStr`: `data_str_bytes` and `Debug` copy element-wise. The `CStr::from_ptr` in `str_dup_raw` (the `Clone` strategy, which `try_clone` now reaches as `Clone` already did) predates it. Neither it nor the new `list` borrowing below covers memory C can write: libgit2 keeps no pointer to any `OdbDataStr`. |
| copying not paid where C cannot write | Not met. Every copy the commit adds is paid where C cannot write. Two avoidable ones are fixed: `misc/odb-data-str-copies-where-libgit2-cannot-write` (`worktree::list`'s per-name copy, two copying sort comparators that also tripped clippy, and `data_str_bytes`'s doc, which blamed a pointer libgit2 does not keep). The rest is forced by ffibox and filed: `misc/ffibox-cstrbox-withholds-views-of-strings-it-owns-alone`. |
| old behaviour kept (non-UTF-8, empty, NUL) | Holds in this crate. `data_str_bytes` keeps every byte (`ub_safe::sys_odb_backend::str_keeps_bytes_that_are_not_utf8`, the empty `take_str` case in `buffer`'s unit tests). A `CStrBox` cannot hold an interior NUL, and the NUL is excluded as before. `try_clone` makes the same allocations in the same order (`equiv::sys_odb_backend::string_allocation_failures`). `OdbDataStrVec`'s `Debug` still formats like `CStr` (new unit test). `list`'s result is unchanged, now also checked on non-UTF-8 names (new equivalence test). ffibox's own `CStrBox` `Debug` does change, filed as `misc/ffibox-cstrbox-debug-diverges-from-cstr`. |
| the three `compile_fail` cases | `odb_data_str_vec_element_outlives_owner` and `odb_data_str_vec_aliased_reborrow` still read the string after the illegal step. `odb_data_str_view_outlives_owner` used `run.len()`, which reads no string memory: `misc/odb-data-str-view-compile-fail-reads-no-string`, fixed (`run.elem(0)`). All three pass trybuild. |
| unsafety | No finding. The commit adds one safe `pub fn` and no `unsafe`, raw pointer or `unsafe fn`. |
| contracts | No finding. The only unsafe code it touches is `dup_each`, whose slot `// SAFETY:` comments still hold with the new closure. |
| UB | No finding. No new unsafe API, and ASan/UBSan, TSan and Miri are clean (below). |
| equivalence | No finding in this crate (see above). The `Debug` divergence is ffibox's. |
| internal | No finding. |
| conventions | Nothing to file. The commit put `use std::ffi::CString;` among the `core` imports of `odb_backend.rs`; moved below them while touching the file. Its message says the wrappers "copy through CString", while they copy into a `Vec<u8>` (only `Debug` goes through a `CString`). |
| misc | 4 reports: 2 on the crate, fixed; 2 on ffibox, filed with reproducers and not fixed, as this batch's instructions ask. |
| coverage | 2 tests added; see Coverage. |
| wavefront | The records for the outputs read here hold: `git_strarray.strings`, `git_buf.ptr`, `git_signature.name`. They are owned by the struct, and libgit2 copies what it keeps. They are the evidence for the ffibox report. The commit changes no ownership modelling, so nothing was resubmitted. |
| ffimap | Not available on this host (no `ffimap` binary; `crustify` has no `ffimap` subcommand). The smells were searched by hand in the touched files (`CStr::from_ptr`, `slice::from_raw_parts`, `from_utf8_unchecked`). None is new. |

## Reports

| report | where | status |
|---|---|---|
| `misc/ffibox-cstrbox-debug-diverges-from-cstr` | ffibox `CStrBox` `Debug` | filed, not fixed; reproducer exits 101 |
| `misc/ffibox-cstrbox-withholds-views-of-strings-it-owns-alone` | ffibox `CStrBox` docs / API | filed, not fixed; reproducer exits 101 |
| `misc/odb-data-str-copies-where-libgit2-cannot-write` | `data_str_bytes`, `worktree::list`, two comparators | fixed |
| `misc/odb-data-str-view-compile-fail-reads-no-string` | `compile_fail/odb_data_str_view_outlives_owner.rs` | fixed |

## Gates

Every run went from a snapshot of its tree. The snapshot is a `git archive` of the base,
or a `tar` of the working tree checked with `diff -r` against the worktree, plus
`include/`, the `builds` and `tests` symlinks, and `git init`.

| run | base `ac7dd20e5` | landed tree |
|---|---|---|
| Miri gate: one `cargo +nightly miri test … -- --skip compile_fail` per binary, in parallel | `libgit2-sys` (0 tests), `lib` (319 pass, 393 ignored), `ub_safe` (154 / 591) and `ub_unsafe` (18 / 78) are clean. `equiv` **aborted** at `object::lookups_in_a_repository_without_paths_match_c`: `can't call foreign function git_libgit2_init`. | all clean. `equiv`: 32 pass, 426 ignored. `lib`: 319 / 394 (the new unit test calls libgit2). |
| `cargo-variant.sh plain test --workspace --no-fail-fast` (doctests and trybuild included) | pass | pass: lib 712, equiv 465, ub_safe 752 (170 `compile_fail` cases), ub_unsafe 96, doctests |
| `cargo-variant.sh asan-ubsan test --workspace -- --skip compile_fail <filters>` (LSan on) | pass | pass: lib 148, equiv 80, ub_safe 176, ub_unsafe 21, doctests 5 + 28 |
| `cargo-variant.sh tsan test --workspace -- --skip compile_fail <filters>` | pass | pass, same counts |
| `cargo-variant.sh plain clippy --workspace --all-targets` | 2 `unnecessary_sort_by` warnings, both from this commit | clean |
| `cargo check --workspace` | | clean |

The ASan/TSan filters were `buffer strarray odb_backend signatures worktree sys_alloc
types::`. They select every test the commit touches and both tests added here.

The base `equiv` abort is outside this commit. Twelve tests from earlier waves call
libgit2 without the Miri attribute: `object::{lookups_match_c,
lookups_in_a_repository_without_paths_match_c, lookups_of_unparsable_objects_match_c,
peel_and_bypath::peels_and_path_lookups_match_c}` (after the first was fixed, the next
stopped the binary too), `tag::tag_object::{lookups_match_c,
lookups_in_a_repository_without_paths_match_c, lookups_of_unparsable_tags_match_c}`,
and `tree::{tree_entry::{copies_match_c, path_lookups_match_c},
tree_object::{getters_match_c, lookups_match_c,
lookups_in_a_repository_without_paths_match_c, lookups_of_unparsable_trees_match_c}}`.
Each was run alone under Miri and stopped at `git_libgit2_init`. Each now carries
`#[cfg_attr(miri, ignore = "calls libgit2")]`, as `testing.md` prescribes. Six other
candidates without the attribute ran to completion under Miri and were left alone.

BSan was not run. No test or reproducer here is expected to trigger it: the reproducers
compare formatting and count allocations, and `list` now borrows its names only before
`take_strings`, through the existing `StrArrayRef::iter`. This is not a subsystem's
last wave.

Reproducers (from their directories, `LD_LIBRARY_PATH=/target/crustify/builds/plain
cargo run`): both ffibox reproducers exit 101 on `ac7dd20e5` (`run-affected.log`). They
target ffibox, which this batch does not change, so they still exit 101 on the landed
tree.

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, landed tree; the `odb_backend worktree
buffer strarray signatures sys_alloc` tests of lib, `equiv`, `ub_safe`, `ub_unsafe`):

- `odb/sys/odb_backend.rs` lines 330-600 (`data_str_bytes`, `OdbDataStrFree`,
  `OdbDataStrVec` with `dup_all`, `try_clone`, `dup_each`, `Debug`): 114 of 114.
- `repository/worktree.rs` `list` (1296-1340): 26 of 27. The uncovered line is the `?`
  on a failing `git_worktree_list`. As root, the only way to make it fail is an
  allocation failure (`dirload` reads any directory), so it is environment-dependent.

C (gcov of the `coverage` build, private `GCOV_PREFIX`): `git_worktree_list`
(`worktree.c:36-75`) is covered except `:51` (`git_str_joinpath` allocation failure)
and `:55` (`git_fs_path_dirload` failure), for the same reason.

Gaps filled:

- `odb_backend::unit_tests::str_vec_debug_formats_each_string_as_cstr_does` (unit:
  Rust-only `Debug`). Before, only `["x", "y"]` was tested. The test compares the run's
  `Debug` with the `CStr`s' on empty, multi-byte UTF-8, non-UTF-8, quote and backslash,
  and control bytes, plus the empty run.
- `equiv::worktree::routines::lists_of_names_that_are_not_utf8_match_c` (equivalence).
  The existing `List` steps compare names through `to_string_lossy`, which hides byte
  differences. This test compares `list` with `git_worktree_list` byte for byte on two
  non-UTF-8 worktree names beside the two ordinary ones.
