# An export whose C declaration no header holds has no raw binding to be checked against

Status: `open` (the workaround `wave-194/batch-4/run-1` took; the spec change is not made).

## Signal

A rule an agent works around (`evolution` §2). `testing-gates`' Regressions section has a
`port` batch check "each export … at compile time against the raw binding of the C
declaration it replaces", and `export-signature-check` (crustify `dfd557c`, awaiting review)
has the binding "allowlisted for it where the bindings lack it". `bindgen`'s Extending
section lets a batch add to `wrapper.h` only "an internal header … when one of them is
declared there". Neither covers a function the remaining C calls through a prototype it
writes in its own translation unit, which no header holds: bindgen never sees it, so it
has no raw binding, the export cannot be checked against one, and an equivalence test cannot
call its C through one.

## Evidence

`wave-194/batch-4/run-1` (`deps/llhttp/api.c`, symbol route). api.c defines 23 dispatchers,
`int llhttp__on_<slot>(llhttp_t* s, const char* p, const char* endp)`, which `llhttp.c`'s
generated parser calls through prototypes it declares itself, `int
llhttp__on_<slot>(llhttp__internal_t* s, const unsigned char* p, const unsigned char*
endp);` (`c/deps/llhttp/llhttp.c`, lines 569–1144); `llhttp.h` declares none of them. The
batch exported all 23 under `ffi-export-deps-llhttp-api-c` and

- checked each against a function-pointer type written by hand in the gateway from
  `llhttp.c`'s prototype (`type Dispatcher = unsafe extern "C" fn(*mut
  sys::llhttp__internal_t, *const c_uchar, *const c_uchar) -> c_int;`, then `let _:
  Dispatcher = llhttp__on_url;` per export, `src/libgit2/deps/llhttp/api.rs`), which catches
  an export that drifts from the others but not from C, since the type is the batch's own
  transcription; and
- compared the Rust dispatchers with api.c's in `tests/equiv/deps/llhttp/api.rs` through an
  `unsafe extern "C"` block of the 23 prototypes, written by hand in the test, outside the
  raw bindings.

## Destination

The spec: `bindgen`'s Extending section, and `porting-symbols`' Functions section, which
the translator's role evolves, where `export-signature-check` describes the check.

## Change

Not made: it touches `bindgen`'s layout, which is not the translator's, and wants its own
review. Two candidates:

- each link unit's bindings gain a hand-written header, `sys/<link-unit>/declarations.h`,
  included by `wrapper.h`, that holds a copy of each prototype the remaining C declares only
  in a translation unit, with the file and line it copies, for the functions a batch exports
  or compares; the allowlist then reaches them as any other function, the export is checked
  against that binding, and the header loses each prototype with the export's
  `port/unexport`; or
- `porting-symbols` states the workaround as the rule: the export is checked against a
  function-pointer type transcribed beside the gateway from the caller's prototype, citing
  it, and an equivalence test declares the C it calls in an `extern` block of its own,
  citing the same.

The first keeps the check against C's own text, which a C change to the prototype would
break at the next regeneration, as the rule intends; the second needs nothing new but
checks only a transcription. The first is recommended.

## Validation

None run.

## Review

None yet.
