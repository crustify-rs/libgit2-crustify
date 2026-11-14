# A type-erased field whose type a sibling tag selects is one tagged enum with its tag

Status: `open`: the change is crustify `755ff2c` on
`libgit2-crustify/evolution/type-erased-field-tagged-by-a-sibling`, awaiting its review.

## Signal

I had to work out a rule by hand that a skill should have stated.

## Evidence

`wave-186/batch-2/run-1` wrapped `src/cli/opt.h`'s `cli_opt_spec`, whose `void *value`
points at an `int` for a spec of type `CLI_OPT_TYPE_BOOL`, `_SWITCH` or `_ACCUMULATOR`, at a
`char *` for `_VALUE` and `_ARG`, at a `char **` for `_ARGS`, and at nothing for `_NONE` and
`_LITERAL`: `opt.c`'s `parse_long`, `parse_short` and `parse_arg` cast it by the spec's `type`
before they write through it. `binding-types`' Field accessors section gives a type-erased
field "a generic element type when it is known, and the discovered untyped lifetime strategy
otherwise". Neither fits: the type is known, but per object, from a sibling field. Separate
setters for `type` and `value` would let safe code store an `int *` and then retype the spec
as `_VALUE`, so that the next read hands out a `Cell<Option<Arg>>` over an `int`. That is UB
reached from safe code, held by data the native code can change, against `binding`'s Safe
boundary.

The batch bound the pair as the Unions section binds a union and its discriminator. One
`Target<'p>` enum has a variant per kind, each holding its variable's safe form
(`&'p Cell<c_int>`, `&'p Cell<Option<Arg<'p>>>`, `&'p ArgsSlot<'p>`). `SpecRef::target` reads
both fields, and `SpecMut::set_target` writes both. `SpecRef::kind` reads the tag alone, and
nothing writes it alone (`src/git2/src/cli/opt.rs`, anchors `fld::cli_opt_spec.value` and
`fld::cli_opt_spec.type` on both accessors).

## Destination

The spec: `binding-types`' Field accessors section, which the binder evolves.

## Change

crustify `755ff2c`. The type-erased bullet of the Field accessors section gains one
sentence. A field whose pointee's type a sibling field selects, as a kind tag does, is
presented as a union is: one tagged enum over the tag and the pointer, each variant holding
its type's safe form, read from both and written together, so that no setter writes the tag
alone. The change drafts its `EVOLUTION.md` entry.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`755ff2c`, which also edited `EVOLUTION.md`; on 2026-11-14 it was put on the working branch's
tip as `7b738b9`, the same diff without that edit, since a change's row is drafted in its
proposal. Review-1 read `7b738b9`; the commit is now `144259c`.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a type-erased field whose type a sibling tag selects is one tagged enum with its tag | `libgit2-crustify`, `type-erased-field-tagged-by-a-sibling` | crustify `144259c` | `binding-types` had a type-erased field take one element type or the untyped strategy, and left a `void *` whose pointee a sibling kind field selects to be worked out by hand | a binding of such a field with a setter that writes its tag or its pointer alone, or a review finding of its pointer read as the wrong type after a tag write | watched |

## Validation

No failure prompted the change, and this batch's `Spec` is the case. Its `unit_pub` tests
round-trip every variant and check that a retyping write replaces the variable with the kind
(`targets_round_trip_with_their_kinds`, `set_target_replaces_kind_and_variable`). Its
`equiv` test has C's `cli_opt_parser_next` write each kind's variable through a table built
with `set_target` (`c_parses_a_table_built_in_rust`). Both pass on plain and under
ASan+UBSan, and the Rust-only ones under Miri.

## Review

None yet.

`review-1` (session `af7ac7fb-e754-446c-8e4c-1d7fac5bfbbe`, Opus 5.5, `$1.88`, 6m10s), on `7b738b9`: **approved**, with no fix required. It reran the two `unit_pub` tests the proposal cites, which passed; the `equiv` test left with the module's switch and could not be rerun cheaply. On 2026-11-14 the commit was put first in the stack, on crustify `4a0e778`, as `144259c`, its diff unchanged (the same `git patch-id`).
Its report is the longest `result` of `crustify/artifacts/evolution/type-erased-field-tagged-by-a-sibling/review-1/stream.jsonl`.
