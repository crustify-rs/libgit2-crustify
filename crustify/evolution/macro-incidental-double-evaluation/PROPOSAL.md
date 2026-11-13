# A macro that evaluates an operand twice only as a hazard is a function

Status: `open`.

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

## Validation

`wave-200/batch-0`'s tests: six `unit_pub` tests check the function against the macro's body,
`ntlm.c`'s three uses for every password length up to 20, ties, unordered floats, and that each
operand is evaluated once. The expansion sites were read in `ntlm.c` (lines 953, 987 and 988),
and wavefront's `query macros --name MIN` lists no other user of the header's macro.

## Review

None yet.
