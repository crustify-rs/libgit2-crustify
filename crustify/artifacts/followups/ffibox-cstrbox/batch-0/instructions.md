# Follow-up review: the ffibox `CStrBox` integration update

This batch reviews one commit that no translation wave scheduled, so three things differ
from your role. These are deliberate departures from the campaign layout; follow them
rather than reconciling them with it.

## Where it sits

- Its batch id is `followups/ffibox-cstrbox/batch-0`. File reports under
  `crustify/reviews/followups/ffibox-cstrbox/batch-0/`, and end each commit message with
  `Batch: followups/ffibox-cstrbox/batch-0`.
- It lands on `crustify/ffibox-cstrbox`, the update's own branch, not on a review wave
  branch. That branch is not checked out; land on it as `landing.md` describes.
- Do not edit `crustify/artifacts/dashboard.md`: its batch tables are keyed to the
  subsystems' schedules, which hold no row for this batch. The orchestrator records the
  batch in the dashboard's Notes.

## What changed, and what it wraps

ffibox's `CStrBox` no longer hands out references over its bytes: `as_c_str`, `as_bytes`
and `to_str` are gone, and `as_handles()` returns a `CSlice<'_, c_char>` whose elements are
copied out one at a time (`elem`, `elems`, `copy_to_slice`). The reason is that C keeps its
pointer to the string, so a `&CStr`, `&[u8]` or `&str` over it would assert `noalias` /
`readonly` over memory C may write. `len()` now reads byte by byte, and `Debug` escapes
bytes as `CStr`'s does. The ffibox change is uncommitted in its checkout at
`/opt/venv/share/ffibox`, which `crustify/rust/libgit2` depends on by path: read it there
with `git -C /opt/venv/share/ffibox diff`, and never edit that checkout. A defect you find
in ffibox itself is filed as a `misc` report in this batch's reviews dir, with its
reproducer, not fixed.

Commit `ac7dd20e5` has no `Batch:` trailer, so no `task.json` names its C items. They are
the items whose wrappers it rewrites from those views to copies through `CString`:

- `git_buf` (`core/buffer.rs`), `git_strarray` (`core/strarray.rs`), the `core/types.rs`
  types and `raw.rs`'s string strategies;
- the odb backend's `data_str` handling (`odb/sys/odb_backend.rs`);
- the worktree wrappers (`repository/worktree.rs`).

Compare each rewritten API against the C it wraps, and check in particular that:

- no new reference over C-held string memory has been formed;
- copying is not paid where C cannot write;
- the copies keep the old behaviour, including on non-UTF-8 bytes, interior and empty
  strings, and the NUL;
- the three `compile_fail` cases still test what they are named for.

## Gates

Run Miri over the whole workspace as `testing.md`'s Regressions section describes, on the
commit before your changes and again on what you land. Run every test this commit touches,
and every test you add, under `asan-ubsan` and `tsan`. Run BSan only on the individual
tests and reproducers you expect to trigger it, one at a time.
