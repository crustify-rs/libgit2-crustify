To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: a ub_unsafe fixture would not link once api.c switches from C

Hi Marius,

This is a test-suite defect, with no safety impact, so it can be handled in public.

`tests/ub_unsafe/deps/llhttp/llhttp.rs`'s fixture `refuse_url` (commit f58d38aa9) stores its
reason through `sys::llhttp_set_error_reason`. api.c's port does not export that function,
since no C outside api.c calls it. With the archive lacking `api.c.o` and
`ffi-export-deps-llhttp-api-c` on, as at api.c's switch, the suite fails to link:

```text
rust-lld: error: undefined symbol: llhttp_set_error_reason
>>> referenced by llhttp.rs:292 (tests/ub_unsafe/deps/llhttp/llhttp.rs:292)
```

The fix writes the field as the C function's body does, `(*parser).reason = reason`. Every
suite but `equiv` then links against the exports on plain and ASan+UBSan. It is on
crustify/waves/wave-194, with the report under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6/misc/`. I can send the
exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
