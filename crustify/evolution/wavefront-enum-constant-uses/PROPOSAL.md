# wavefront records no use of an enum constant

Status: `open`

## Signal

A tool that reports what the source contradicts (`evolution` §2): a function that uses an
enum's constants, without naming the enum's type, does not depend on the enum in wavefront's
graph, so the plan can schedule it, and its module, before the enum and its home.

## Evidence

- `reftable_buf_add` (`c/deps/reftable/basics.c:122`) returns `REFTABLE_OUT_OF_MEMORY_ERROR`,
  a constant of `enum reftable_error` (`deps/reftable/reftable-error.h`). Its record
  (`wavefront --config crustify/wavefront/configs/wavefront-config.json c query functions
  --name reftable_buf_add`, wavefront `e529c26`) lists in `depends_on` only
  `REFTABLE_ALLOC_GROW`, `memcpy`, `reftable_alloc_grow` and the type `reftable_buf`; the
  enum's record (`query enums --name reftable_error`) lists no user at all.
- wavefront extracts no such edge: `src/edges/` has no query over `EnumConstantAccess`, and
  `crustify/wavefront/codeql/t2/` no table of it.
- Consequence in this campaign: `deps/reftable/basics.c` was scheduled at wave 169 with
  `depends_on.scheduled` empty, and its batch defined an unanchored `basics::Error` that the
  enum's port, in wave 217, must take over (`reftable-error-wrapped-ahead-of-basics`).
- The pattern is pervasive in C: `c/src` and `c/deps` hold 307 `return GIT_E…` /
  `return REFTABLE_…_ERROR` statements alone, most in functions returning `int`.

## Destination

wavefront: a T2 edge query over enum-constant accesses, the graph's dependency on the
constant's enum, and the `query` records' `depends_on` / `used_by`; then a planner run to
re-plan the `todo` waves under it.

## Change

Extract each access of an enum constant, from the enclosing function, global initialiser or
macro, with its enum, and add it to the user's dependencies as a use of the enum type
(as `local_type_uses` does for a declared variable), so that ordering, closure and
`depends_on` follow. With its test, a reproducer over a fixture where a function returns
an enum constant through `int`.

## Validation

Not yet: wavefront's suite, plus the count of new dependencies the extraction yields over
libgit2 and of the `todo` waves whose `depends_on` it changes.
