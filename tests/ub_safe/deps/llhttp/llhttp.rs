//! UB tests for `deps/llhttp/llhttp.c`'s parser on the safe API: every shared and exclusive
//! access path of [`Parser`], its reborrows, the reason it lends and the error position it
//! compares, with inputs at the edges of the address space a slice can take.
//!
//! The parse and the two callback types: parsers parsed, moved and parsed again, inputs
//! freed between runs, runs that end inside a span and at the edges a slice can take, and
//! callbacks called with every handle, whose handlers parse, initialise, panic and cross
//! threads.
//!
//! The illegal paths, two exclusive handles, a shared one beside an exclusive one, a handle
//! or a reason outliving the parser and a reason that is not `'static`, are `Parser`'s,
//! `ParserRef::reason`'s and `ParserMut::set_reason`'s compile-fail doctests, and a handler
//! sending its parser to another thread `EventHandler`'s. The tests that parse call C, and
//! are ignored under Miri; the rest runs there.

use std::ffi::c_int;
use std::thread;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Parsed, Parser, ParserMut, ParserType,
};

/// Writes every field a setter reaches through `m`, from `seed`.
fn write_all(m: &mut ParserMut<'_>, seed: u8) {
    m.set_parser_type(if seed.is_multiple_of(2) {
        ParserType::Request
    } else {
        ParserType::Response
    });
    m.set_error(Errno::from_raw(u32::from(seed % 36)).unwrap());
    m.set_reason(if seed.is_multiple_of(3) {
        None
    } else {
        Some(c"reason")
    });
    m.set_lenient_flags_raw(u16::from(seed) << 2);
    m.set_upgrade(seed.is_multiple_of(5));
    m.set_finish_raw(seed % 3);
    m.set_flags_raw(u16::from(seed).rotate_left(7));
}

#[test]
fn reborrows_alternate_with_shared_reads() {
    let mut parser = Parser::zeroed();
    for seed in 0..=u8::MAX {
        let mut m = parser.as_mut();
        write_all(&mut m.as_mut(), seed);
        // A shared reborrow of the exclusive handle reads what it wrote.
        let r = m.as_ref();
        assert_eq!(r.error_raw(), i32::from(seed % 36));
        assert_eq!(r.reason().is_some(), seed % 3 != 0);
        // Then a fresh shared handle, once the exclusive one is gone.
        let r = parser.as_ref();
        assert_eq!(r.flags_raw(), u16::from(seed).rotate_left(7));
        assert_eq!(r.upgrade(), seed.is_multiple_of(5));
    }
}

#[test]
fn a_reason_read_before_it_is_replaced_stays_valid() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_reason(Some(c"first"));
    let first = parser.as_ref().reason().unwrap();
    // `first` borrows the parser, so the parser is read, not written, while it lives.
    assert_eq!(first.to_bytes(), b"first");
    parser.as_mut().set_reason(Some(c"second"));
    parser.as_mut().set_reason(None);
    assert_eq!(parser.as_ref().reason(), None);
}

#[test]
fn error_offset_without_an_error_position_is_none_for_any_input() {
    let parser = Parser::zeroed();
    let heap = vec![0_u8; 4096];
    let inputs: [&[u8]; 5] = [&[], b"x", &heap, &heap[4095..], &heap[..0]];
    for input in inputs {
        assert_eq!(parser.as_ref().error_offset(input), None);
    }
}

#[test]
fn parsers_in_a_vec_are_written_and_read_independently() {
    let mut parsers: Vec<Parser> = (0..64).map(|_| Parser::zeroed()).collect();
    for (seed, parser) in (0_u8..).zip(&mut parsers) {
        write_all(&mut parser.as_mut(), seed);
    }
    // Growing the vector moves every parser, which carries its state.
    parsers.reserve(1024);
    for (seed, parser) in (0_u8..).zip(&parsers) {
        assert_eq!(parser.as_ref().lenient_flags_raw(), u16::from(seed) << 2);
        assert_eq!(parser.as_ref().finish_raw(), seed % 3);
    }
}

// --- Parsing ---------------------------------------------------------------------------

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nX-Long: 0123456789\r\nContent-Length: 6\r\n\r\nbodies";

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn each_run_may_be_freed_before_the_next() {
    // The parser keeps pointers into a run it ends inside a span, the header's value here,
    // and its error position: neither is followed once the run is gone.
    let mut parser = Parser::new();
    for byte in RESPONSE {
        let run = vec![*byte];
        assert_eq!(parser.as_mut().parse(&run), Ok(Parsed::All));
        drop(run);
    }
    assert_eq!(parser.as_ref().status_code(), 200);
    let bad = b"garbage".to_vec();
    assert!(parser.as_mut().parse(&bad).is_err());
    drop(bad);
    assert_eq!(parser.as_ref().error_offset(&[0; 64]), None);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parsers_move_between_runs() {
    let mut parsers: Vec<Parser> = (0..32).map(|_| Parser::new()).collect();
    let (head, body) = RESPONSE.split_at(RESPONSE.len() - 3);
    for parser in &mut parsers {
        assert_eq!(parser.as_mut().parse(head), Ok(Parsed::All));
    }
    // Growing the vector moves every parser mid-message; C kept no pointer to any.
    parsers.reserve(4096);
    let mut moved: Vec<Parser> = parsers.into_iter().rev().collect();
    for parser in &mut moved {
        assert_eq!(parser.as_mut().parse(body), Ok(Parsed::All));
        assert_eq!(parser.as_ref().content_length(), 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn runs_at_the_edges_of_their_allocations() {
    let heap = RESPONSE.to_vec().into_boxed_slice();
    let mut parser = Parser::new();
    // An empty run at the very end of an allocation, then one at its start.
    assert_eq!(parser.as_mut().parse(&heap[heap.len()..]), Ok(Parsed::All));
    assert_eq!(parser.as_mut().parse(&heap[..0]), Ok(Parsed::All));
    for i in 0..heap.len() {
        assert_eq!(parser.as_mut().parse(&heap[i..=i]), Ok(Parsed::All));
    }
    assert_eq!(parser.as_ref().status_code(), 200);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_handle_reborrowed_to_parse_reads_its_results() {
    let mut parser = Parser::new();
    let mut m = parser.as_mut();
    assert_eq!(m.as_mut().parse(RESPONSE), Ok(Parsed::All));
    assert_eq!(m.as_ref().status_code(), 200);
    m.reinit();
    assert_eq!(m.as_ref().status_code(), 0);
    assert_eq!(m.parse(b"HTTP/1.0 404 Not Found\r\n\r\n"), Ok(Parsed::All));
    assert_eq!(parser.as_ref().http_minor(), 0);
}

// --- Callbacks -------------------------------------------------------------------------

/// Parses the parser it is handed, which is not being parsed when [`Callback::call`] calls
/// it.
struct ParsingItsParser;

impl EventHandler for ParsingItsParser {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        match parser.parse(b"HTTP/1.1 204 No Content\r\n\r\n") {
            Ok(Parsed::All) => 0,
            _ => -1,
        }
    }
}

/// Initialises the parser it is handed, and parses another of its own.
struct InitialisingItsParser;

impl EventHandler for InitialisingItsParser {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.reinit();
        let mut other = Parser::new();
        match other.as_mut().parse(RESPONSE) {
            Ok(Parsed::All) => 0,
            _ => -1,
        }
    }
}

/// Reads every byte of the run it is handed.
struct Summing;

impl DataHandler for Summing {
    fn on_data(parser: ParserMut<'_>, data: &[u8]) -> c_int {
        let sum: u32 = data.iter().map(|&b| u32::from(b)).sum();
        c_int::try_from(sum).unwrap() + c_int::from(parser.as_ref().upgrade())
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_handler_called_from_rust_may_parse_and_initialise_its_parser() {
    let mut parser = Parser::new();
    assert_eq!(Callback::new::<ParsingItsParser>().call(parser.as_mut()), 0);
    assert_eq!(parser.as_ref().status_code(), 204);
    assert_eq!(
        Callback::new::<InitialisingItsParser>().call(parser.as_mut()),
        0
    );
    assert_eq!(parser.as_ref().status_code(), 0);
}

#[test]
fn a_data_callback_reads_runs_at_the_edges_of_their_allocations() {
    let callback = DataCallback::new::<Summing>();
    let heap = vec![1_u8; 256].into_boxed_slice();
    let mut parser = Parser::zeroed();
    assert_eq!(callback.call(parser.as_mut(), &heap[256..]), 0);
    assert_eq!(callback.call(parser.as_mut(), &heap[..0]), 0);
    assert_eq!(callback.call(parser.as_mut(), &heap[255..]), 1);
    parser.as_mut().set_upgrade(true);
    assert_eq!(callback.call(parser.as_mut(), &heap), 257);
}

#[test]
fn callbacks_cross_threads_and_run_with_each_threads_parser() {
    let event = Callback::new::<InitialisingItsParserWithoutC>();
    let data = DataCallback::new::<Summing>();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            thread::spawn(move || {
                let mut parser = Parser::zeroed();
                parser.as_mut().set_error(Errno::User);
                assert_eq!(event.call(parser.as_mut()), 0);
                data.call(parser.as_mut(), b"ab")
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), 195);
    }
}

/// Sets the parser it is handed back to its zero type, without calling C.
struct InitialisingItsParserWithoutC;

impl EventHandler for InitialisingItsParserWithoutC {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.set_parser_type(ParserType::Both);
        parser.set_error(Errno::Ok);
        0
    }
}
