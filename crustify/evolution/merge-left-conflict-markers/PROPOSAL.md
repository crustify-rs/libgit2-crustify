# A merge into a wave branch is checked before it is committed

Status: `landed`, as crustify `5de4538` on `devel`.

## Signal

A failed batch's precondition, and a rule no skill states. `4730ad9ba` ("wave-169 takes the
static campaign link") merged the working branch (`d21c78c70`) into wave 169's branch. It
was committed with the conflict markers of three files still in it:

- `libgit2-sys/bindgen/wrapper.h`, at line 30;
- `libgit2-sys/build.rs`, at lines 188 and 230; and
- `libgit2-sys/src/bindings.rs`, at line 14476.

The wave's base therefore does not build, and every batch forked from it inherits that.
`git grep -n '^<<<<<<<' 4730ad9ba -- libgit2-sys` lists them.

## Evidence

`foundational/libgit2/deps/reftable/basics.c` `wave-169/batch-2/run-1` found the markers
before its first build. The conflicts paired batch 0's way of finding the internal
headers (`INTERNAL_INCLUDE` and the CMake cache's include dirs, `85ad07673`) with the
working branch's way (the compile edge of `build.ninja`, `d21c78c70`). The working branch's
way subsumes batch 0's. The batch resolved them in its own changeset: `build.rs` and
`wrapper.h` as `d21c78c70` has them, and `bindings.rs` regenerated. Batch 1 of the same
wave forked from the same base and must resolve them too, so the two batches' landings
conflict on the same hunks.

## Destination

The spec: `versioning`, Integration and Lifecycle › Waves, which the assistant follows when
it merges into a wave or module branch. The binder does not own it.

## Change

In `versioning`, Integration, step 1, after "Any other conflict is a defect of the
integration, resolved without discarding either side.":

> No merge is committed while its tree holds a conflict marker (`git diff --check`, or
> `git grep -n '^<<<<<<< '`), and its tree is built in campaign mode before the commit, so
> that the batches forked from it start from a tree that builds.

The same applies wherever a branch takes another by merge: a wave's or a module's branch
taking the working branch, as `4730ad9ba` did.

## Validation

None beyond the evidence: the check is the one this batch ran by hand.
