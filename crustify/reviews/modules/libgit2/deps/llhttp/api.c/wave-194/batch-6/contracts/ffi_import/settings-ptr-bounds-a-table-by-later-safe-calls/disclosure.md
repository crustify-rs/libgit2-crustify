To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: set_settings_ptr and set_data bound their obligations by parses the safe dispatchers outlast

Hi Marius,

This is an imprecise safety contract in the crate's unsafe API. Only a caller of an unsafe
setter can reach it, so it can be handled in public.

In `src/libgit2/deps/llhttp/llhttp.rs` (commit b3d6b9b37), `ParserMut::set_settings_ptr`
asks for a table that stays live "for as long as the parser executes with it", and
`set_data` for a context valid "while the parser executes with those settings". The same
commit made api.c's 23 dispatchers safe methods (`ParserMut::on_url` and the others), which
read the table and call its callbacks at any time. A caller who keeps the table for every
parse and frees it afterwards honours the wording, and safe code then reads the freed table:

```rust
unsafe { parser.as_mut().set_settings_ptr(table) }; // a Box'd C-style table
parser.as_mut().parse(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
drop(unsafe { Box::from_raw(table) });             // done executing
parser.as_mut().on_message_complete();             // safe
```

```text
==ERROR: AddressSanitizer: heap-use-after-free on address 0x76f4d3fe0098
READ of size 8 at 0x76f4d3fe0098 thread T0
    #0 in <…::llhttp::api::EventSlot>::read src/libgit2/deps/llhttp/api.rs:976:48
    #3 in <…::ParserMut>::on_message_complete src/libgit2/deps/llhttp/api.rs:814:26
```

The fix states both obligations over what the parser holds: the table, and the context beside
it, until a setter replaces the table or `reinit` clears it (`reset` keeps it), or else for
as long as the parser lives. Two UB tests call the dispatchers outside any parse within the
new contracts. Both are on crustify/waves/wave-194; the reproducer is under
`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6/contracts/ffi_import/`.
I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
