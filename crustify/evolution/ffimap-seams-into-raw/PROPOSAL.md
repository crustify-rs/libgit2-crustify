# The seam list names `from_raw` but not its inverse `into_raw`

Status: `landed`, as crustify `200bc65` on `devel`; scans name `into_raw` from the next wave, which the dashboard's Notes say.

## Signal

A tool's figures that the source contradicts: the `ffimap` skill's seam list, which every
scan passes as `--seams`, names ffibox's adoption `from_raw` and the run hand-offs
`from_raw_parts` / `into_raw_parts`, but not `into_raw`, the hand-off ffibox's own owners
give a single pointer (`CBox::into_raw`, `CStrBox::into_raw`, `CArc::into_raw`). An owner
that follows ffibox's naming then counts its give-up as pointer management a consumer faces,
while the adoption that reverses it is plumbing.

## Evidence

wave 184 batch 1 (`src/util/strlist.c` module pass), its scan with the skill's seams
(`crustify/tmp/ffimap-base-sites.json` in the batch's worktree): `raw_ptr.fn_sig_ret.safe.pub`
lists `src/libgit2/src/util/strlist.rs` lines 294, 302, 516 and 523. Three are seams
(`StrList::into_raw_parts`, `StrList::as_ptr`, `NullStrList::as_ptr`); line 516,
`NullStrList::into_raw`, the give-up of a NULL-terminated array, is not, though its inverse
`NullStrList::from_raw` is counted under `of_which.seam`. It is the crate's first `into_raw`
returning a pointer; the five others (`trailer.rs`, `buf.rs`, `index.rs`, `commit.rs`) return
C structs by value.

## Destination

The spec: `skills/tools/ffimap/SKILL.md`, the seam list of its preamble and its `--seams`
example (the binder's and translator's, as the tool skill's Binding section is; the
preamble is shared).

## Change

Add `into_raw` to the list ("the ownership hand-offs `from_raw_parts`, `into_raw_parts`,
`into_raw`, `from_c`, `into_c` and `as_c_ptr`") and to the example's `--seams`. Two scans
compare only with the same seams, so the change takes effect from the next wave, and the
dashboard's Notes name the wave it starts in.

## Validation

None run: the seam share of one counter moves by one site in this crate today.
