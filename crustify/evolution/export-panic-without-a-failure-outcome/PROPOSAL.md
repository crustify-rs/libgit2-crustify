# An export whose failure needs an outcome a panic cannot give

Status: `open`.

## Signal

A rule an agent works around: `porting-symbols`' Functions section has a panic caught at an
export's gateway "reported as a failure, or aborts where the function cannot fail", and a
function can fail through a convention that a panic leaves no way to follow.

## Evidence

`wave-186/batch-5/run-1` (`git2/src/cli/opt.c`) exported `cli_opt_parse` and
`cli_opt_parser_next`. Both report their failures through a status and the outcome they write
to the caller's `cli_opt`: the CLI's commands then call `cli_opt_usage_error(cmd, opts,
&invalid_opt)`, which describes that outcome, its spec and argument. A panic in the parse
leaves no outcome to write: a zeroed one with a failing status would make the CLI print
`unknown option: (null)` for an argument that was never unknown. Both gateways abort on a
panic instead, documented on each. `cli_opt_foreach`, whose failure is a bare non-zero
`int` its caller (`common.c`) only tests, reports a panic as `-1`. The rule, read literally,
asks for the failure in all three; the batch departed from it for two.

## Destination

The spec: `porting-symbols`, Functions, the panic sentence.

## Change

Not made; drafted for review:

> A panic is caught at the gateway and reported as a failure where the function's error
> convention can report one from what the gateway holds, its return and the error state
> alone; it aborts where the function cannot fail, or where its convention reports a failure
> through an out-parameter or a buffer the panic left unwritten, which C would read as the
> failure's own: none unwinds into C.

## Validation

None yet: the rerun is `wave-186/batch-5`'s gateways under the changed rule, which they
already follow.

## Review

None yet.
