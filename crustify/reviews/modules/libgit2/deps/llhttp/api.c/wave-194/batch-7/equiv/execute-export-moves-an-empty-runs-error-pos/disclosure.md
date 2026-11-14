To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: llhttp_execute's export loses an empty run's error position

Hi Marius,

This is a behaviour defect in a gateway of the port, with no memory-safety impact in the
crate, so it can be handled in public.

api.c's `llhttp_execute(parser, data, len)` runs llhttp's parser over `data..data + len`. When
the run is empty and closes a span the previous run left open, and the span's callback refuses
it, llhttp stores `data + len` as the error position, which `llhttp_get_error_pos` returns and
`git_http_parser_execute` subtracts `data` from. The Rust export under
`ffi-export-deps-llhttp-api-c` (`src/libgit2/deps/llhttp/api.rs`) read a run of 0 bytes as
`&[]`, whose address is `0x1`:

```rust
let bytes: &[u8] = if len == 0 { &[] } else { unsafe { slice::from_raw_parts(data.cast::<u8>(), len) } };
```

A reproducer that runs the export beside api.c's body, with the exports linked in place of
`api.c.o`, fails:

```text
error_pos: export 0x1, api.c's body 0x7fffefa54091, data 0x7fffefa54091
git_http_parser_execute's parsed_len: export 18446603336495578992, api.c's body 0
```

libgit2's own C does not hit it, since it calls `llhttp_finish` for an empty run, but the
export keeps C's contract for every caller.

The fix reads any non-NULL `data` as the `len` bytes there, so the parser sees
`data..data + len` as in C, and a gateway unit test checks it. It is on
crustify/waves/wave-194, with the report under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-7/equiv/`. I can send the
reproducer or the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
