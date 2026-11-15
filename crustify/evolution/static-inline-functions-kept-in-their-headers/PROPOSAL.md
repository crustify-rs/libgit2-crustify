# When a ported `static inline` function stays in its header

Status: `open`.

## Signal

Two texts disagree, and a run had to follow one of them against the other (`evolution` §2,
"skills that contradict each other, or an agent prompt that leads its agent wrong").
`porting-symbols` § Static inline functions has every ported `static inline` function lose its
header definition at its module's switch: "the header's definition is replaced by its
prototype, and the remaining C calls the Rust module's export". `porting-workflow` § 1.1 states
the principle behind it: "each function has one implementation". The task message of
`wave-226/batch-1/run-1` said the opposite, and cited `porting-symbols` for it: "A macro or a
`static inline` function the remaining C expands from `thread.h` or `unix/pthread.h` stays in
the header and needs no export, as `porting-symbols` describes". The section says that of a
header's macros, not of its `static inline` functions.

## Evidence

`wave-226/batch-1/run-1` (`src/util/thread.c`, symbol route) ports `thread.h`'s eleven
`static inline` atomic operations, `git_atomic32_{get,set,inc,dec,add}`,
`git_atomic64_{get,set,add}` and `git_atomic__{compare_and_swap,swap,load}`. Each body is one
`__atomic` builtin in this build: no call, no state of the module. wavefront lists 57 C
callers of `git_atomic32_inc` alone: every `GIT_REFCOUNT_INC`, the runtime's `init_count`, and
the object cache's total. None of them is ported. The run followed the task message. The ports
are native methods (`Atomic32Ref::inc` and its siblings, `atomic_compare_and_swap` over a
pointer-sized standard atomic). There is no export and no export feature, and `thread.h`
keeps its bodies.

The two forms are equivalent where C and Rust share an object. Both are the C11 memory model's
sequentially consistent operations on the same bytes. The run's equivalence tests run C's
`git_atomic32_inc` and the port's from four threads on one count, and race C's and Rust's
compare-and-swap on one slot. Both pass, plain, under ASan+UBSan and under TSan.

Under the section as written, the same port would make these changes at the switch:

- replace each body by a prototype;
- route every reference count's increment in the remaining C through an exported Rust call
  that cannot be inlined, since cross-language LTO is the section's own TODO; and
- add eleven exports and their gateway.

These changes keep no behaviour the header does not already keep. They also change no state
the Rust holds: the C's body reaches nothing of the module but the object it is given. The
same holds for any C function whose body is the language's own operations on its arguments:
an atomic builtin, a byte swap, an arithmetic helper.

## Goal

Correctness of the spec (no two texts disagree), and Idiomatic Rust / performance: what C
expands keeps its native code. The fitness measure: no task message contradicts
`porting-symbols`, and no export is added that only forwards one builtin.

## Destination

The spec: `porting-symbols` § Static inline functions. It is unprotected, and the translator
role evolves it.

## Change

Add a paragraph to `porting-symbols` § 4, after its first:

> A `static inline` function whose body reaches nothing but its arguments and the operations
> of the language and its compiler, such as an atomic builtin or a byte swap, keeps its body
> in its header for the remaining C, as a header's macro does. It is unexported, and its C and
> Rust forms are not linked beyond the Rust form's annotation. They share no state, so the
> one implementation `porting-workflow`'s Principles section asks for is the Rust one for
> Rust consumers. A remaining C caller that expands the header keeps the header's
> instructions, which an export it could not inline would only slow. Its equivalence tests
> compare the two forms. Where C and Rust share an object it works on, they also run both
> forms on it at once. The body goes when its header goes, with the last C file that
> includes it.

`porting-workflow` § 1.3's sentence on the headers bundled with a module, "declaring the
module's exports, its `static inline` functions included", gains "but those `porting-symbols`
leaves defined in them".

## Validation

None run: the rule is the one `wave-226/batch-1/run-1` followed under its task message. A
rerun under the change would produce the same port, so the change only removes the
contradiction. Whether a body "reaches nothing but its arguments" is read from the source, as
the analysis reads it now.

## Row

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a `static inline` function whose body reaches only its arguments and the language's own operations keeps its body in its header, unexported, as a header's macro does | `libgit2-crustify`, `static-inline-functions-kept-in-their-headers` | — | a task message kept `thread.h`'s atomics in their header against `porting-symbols` § 4, which exports every ported `static inline` function at the switch | a switch that replaces such a body by a prototype and exports it, or a task message or review finding that keeps a `static inline` body the rule does not cover | — |

## Review

None yet.
