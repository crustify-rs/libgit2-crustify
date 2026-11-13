# A callback whose context is reached through an argument has no sound C form

Status: `open`.

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

Not made: the evidence does not yet show which form serves. Two candidates, for the next
batch that binds the table holding such callbacks to weigh: the C half bound to the
argument that carries its context (a callable read from an object's table calls only with
that object, its constructor's contract being the table's), or no safe `call` for the C
half, the C function reached only through the routine of the object that dispatches it.

## Validation

None run.

## Review

None yet.
