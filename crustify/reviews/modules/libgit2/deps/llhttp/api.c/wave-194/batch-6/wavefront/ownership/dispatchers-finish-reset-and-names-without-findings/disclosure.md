To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: 28 llhttp api.c functions had no pointer findings in the ownership store

Hi Marius,

This is an incomplete analysis record, with no safety impact, so it can be handled in public.

`crustify/wavefront/ownership-store.json` held no pointer findings for `llhttp_finish`,
`llhttp_reset`, the three name functions and the 23 `llhttp__on_<slot>` dispatchers that
wave-194/batch-4 ported (commit b3d6b9b37): `wavefront … query functions --name
llhttp__on_url` printed `"submitted": false` with every `ptr` slot null.

The fix submits them: each dispatcher's parser borrowed and mutable, a span's `p` read for
`endp - p` bytes and `endp` never read, `llhttp_finish`'s and `llhttp_reset`'s parser
borrowed, and the name functions' static literal, never NULL. The dispatchers' records keep one
pending slot per parameter, since wavefront merges `llhttp.c`'s prototype into the
definition's signature. That is proposed as a wavefront change
(`crustify/evolution/wavefront-two-prototypes-one-signature`). It is on
crustify/waves/wave-194. I can send the exact submissions on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
