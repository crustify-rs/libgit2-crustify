To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: api.c's export gateway justifies its parser handles by more than C promises

Hi Marius,

This is an incorrect safety justification, not a reachable defect, so it can be handled in
public.

In `src/libgit2/deps/llhttp/api.rs` (commit b3d6b9b37), the gateway under
`ffi-export-deps-llhttp-api-c` builds each export's parser handle with `parser_mut`, whose
`// SAFETY:` comment says the parser's `settings` and `data` meet `set_settings_ptr`'s and
`set_data`'s contracts, "which is what C's own callers of `llhttp_execute` promise". Those
contracts now ask that the table's callbacks be sound at any time while the parser holds it.
libgit2's HTTP client does not keep that: it leaves its static table installed and sets
`data` to NULL between calls (`transports/httpclient.c:1283`, `1473`, `1529`, `1565`),
and its callbacks dereference `data` unconditionally (`httpclient.c:163`).

The exports are sound all the same, since each handle lives for its export's call alone,
during which C keeps its table and context. The fix rests the comment, and `table()`'s, on
that confinement; no code changes. It is on crustify/waves/wave-194, with the report under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6/contracts/ffi_export/`.
I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
