# How a binding keeps a C defect out of its safe API

Status: `landed`, as crustify `8f43548` on `devel`.

## Signal

A rule the review had to work out by hand that a skill should have stated, and a task
message that leads its agent to a rule no skill holds: the review task of wave 184 batch 1
asks to judge two C defects "against `binding`'s rule for C defects", and `binding`,
`binding-types` and `binding-symbols` hold none. The rule exists only on the verification
side: `testing` (Equivalence tests: "A confirmed C defect is not copied into Rust. The
divergence is documented …") and `reviewing` (`logic`: "Where the C departs from it too, the
defect is not copied into the Rust"; `equiv`: a divergence that "corrects a C defect on
purpose is no finding"). A binder reads the binding skills, not those, so it keeps C's
behaviour and documents it, and a review then files and fixes it.

## Evidence

- wave 184 batch 0 (`src/util/strlist.c`, `aba03725e`) bound `git_strlist_copy`,
  `git_strlist_copy_with_null` and `git_strlist_contains_key` as calls of the C, and kept two
  documented C defects: a refused copy leaks the array and earlier copies, and
  `git_strlist_contains_key` finds a name at the start of a longer one (`FOO=1` in
  `["FOOBAR=2"]`). Its module pass (wave 184 batch 1, run 1) filed both as `logic` findings
  with reproducers and fixed them: the copies composed from `git__calloc` and `git__strdup`
  (through `alloc.rs`'s strategies), `contains_key` from `git_strlist_contains_prefix` with
  the delimiter included, each anchor moved from `Wraps` to `Replaces`.
- Earlier batches worked the same answer out alone, each differently worded:
  `src/libgit2/src/libgit2/commit.rs`'s `create_from_stage` / `create_from_tree` ("composed
  from the routines it calls … for the same defects", `Replaces`), `repository.rs`'s
  `commit_parents` (`Replaces`, releasing a reference C leaks), and refusals in Rust before
  C runs (`tag.rs`, `tree.rs`, `config.rs`; the campaign's memory lists a dozen).
- `binding-symbols`' Lifecycle strategies section says a foundational item with a lifecycle
  role "becomes a generic policy or strategy that calls the primitive"; with a defective
  primitive the two rules pull apart, and nothing says which wins.

## Destination

The spec: `skills/rust/binding/SKILL.md`, Safe boundary (the binder's to evolve).

## Change

After the Safe boundary section's second paragraph:

> A confirmed C defect, a behaviour that departs from the C's specification as `reviewing`'s
> `logic` objective orders it, is not carried into the safe API. Where Rust can tell the
> defect's input before the call, the binding refuses it, with the error the C gives an
> invalid input; where it cannot, it composes the item from the routines its C calls,
> through their own bindings and in the C's order, correcting the defect, and its anchor is
> `Replaces`, as a port's is, its alias kept. A lifecycle primitive that is defective only on
> a failure path is composed so, while its policy still calls the primitives that are not.
> Each divergence is documented on the item, and kept by a unit test, as `testing`'s
> Equivalence tests section describes.

## Validation

The rule restates what the campaign's reviews already applied (the evidence above). Its
effect on a binder is not rerun; a lightweight rerun of `git_strlist_contains_key`'s wrap
would show whether the binder then gives the corrected answer instead of documenting C's.
