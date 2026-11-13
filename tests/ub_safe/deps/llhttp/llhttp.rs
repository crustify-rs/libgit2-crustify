//! UB tests for `deps/llhttp/llhttp.c`'s parser on the safe API: every shared and exclusive
//! access path of [`Parser`], its reborrows, the reason it lends and the error position it
//! compares, with inputs at the edges of the address space a slice can take.
//!
//! The illegal paths, two exclusive handles, a shared one beside an exclusive one, a handle
//! or a reason outliving the parser and a reason that is not `'static`, are `Parser`'s,
//! `ParserRef::reason`'s and `ParserMut::set_reason`'s compile-fail doctests. Nothing here
//! calls C, so every test also runs under Miri.

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Errno, Parser, ParserMut, ParserType};

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
