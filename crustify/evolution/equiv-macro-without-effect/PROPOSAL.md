# A macro whose expansion computes nothing has no equivalence test to run

Status: `open`.

Goal: Equivalence: its tests compare the Rust with the C, and a test that compares nothing
is no evidence of it. Also Consistency: two batches facing the same kind of macro reach the
same tests.

## Signal

The recurrence signal of a watched row of the record: a dashboard note and a commit message
that settle again what stands in for a macro's equivalence tests. The row is
`equiv-unreachable-macro-sites` (crustify `9780804`): "a task message, report, commit message,
dashboard note or plan correction that settles again what stands in for a macro's
equivalence tests". The change that landed covers a macro whose expansions no reachable
function runs. It does not cover a macro whose expansions reachable functions do run but
which computes nothing they could show. So this run had to settle the question by hand.

## Evidence

`wave-219/batch-0/run-1` (`src/util/cc-compat.h`) ported `mac::GIT_UNUSED` as
`const fn unused<T: ?Sized>(_: &T)`. On this build (clang, `__GNUC__`) the macro expands to
`do { __typeof__(x) _unused __attribute__((unused)); _unused = (x); } while (0)`, and
elsewhere to `((void)(x))`. Wavefront records 338 functions that expand it
(`query macros --name GIT_UNUSED`). Many of them are reached by the raw bindings, such as
`git_odb_backend_data_free`, whose `backend` parameter it discards. The rules the batch
reads say:

- `testing`'s Equivalence tests section: "A macro that implements logic is compared where it
  expands, through the functions that hold its expansions". `GIT_UNUSED` implements no
  logic, so the sentence does not apply to it. Read loosely, it asks for a comparison
  through, say, `git_odb_backend_data_free`. That test compares the function's own
  behaviour, which the macro's expansion does not change, so it would compare nothing of
  the macro.
- The landed exception, "When no function the raw bindings reach runs any of its
  expansions", does not apply either: hundreds of reachable functions run them.

The batch wrote no equivalence test of `GIT_UNUSED`. Its `unit_pub` tests check what the
replacement list does: the argument is evaluated, and neither moved nor dropped, whatever
its type. It recorded its reason in the module's docs (`tests/equiv/src/util/cc_compat_h.rs`)
and in its dashboard note: the expansion only evaluates its argument, and computes nothing
that a function holding it could show. The batch's other macros in the same header, `PRIdZ`,
`PRIuZ`, `PRIxZ` and `va_copy`, compute values the C formats, and are compared through
`git_error_set`.

## Destination

The spec: `testing`'s Equivalence tests section, a protected path (`skills/verification/**`),
which lands only with the user's approval. The translator does not evolve `testing`, so the
change is proposed, not made.

## Change

Append to the paragraph that ends "until the port of the module that holds them compares
them":

> A macro whose expansion computes nothing a caller can observe, such as one that only
> evaluates its argument to mark it used, has no equivalence test either, wherever it
> expands: a function that holds it behaves the same without it. Its unit tests check what
> the replacement list does to its argument.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro whose expansion computes nothing a caller can observe has no equivalence test, wherever it expands; its unit tests check what its replacement list does to its argument | `libgit2-crustify`, `equiv-macro-without-effect` | — | a run settled by hand that a macro which only marks its argument used, expanded in functions the bindings reach, has no equivalence test | a task message, report, commit message, dashboard note or plan correction that settles again what stands in for a macro's equivalence tests | — |

The row `equiv-unreachable-macro-sites` is `recurred` by this proposal's evidence.

## Validation

Not run: no batch failed. Read under the sentence, `wave-219/batch-0`'s tests meet it as
they stand: unit tests of `unused`, and no equivalence test of `GIT_UNUSED`. Its equivalence
tests of `PRIdZ`, `PRIuZ`, `PRIxZ` and `va_copy` fall under the section's first sentence and
stay.

## Review

None yet.
