# A function that is one field's read or write is ported onto that field's accessor

Status: `open` (the rule `wave-194/batch-3/run-1` followed; the spec change is not made).

## Signal

A case the how-to skills leave unspecified, which a port had to settle by hand (`evolution`
§2). `porting-symbols`' Functions section gives a ported function "the safe API
`binding-symbols`' Functions section gives a function", homed in the file its item names,
and the conventions' Anchors section allows one Rust item to stand in for several C items
only for a symbol alias and for the accessors of one field. Neither says what a port does
with a function whose whole body reads or writes one member of a struct whose layout is
already bound, with an accessor for that member, in another module's file: a new method
in the function's own home duplicates the accessor, and annotating the accessor puts the
function's anchor outside its home.

## Evidence

`wave-194/batch-3/run-1` (`deps/llhttp/api.c`, symbol route). Nine of api.c's functions,
`llhttp_get_type`, `llhttp_get_http_major`, `llhttp_get_http_minor`, `llhttp_get_method`,
`llhttp_get_status_code`, `llhttp_get_upgrade`, `llhttp_get_errno`,
`llhttp_get_error_reason` and `llhttp_get_error_pos`, are `return parser-><field>;`, and
`llhttp_set_error_reason` is `parser->reason = reason;`. `deps/llhttp/llhttp.rs`
(`llhttp.c`'s file, wave-194/batch-0) already has `ParserRef::parser_type`, `http_major`,
… `reason`, `error_offset` and `ParserMut::set_reason` over those members. The three
X-macro generators `HTTP_ERRNO_GEN`, `HTTP_METHOD_GEN` and `HTTP_STATUS_GEN`, each `case
PREFIX##NAME: return <text>;`, are batch 1's `Errno::constant_name`, `Method::map_string`
and `Status::map_string`. The batch's task message asked for these to be ported "as the
parser's native API, without duplicating what `llhttp.rs` gives": the batch added each
function's `Replaces` anchor and alias to the existing accessor beside the member's
`Wraps` one, so that 13 of its 28 items stand in through items in `llhttp.rs`, not in
their home `api.rs`, and wrote native code only for the items with logic of their own
(`execute`, `pause`, the resumptions, the lenient setters, `debug_trace`).

`wave-194/batch-4/run-1` met the case again with three functions whose whole body is a map's
text: `llhttp_errno_name`, `llhttp_method_name` and `llhttp_status_name` are each a `switch` of
the `case`s `HTTP_ERRNO_GEN`, `HTTP_METHOD_GEN` and `HTTP_STATUS_GEN` make, with a `default:
abort();` no Rust enum reaches. It ported them as `Errno::constant_name`, `Method::map_string`
and `Status::map_string`, which now carry a third anchor each (the map's, the generator's and
the function's), in `llhttp.rs`, as batch 3 did for the generators. So the rule should also
cover a function whose body only maps an enum's value to what an existing item of the safe API
already returns for it, and say that a third or later anchor on one item is expected.

## Destination

The spec: `porting-symbols` § Functions (the translator's to evolve), and the
conventions' Anchors § Several stand-ins, which should name the case.

## Change

Not made: no review run was available to this batch. In `porting-symbols` § Functions,
after the first paragraph:

> A function whose body only reads or writes one member, or returns a value an existing
> item of the safe API already gives, is ported as that item: the accessor or method
> carries the function's `Replaces` anchor and alias beside its own, wherever it is
> homed, and the port writes no second API. Its export, where remaining C calls it, is
> written in its own module's gateway, over that item.

And in the conventions' § 3.2, beside the symbol alias: "and the existing accessor or
method a ported function is, as `porting-symbols`' Functions section describes".

## Validation

None run: the rule is the one this batch followed; a rerun under it would produce the
same port.

## Review

None yet.
