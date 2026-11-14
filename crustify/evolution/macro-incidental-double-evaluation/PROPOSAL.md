# A macro that evaluates an operand twice only as a hazard is a function

Status: `landed` as crustify `e32ac8f` (record `a49c1b1`) on `libgit2-crustify/devel`, on 2026-11-14, once its
review approved it.

Goal: Idiomatic Rust: a function where a function expresses the macro.

## Signal

A rule an agent had to work out by hand that a skill should have stated.

## Evidence

`wave-200/batch-0/run-1` ported `mac::MIN` (`deps/ntlmclient/compat.h`),
`#define MIN(x, y) (((x) < (y)) ? (x) : (y))`. The expansion evaluates the operand it
returns twice and the other once.

`binding-symbols`' Macros section, which `porting-symbols`' Macros section defers to, gives a
macro that implements logic a function, and lets a `macro_rules!` take its place "only where
a function cannot express it: control flow that leaves its caller, token pasting or
stringification, an argument used as a place, or one evaluated lazily or more than once."
Read literally, `MIN` is such a macro, and so is every C `MIN`, `MAX`, `ABS` or `CLAMP` macro of
the usual form: its operand is evaluated more than once. Yet no caller relies on that: the
double evaluation is the classic hazard of such macros, not their meaning, and `ntlm.c`'s
three uses pass operands without side effects, whose result is the same whether they are
evaluated once or twice. A `macro_rules!` copying the double evaluation would carry the
hazard into Rust and lose the function's type checking. The batch ported a generic
`fn min<T: PartialOrd>(x: T, y: T) -> T`, documenting the departure, and decided the
reading by hand.

The rule's exception is right for a macro whose callers observe the repeated or lazy
evaluation: one that re-reads an argument it also writes, or evaluates an argument only on
one branch, as an `assert`- or `likely`-style macro, or a short-circuiting one, does. It
needs to say that the evaluation counts only where a caller depends on it.

The module pass, `wave-200/batch-1/run-1`, judged the port against `porting-symbols`' Macros
section and `testing` and filed no finding. Its reading of the exception agrees with the
batch's: the list names what a function cannot express of a macro's meaning, and `MIN`'s
second evaluation is no part of its meaning, since each operand at each site is the same
value every time it is evaluated, `plaintext_len`, `password_len`, `7` and `14`. The
preprocessed `ntlm.c` (the `plain` variant's flags) expands `compat.h`'s own definition, not
`<sys/param.h>`'s, which wavefront also records (`merge`'s, outside the closure), so the
`#ifndef MIN` guard selects no other body. The same section's "`const` where its body computes
on its arguments alone" is the second reading the batch settled by hand: a generic
comparison cannot be `const` on Rust 1.100 (`[const] PartialOrd` is E0658, `const_cmp`; a
plain bound is E0015), and the batch kept the macro's type-generic body over a `const fn` on
`usize`, which `ntlm.c`'s three sites would also take. The section could say which wins
when the two conflict; this proposal's sentence does not need to.

## Destination

The spec: `porting-symbols`' Macros section, which this role evolves; `binding-symbols`'
Macros section states the same exception for a binding, whose owner may take the same
sentence.

## Change

Add to `porting-symbols`' Macros section, after its first sentence:

> An argument the expansion evaluates lazily or more than once calls for a `macro_rules!`
> only where a caller depends on it: where every expansion site passes an argument without
> side effects, whose result is the same evaluated once, the macro is a function, its
> documentation naming the difference, as a `MIN` or `MAX` of the usual form is.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro that evaluates an argument more than once is a `macro_rules!` only where a caller depends on it; where every expansion site passes an argument without side effects it is a function, its documentation naming the difference | `libgit2-crustify`, `macro-incidental-double-evaluation` | crustify `e32ac8f` | "evaluated more than once" read literally makes every `MIN` and `MAX` a `macro_rules!`; two batches settled it by hand | a review finding of a `macro_rules!` that a function expresses, or of a function whose macro's caller depended on the repeated evaluation | watched |

## Validation

`wave-200/batch-0`'s tests: six `unit_pub` tests check the function against the macro's body,
`ntlm.c`'s three uses for every password length up to 20, ties, unordered floats, and that each
operand is evaluated once. The expansion sites were read in `ntlm.c` (lines 953, 987 and 988),
and wavefront's `query macros --name MIN` lists no other user of the header's macro.

## Review

None yet.

The change is made in `binding-symbols`' Macros section alone, where the list's fourth case
states the exception: `porting-symbols`' Macros section takes its forms from there, so one
sentence serves both. The sentence first drafted for `porting-symbols` is not added.

`review-1` (Opus 5.5, `$0.93`), on `4a69dd7`: **rejected**, one fix: "that last case" also took in an
argument evaluated lazily, which the test does not fit: one the C sometimes does not evaluate
can have no side effect and still fail when evaluated up front. Made as `e32ac8f`: the sentence
speaks of one evaluated more than once, and the row drops the lazy case. It weakens no check;
the condition is the analysis's to establish, over every expansion site.

`review-2` (Opus 5.5, `$0.22`), on `e32ac8f`: **approved**.
