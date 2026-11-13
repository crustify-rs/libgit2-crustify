To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: ParserMut::set_settings_ptr admits a callback reason that the safe ParserRef::reason reads after it is freed

Hi Marius,

This is an incomplete safety contract in the crate's unsafe API. Only a caller of an unsafe
setter can reach it, so it can be handled in public.

In `src/libgit2/deps/llhttp/llhttp.rs` (commit 1ead111ee), `ParserMut::set_settings_ptr`
asks for a table "that stays live and unchanged, and whose callbacks are sound to call with
this parser and its `data`, for as long as the parser executes with it". llhttp lets a
callback set any string as the parser's error reason (`llhttp_set_error_reason`, "called in
user callbacks right before returning the errno"). Nothing asks how long that string lives,
and the safe `ParserRef::reason` follows it at any later time. A callback that sets a reason
its context owns, followed by a caller that frees the context once the parser is done
executing, honours every word of the contract:

```rust
unsafe { parser.as_mut().set_settings_ptr(&raw const SETTINGS) }; // on_url sets ctx's reason
unsafe { parser.as_mut().set_data(ctx.cast()) };
parser.as_mut().parse(b"GET /refused HTTP/1.1\r\n").unwrap_err();
unsafe { parser.as_mut().set_settings_ptr(ptr::null()) };
unsafe { parser.as_mut().set_data(ptr::null_mut()) };
drop(unsafe { Box::from_raw(ctx) });
parser.as_ref().reason(); // safe
```

```text
==ERROR: AddressSanitizer: heap-use-after-free on address 0x700666de0010
READ of size 2 at 0x700666de0010 thread T0
    #3 in <core::ffi::c_str::CStr>::from_ptr
    #4 in <…::llhttp::ffi_import::ParserRef>::reason src/libgit2/deps/llhttp/llhttp.rs:1862:27
```

`Callback::from_fn`'s contract is silent on the reason too, though no caller passes it a C
function yet.

The fix makes the obligations explicit. `set_settings_ptr` now asks that a reason a callback
sets stay live and unchanged while the parser holds it, and `from_fn` that it live as long as
the program. A UB test reads such a reason after the parse and gives it up before its
context goes. Both are on the review branch (crustify/waves/wave-194). The reproducer is
under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-5/contracts/ffi_import/`.
I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
