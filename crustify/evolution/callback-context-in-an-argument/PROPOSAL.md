# A callback whose context is reached through an argument has no sound C form

Status: `landed` as crustify `a43b6b8` (record `98ed4a1`) on `libgit2-crustify/devel`, on 2026-11-14, once its
review approved it.

Goal: Safety, its soundness measure.

## Signal

Skills that leave a case unspecified that a binding cannot settle well (`evolution` §2).
`binding-symbols`' Function pointers section builds a callable type's C half from "a C
function, with its context, which Rust calls … through an `unsafe` constructor whose
contract states … its context's lifetime and threads, and safe to call". For a function
pointer "without a context parameter" it says the Rust half carries no captured state. It
does not cover the case where C has no context parameter because the context lives in an
argument: there the C function's soundness depends on the argument, not on anything the
callable type holds, so a safe `call` with an arbitrary argument cannot be sound.

## Evidence

`wave-194/batch-1/run-1`: `llhttp_cb` (`int (*)(llhttp_t *)`) and `llhttp_data_cb` take no
context; their context is the parser's `data`, which libgit2's C callbacks
(`transports/httpclient.c`'s `on_header_field`, `on_body`, …) read as an
`http_parser_context *`. The batch's `Callback::from_fn` (`pub(crate)`) can only require
that the function be sound "with every live, exclusively held parser, whatever its `data`
and `settings` hold", which no such C callback meets: `Callback::call(Parser::new()…)`
would hand it a NULL `data`. The sound C form is the call made through the settings its
parser executes with, whose contract (`set_settings_ptr`) vouches for that parser; the
batch leaves that to the settings binding (api.c's `wrap`).

## Destination

The spec: `binding-symbols`, Function pointers, its three cases that shape the type.

## Change

`wave-194/batch-2/run-1`, the next batch that binds the table holding such
callbacks, weighed the two candidates and took the first, which needs no new primitive:

- the callable type's C half keeps the strong contract, sound with any argument whatever
  context it carries, and a C function that reads its context from an argument is never
  one, so `call` stays safe;
- a table of callables, bound as a layout whose slots are set only from the callable types,
  is sound with any object, and the object's setter that stores it is safe, taking a
  `'static` borrow where C keeps its address (`ParserMut::set_settings(Option<&'static
  Settings>)`);
- a table C builds of functions that read their context from the object is stored only
  through the object's `unsafe` setter, whose contract vouches for the object's context
  (`set_settings_ptr`), and is read back from the object as the address it holds, not as the
  table's handle, whose getters would hand out callables callable with any object; its
  functions are reached only through the routine of the object that dispatches them (api.c's
  `CALLBACK_MAYBE`, `port` in wave 195), with that object.

Made, in `binding-symbols` § Function pointers, as a fourth case after "one C compares with a
known function":

> - one whose context C reaches through another argument, such as the object it is handed,
>   has a C half only for functions sound with any such argument. A C function that reads its
>   context there is reached only through the object that holds it, by the routine that
>   dispatches it with that object, bound or ported, which calls it through its raw binding,
>   and never as the callable type, whose safe call could hand it another object. Such a
>   function is stored only from C, or through an `unsafe` setter whose contract carries the
>   object's context, and is read back as its address, no callable.

## Evidence (`wave-194/batch-2/run-1`)

`llhttp_settings_s`, bound as `Settings` in `src/libgit2/deps/llhttp/api.rs`: its 23 slots
are set from `Callback` / `DataCallback` only, read back as them through the handles'
invariant (each non-NULL slot is one `from_fn` accepts), and `Settings` is `Send` and
`Sync`, so that a `static` holds one, as libgit2's `httpclient.c` does in C. `ParserRef`'s
`settings` getter stays the raw address: `transports/httpclient.c`'s `on_header_field`,
`on_body` and the others cast `parser->data` to `http_parser_context *`, so a
`SettingsRef` over that table would let `Settings`' getter return a `DataCallback` whose
safe `call(Parser::new().as_mut(), …)` hands them a NULL context.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a function pointer whose context C reaches through another argument has a callable C half only for functions sound with any such argument; one that reads its context there is reached only through its object | `libgit2-crustify`, `callback-context-in-an-argument` | crustify `a43b6b8` | a table of callbacks that read their context from the object they are handed: a callable handle over it would let a safe call hand them another object's, or no, context | a `ub` or `contracts` finding on a callable type whose C function reads its context from an argument | watched |

## Validation

None run.

## Review

`review-1` (Opus 5.5, `$1.19`), on `15845fd`: **rejected**, one fix required: "never as the callable
type" contradicted `porting-symbols` § 3, by which ported code calls a function pointer through
its type alone, for the ported routine that dispatches such functions. Fixed as `4a9557a`: that
routine, bound or ported, calls it through its raw binding. One unrequired rewording is taken
at the same length, so that the restriction falls on the functions, not on every setter of
the field. Evidence, goal, placement and row held; it weakens no check.

`review-2` (Opus 5.5, `$0.33`), on `4a9557a`: **approved**. It landed as `a43b6b8`, that commit rebased, its
file the same in the commit and its parent.
