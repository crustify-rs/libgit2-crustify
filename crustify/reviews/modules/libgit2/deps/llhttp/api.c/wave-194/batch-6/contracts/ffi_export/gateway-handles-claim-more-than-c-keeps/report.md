# The gateway's handles claim of C's parsers more than C keeps

- **finding**: `contracts/ffi_export/gateway-handles-claim-more-than-c-keeps`
- **batch**: `wave-194/batch-6/run-1`, the `symbol` review of `deps/llhttp/api.c`'s folded
  waves
- **items**: api.c's exports under `ffi-export-deps-llhttp-api-c` (`fn::llhttp_execute`,
  `fn::llhttp_finish`, the 23 `fn::llhttp__on_<slot>` and the others built on `parser_mut`),
  and `ffi_import`'s `ParserMut::table`, which the dispatchers read
- **affected commit**: `b3d6b9b37` (`parser_mut` since `2a51ccb3f`)
- **files**: `src/libgit2/deps/llhttp/api.rs`

## Finding

Every export but `llhttp_get_error_pos` and `llhttp_get_error_reason` builds its parser's
exclusive handle with `parser_mut`, whose `// SAFETY:` comment vouches for the handle's
invariant on this ground:

> its `settings` and `data` meet `set_settings_ptr`'s and `set_data`'s contracts, which is
> what C's own callers of `llhttp_execute` promise

and `table()`'s comment counts "C's callers of api.c's functions with their own tables" among
those that meet `set_settings_ptr`'s contract "for as long as the parser holds it". Since
`b3d6b9b37`, those contracts ask that the table's callbacks be sound with the parser's `data`
*at any time* while the parser holds the table, since the dispatchers are safe methods. C's
callers promise no such thing. libgit2's HTTP client, the only C that calls these exports
with a table, installs a `static` table once (`reset_parser`, `llhttp_init`,
`c/src/libgit2/transports/httpclient.c:917-922`), points `data` at a stack
`http_parser_context` only around its own parses, and sets it NULL again before returning
(`httpclient.c:1271`/`1283`, `1458`/`1473`, `1503`/`1529`, `1548`/`1565`). Its callbacks
dereference `data` unconditionally (`on_header_complete`, `httpclient.c:161-165`, and the
others at `228`, `263`, `351`, `395`, `417`). Between those calls a dispatcher would follow
NULL.

The exports are nonetheless sound: each builds its handle for its own call, uses it for that
call's one operation and drops it before returning, and during the call C's caller keeps its
table and its context, as llhttp asks of it. So the invariant the comments claim does not
hold, but the one the code needs does. A justification resting on what C does not promise is
a contracts finding; no reproducer is needed, since nothing unsound is reachable.

## Fix

In this batch's second commit, `parser_mut`'s `// SAFETY:` comment rests the handle's
invariant on its confinement: the handle's lifetime is the export's call, for which C's
caller keeps its table and the context its callbacks read, and no longer, as
`httpclient.c`'s NULL `data` between calls shows, so no handle over a C parser may outlive
the call that builds it. `table()`'s comment says the same of a handle the gateway builds.
The code does not change.
