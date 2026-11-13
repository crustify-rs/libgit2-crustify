//! Unit tests for `deps/llhttp/llhttp.c`'s types, bound ahead of their consumers: the four
//! enums' conversions and their errors, and the parser [`Parser`] through its handles, decided
//! by Rust assertions.
//!
//! None calls C, so all run under Miri.

use std::collections::HashSet;
use std::error::Error as _;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Errno, ErrnoT, HttpParser, InternalT, LlhttpT, Method, MethodT, Parser, ParserMut, ParserRef,
    ParserType, ParserTypeT, Status, StatusT, UnnamedErrno, UnnamedMethod, UnnamedParserType,
    UnnamedStatus,
};
use libgit2_crustify::sys::libgit2 as sys;

/// The values in `0..=limit` that `from_raw` names, each checked to convert back to itself.
fn named<T: Copy>(limit: u32, from_raw: fn(u32) -> Option<T>, raw: fn(T) -> u32) -> Vec<u32> {
    (0..=limit)
        .chain([u32::MAX])
        .filter(|&value| {
            from_raw(value).is_some_and(|variant| {
                assert_eq!(raw(variant), value);
                true
            })
        })
        .collect()
}

#[test]
fn errno_names_the_36_codes_of_llhttp_errno() {
    let codes = named(1024, Errno::from_raw, Errno::raw);
    // `HPE_OK` .. `HPE_CB_CHUNK_EXTENSION_VALUE_COMPLETE`: `0..=35`, none missing.
    assert_eq!(codes, (0..=35).collect::<Vec<_>>());
    assert_eq!(Errno::from_raw(sys::HPE_OK), Some(Errno::Ok));
    assert_eq!(Errno::from_raw(21), Some(Errno::Paused));
    assert_eq!(Errno::from_raw(22), Some(Errno::PausedUpgrade));
    assert_eq!(Errno::from_raw(24), Some(Errno::User));
    assert_eq!(Errno::from_raw(25), Some(Errno::CrExpected));
    assert_eq!(Errno::from_raw(31), Some(Errno::CbReset));
    assert_eq!(Errno::InvalidEofState.raw(), 14);
    assert_eq!(Errno::CbChunkExtensionValueComplete.raw(), 35);
}

#[test]
fn method_names_the_47_methods_of_llhttp_method() {
    let codes = named(1024, Method::from_raw, Method::raw);
    assert_eq!(codes, (0..=46).collect::<Vec<_>>());
    assert_eq!(Method::from_raw(0), Some(Method::Delete));
    assert_eq!(Method::from_raw(1), Some(Method::Get));
    assert_eq!(Method::from_raw(5), Some(Method::Connect));
    assert_eq!(Method::from_raw(24), Some(Method::MSearch));
    assert_eq!(Method::from_raw(34), Some(Method::Pri));
    assert_eq!(Method::from_raw(41), Some(Method::GetParameter));
    assert_eq!(Method::Query.raw(), 46);
}

#[test]
fn status_names_the_99_codes_of_llhttp_status_by_their_value() {
    let codes = named(1024, Status::from_raw, Status::raw);
    assert_eq!(codes.len(), 99);
    assert_eq!(codes.first(), Some(&100));
    assert_eq!(codes.last(), Some(&599));
    for code in [100, 101, 200, 204, 301, 304, 404, 418, 500, 599] {
        assert!(codes.contains(&code), "{code}");
    }
    for code in [0, 1, 99, 104, 209, 309, 427, 600, 999] {
        assert!(!codes.contains(&code), "{code}");
    }
    assert_eq!(Status::from_raw(200), Some(Status::Ok));
    assert_eq!(Status::from_raw(418), Some(Status::ImATeapot));
    assert_eq!(Status::from_raw(524), Some(Status::TimeoutOccured));
    assert_eq!(Status::NotFound.raw(), 404);
}

#[test]
fn parser_type_names_both_request_and_response() {
    assert_eq!(
        named(1024, ParserType::from_raw, ParserType::raw),
        [0, 1, 2]
    );
    assert_eq!(ParserType::from_raw(0), Some(ParserType::Both));
    assert_eq!(ParserType::from_raw(1), Some(ParserType::Request));
    assert_eq!(ParserType::from_raw(2), Some(ParserType::Response));
}

#[test]
fn enums_convert_to_and_from_their_c_integer() {
    assert_eq!(Errno::try_from(sys::HPE_PAUSED), Ok(Errno::Paused));
    assert_eq!(sys::llhttp_errno::from(Errno::User), sys::HPE_USER);
    assert_eq!(Method::try_from(sys::HTTP_POST), Ok(Method::Post));
    assert_eq!(sys::llhttp_method::from(Method::Head), sys::HTTP_HEAD);
    assert_eq!(Status::try_from(sys::HTTP_STATUS_OK), Ok(Status::Ok));
    assert_eq!(sys::llhttp_status::from(Status::Gone), 410);
    assert_eq!(
        ParserType::try_from(sys::HTTP_RESPONSE),
        Ok(ParserType::Response)
    );
    assert_eq!(
        sys::llhttp_type::from(ParserType::Request),
        sys::HTTP_REQUEST
    );
    // The typedefs' names are the enums.
    let errno: ErrnoT = Errno::Ok;
    let method: MethodT = Method::Get;
    let status: StatusT = Status::Continue;
    let parser_type: ParserTypeT = ParserType::Both;
    assert_eq!(
        (errno, method, status, parser_type),
        (Errno::Ok, Method::Get, Status::Continue, ParserType::Both)
    );
}

#[test]
fn unnamed_values_are_refused_with_their_value() {
    let err = Errno::try_from(36).unwrap_err();
    let _: UnnamedErrno = err;
    assert_eq!(err.raw(), 36);
    assert_eq!(err.to_string(), "llhttp errno 36 is not named by llhttp.h");
    assert!(err.source().is_none());

    let err = Method::try_from(47).unwrap_err();
    assert_eq!(err.raw(), 47);
    assert_eq!(err.to_string(), "llhttp method 47 is not named by llhttp.h");
    let _: UnnamedMethod = err;

    let err = Status::try_from(999).unwrap_err();
    assert_eq!(err.raw(), 999);
    assert_eq!(
        err.to_string(),
        "llhttp status 999 is not named by llhttp.h"
    );
    let _: UnnamedStatus = err;

    let err = ParserType::try_from(3).unwrap_err();
    assert_eq!(err.raw(), 3);
    assert_eq!(
        err.to_string(),
        "llhttp parser type 3 is not named by llhttp.h"
    );
    let _: UnnamedParserType = err;

    // Errors of one value are equal, and hash alike.
    let set: HashSet<_> = [
        Status::try_from(1).unwrap_err(),
        Status::try_from(1).unwrap_err(),
    ]
    .into();
    assert_eq!(set.len(), 1);
}

#[test]
fn a_zeroed_parser_reads_as_zero() {
    let parser = Parser::zeroed();
    let p = parser.as_ref();
    assert_eq!(p.parser_type(), Ok(ParserType::Both));
    assert_eq!(p.parser_type_raw(), 0);
    assert_eq!(p.error(), Ok(Errno::Ok));
    assert_eq!(p.error_raw(), 0);
    assert_eq!(p.method(), Ok(Method::Delete));
    assert_eq!(p.method_raw(), 0);
    assert_eq!(p.reason(), None);
    assert_eq!(p.error_offset(b"HTTP/1.1"), None);
    assert_eq!(p.error_offset(&[]), None);
    assert!(p.data().is_null());
    assert!(p.settings_ptr().is_null());
    assert_eq!(p.content_length(), 0);
    assert_eq!((p.http_major(), p.http_minor()), (0, 0));
    assert_eq!(p.status_code(), 0);
    assert!(!p.upgrade());
    assert_eq!(p.finish_raw(), 0);
    assert_eq!(p.flags_raw(), 0);
    assert_eq!(p.lenient_flags_raw(), 0);
}

#[test]
fn the_parser_has_its_c_layout_and_its_typedefs_name_it() {
    assert_eq!(size_of::<Parser>(), size_of::<sys::llhttp__internal_s>());
    assert_eq!(align_of::<Parser>(), align_of::<sys::llhttp__internal_s>());
    let a: InternalT = Parser::zeroed();
    let b: LlhttpT = a;
    let c: HttpParser = b;
    assert_eq!(c.as_ref().status_code(), 0);
}

#[test]
fn each_setter_writes_what_its_getter_reads() {
    let mut parser = Parser::zeroed();
    let mut m: ParserMut<'_> = parser.as_mut();

    m.set_parser_type(ParserType::Response);
    m.set_error(Errno::Paused);
    m.set_reason(Some(c"Paused"));
    m.set_lenient_flags_raw(0x0201);
    m.set_upgrade(true);
    m.set_finish_raw(2);
    m.set_flags_raw(0x0228);

    let r = m.as_ref();
    assert_eq!(r.parser_type(), Ok(ParserType::Response));
    assert_eq!(r.parser_type_raw(), 2);
    assert_eq!(r.error(), Ok(Errno::Paused));
    assert_eq!(r.error_raw(), 21);
    assert_eq!(r.reason(), Some(c"Paused"));
    assert_eq!(r.lenient_flags_raw(), 0x0201);
    assert!(r.upgrade());
    assert_eq!(r.finish_raw(), 2);
    assert_eq!(r.flags_raw(), 0x0228);
    // The fields no setter writes are untouched.
    assert_eq!(r.method_raw(), 0);
    assert_eq!(r.status_code(), 0);
    assert_eq!(r.content_length(), 0);
    assert!(r.data().is_null());
    assert!(r.settings_ptr().is_null());

    // Each setter replaces what it set before.
    m.set_parser_type(ParserType::Request);
    m.set_error(Errno::Ok);
    m.set_reason(None);
    m.set_lenient_flags_raw(0);
    m.set_upgrade(false);
    m.set_finish_raw(0);
    m.set_flags_raw(u16::MAX);
    let r = parser.as_ref();
    assert_eq!(r.parser_type(), Ok(ParserType::Request));
    assert_eq!(r.error(), Ok(Errno::Ok));
    assert_eq!(r.reason(), None);
    assert_eq!(r.lenient_flags_raw(), 0);
    assert!(!r.upgrade());
    assert_eq!(r.finish_raw(), 0);
    assert_eq!(r.flags_raw(), u16::MAX);
}

#[test]
fn every_parser_type_and_errno_round_trips_through_its_field() {
    let mut parser = Parser::zeroed();
    for raw in 0..=2 {
        let parser_type = ParserType::from_raw(raw).unwrap();
        parser.as_mut().set_parser_type(parser_type);
        assert_eq!(parser.as_ref().parser_type(), Ok(parser_type));
        assert_eq!(u32::from(parser.as_ref().parser_type_raw()), raw);
    }
    for raw in 0..=35 {
        let errno = Errno::from_raw(raw).unwrap();
        parser.as_mut().set_error(errno);
        assert_eq!(parser.as_ref().error(), Ok(errno));
        assert_eq!(parser.as_ref().error_raw(), i32::try_from(raw).unwrap());
    }
}

#[test]
fn shared_handles_copy_and_read_the_same_parser() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_reason(Some(c"Invalid EOF state"));
    let a: ParserRef<'_> = parser.as_ref();
    let b = a;
    assert_eq!(a.reason(), b.reason());
    // A reason outlives the handle it was read through, for the parser's borrow.
    let reason = {
        let handle = parser.as_ref();
        handle.reason()
    };
    assert_eq!(reason, Some(c"Invalid EOF state"));
}

#[test]
fn a_parser_moves_with_its_state() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_error(Errno::User);
    let moved = Box::new(parser);
    assert_eq!(Parser::as_ref(&moved).error(), Ok(Errno::User));
}
