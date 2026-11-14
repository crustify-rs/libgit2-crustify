To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: llhttp.c's equivalence tests would unlink equiv at api.c's switch

Hi Marius,

This is a test-suite defect, with no safety impact, so it can be handled in public.

At `deps/llhttp/api.c`'s switch, `api.c.o` leaves the archives and only its 31 exports
define its symbols. The earlier link checks left the whole `equiv` binary out, on the
ground that api.c's comparisons retire with its C. But `equiv` holds every module's
equivalence tests, and `tests/equiv/deps/llhttp/llhttp.rs`, `llhttp.c`'s, also reads its
parsers through api.c's getters, its name functions and `llhttp_settings_init`, none of
which has an export. With the archive lacking `api.c.o` and the export feature on:

```text
rust-lld: error: undefined symbol: llhttp_get_type
          >>> referenced by llhttp.rs:78 (tests/equiv/deps/llhttp/llhttp.rs:78)
rust-lld: error: undefined symbol: llhttp_settings_init
          >>> referenced by llhttp.rs:250 (tests/equiv/deps/llhttp/llhttp.rs:250)
…
rust-lld: error: too many errors emitted, stopping now
```

So retiring api.c's own equivalence file alone would leave `equiv`, every module's, unable
to link.

The fix moves the nine tests that compare `llhttp.rs`'s accessors with api.c's getters into
api.c's equivalence file, since they test api.c's ported items, so that retiring that one
file is api.c's whole retirement. Five `unit_pub` tests keep what they established that no
unit test did. With the file retired in a scratch tree, the whole workspace links against the
exports and passes on plain, ASan+UBSan and TSan, but for the known nightly failures. It is on
crustify/waves/wave-194, with the report, which maps every retiring test to the unit tests
that keep it, under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-7/misc/`. I can send the
exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
