//! `ParserMut::set_settings_ptr`'s `# Safety` asks for a table that "stays live and
//! unchanged, and whose callbacks are sound to call with this parser and its `data`, for as
//! long as the parser executes with it", and `set_data`'s for a `data` that is what those
//! callbacks expect "while the parser executes with those settings". Both bound the caller's
//! obligation by when the parser *executes*, which `wave-194/batch-4` widened to every call
//! of api.c's dispatchers, now the parser's safe methods (`ParserMut::on_url` and the
//! others). A caller who keeps the table live for every parse and finish it runs, and frees
//! it once the parser is done executing, honours that wording; any safe code that later
//! holds the parser then calls a dispatcher, which reads the freed table, though no
//! `unsafe` block is written after the table goes.
//!
//! The table's address stays in the parser until it is replaced or the parser initialised
//! again, so what the dispatchers rely on is that the table lives *while the parser holds
//! it*, as `table()`'s `// SAFETY:` comment in `src/libgit2/deps/llhttp/api.rs` states it.
//!
//! Run under the `asan-ubsan` variant: AddressSanitizer reports a heap use after free.

use std::ffi::c_int;

use libgit2::libgit2::deps::llhttp::llhttp::{Parsed, Parser, ParserMut};
use libgit2::sys::libgit2 as sys;

/// An `on_message_complete` as C writes one: it counts messages in the `u32` its parser's
/// `data` addresses.
///
/// # Safety
///
/// `parser` is a live parser whose `data` addresses a live `u32` nothing else accesses
/// during the call.
unsafe extern "C" fn count(parser: *mut sys::llhttp_t) -> c_int {
    // SAFETY: per this function's contract.
    unsafe { *(*parser).data.cast::<u32>() += 1 };
    0
}

/// Code that holds a parser and knows nothing of its table: it signals the end of a message
/// through the parser's safe dispatcher, as `llhttp.c`'s port will.
fn safe_code(mut parser: ParserMut<'_>) -> c_int {
    parser.on_message_complete()
}

fn main() {
    let mut messages = 0u32;
    // Every other slot NULL, as `llhttp_settings_init` leaves a table.
    let table = Box::into_raw(Box::new(sys::llhttp_settings_t {
        on_message_complete: Some(count),
        ..Default::default()
    }));
    let mut parser = Parser::new();
    // SAFETY: `messages` is the `u32` the callback reads, live for the rest of `main`.
    unsafe { parser.as_mut().set_data((&raw mut messages).cast()) };
    // SAFETY: the table is live and unchanged, and its one callback sound with this parser
    // and its `data`, for as long as the parser executes with it: the parse below, after which
    // this program runs no parse nor finish of the parser.
    unsafe { parser.as_mut().set_settings_ptr(table) };
    let response = b"HTTP/1.1 204 No Content\r\n\r\n";
    assert_eq!(parser.as_mut().parse(response), Ok(Parsed::All));
    println!("messages after the parse: {messages}");

    // The caller is done executing the parser, and frees its table.
    // SAFETY: `table` came from `Box::into_raw`, and the parser executes no more under
    // the caller.
    drop(unsafe { Box::from_raw(table) });

    // Safe code: the dispatcher reads the freed table.
    println!("dispatched: {}", safe_code(parser.as_mut()));
}
