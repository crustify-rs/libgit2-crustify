//! Equivalence tests for `deps/llhttp/http.c`'s port against the C.
//!
//! Each of http.c's five functions is compared with its port over every state its body can
//! tell apart: pairs of parsers built from one description of the fields it reads, the type,
//! the flag words, the status code, the method, the version, the `Content-Length`, the
//! upgrade and the finish byte, each over the values C distinguishes and some it does not,
//! such as an unnamed type or method, an upgrade byte of 2, the unused flag bits and an
//! HTTP/2.0 version. Every combination of them is run through C on one parser and through the
//! Rust on the other: their returns are compared, [`Body`] and [`InvalidTransferEncoding`]
//! through the code `llhttp.c` switches on, and the whole parsers after the call.
//!
//! `llhttp_message_needs_eof` and `llhttp_should_keep_alive` are called through their raw
//! bindings. The three hooks have none, since no header declares them: they are declared
//! here as `llhttp.c` declares them for itself (`c/deps/llhttp/llhttp.c`, lines 711, 753 and
//! 761), as api.c's equivalence tests declared its dispatchers. These tests are retired with
//! http.c's C at the module's switch.

use std::ffi::{c_int, c_uchar};

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::http::{Body, InvalidTransferEncoding};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserMut};
use libgit2_crustify::sys::libgit2 as sys;

unsafe extern "C" {
    fn llhttp__before_headers_complete(
        s: *mut sys::llhttp__internal_t,
        p: *const c_uchar,
        endp: *const c_uchar,
    ) -> c_int;
    fn llhttp__after_headers_complete(
        s: *mut sys::llhttp__internal_t,
        p: *const c_uchar,
        endp: *const c_uchar,
    ) -> c_int;
    fn llhttp__after_message_complete(
        s: *mut sys::llhttp__internal_t,
        p: *const c_uchar,
        endp: *const c_uchar,
    ) -> c_int;
}

/// The fields http.c's functions read and write.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    parser_type: u8,
    flags: u16,
    status_code: u16,
    method: u8,
    version: (u8, u8),
    content_length: u64,
    lenient_flags: u16,
    upgrade: u8,
    finish: u8,
}

impl State {
    /// A zeroed parser with the state's fields, its others as `Parser::zeroed()` leaves them
    /// but `_index`, a marker that the call must leave as it is.
    fn parser(self) -> Parser {
        let mut parser = Parser::zeroed();
        let raw = parser.as_mut().as_mut_ptr();
        // SAFETY: `raw` addresses the live parser, which nothing else accesses until the
        // value is returned; `llhttp__internal_s` is `Copy`, its fields integers and
        // pointers, so a copy of it forms nothing it does not hold.
        let zeroed = unsafe { raw.read() };
        let state = sys::llhttp__internal_s {
            _index: 0x5A5A,
            type_: self.parser_type,
            flags: self.flags,
            status_code: self.status_code,
            method: self.method,
            http_major: self.version.0,
            http_minor: self.version.1,
            content_length: self.content_length,
            lenient_flags: self.lenient_flags,
            upgrade: self.upgrade,
            finish: self.finish,
            ..zeroed
        };
        // SAFETY: as above; every field is an integer or a pointer, for which any value is
        // valid, and the write drops nothing.
        unsafe { raw.write(state) };
        parser
    }
}

/// The parser's fields, compared as integers and addresses.
fn fields(parser: &Parser) -> impl PartialEq + std::fmt::Debug + use<> {
    // SAFETY: the handle addresses a live parser; `llhttp__internal_s` is `Copy`, its fields
    // integers and pointers, so a copy of it forms nothing it does not hold.
    let p = unsafe { parser.as_ref().as_ptr().read() };
    (
        (p._index, p._span_pos0.addr(), p._span_cb0.addr()),
        (p.error, p.reason.addr(), p.error_pos.addr()),
        (p.data.addr(), p._current.addr(), p.content_length, p.type_),
        (p.method, p.http_major, p.http_minor, p.header_state),
        (p.lenient_flags, p.upgrade, p.finish, p.flags, p.status_code),
        (p.initial_message_completed, p.settings.addr()),
    )
}

/// Every subset of `bits`.
fn subsets(bits: &[u32]) -> Vec<u16> {
    (0..1_u32 << bits.len())
        .map(|mask| {
            let set = bits
                .iter()
                .enumerate()
                .filter(|&(i, _)| mask & (1 << i) != 0)
                .fold(0, |acc, (_, &bit)| acc | bit);
            u16::try_from(set).unwrap()
        })
        .collect()
}

/// Every flag llhttp defines, and two bits it does not use.
fn all_flags() -> Vec<u16> {
    subsets(&[
        sys::F_CONNECTION_KEEP_ALIVE,
        sys::F_CONNECTION_CLOSE,
        sys::F_CONNECTION_UPGRADE,
        sys::F_CHUNKED,
        sys::F_UPGRADE,
        sys::F_CONTENT_LENGTH,
        sys::F_SKIPBODY,
        sys::F_TRAILING,
        sys::F_TRANSFER_ENCODING,
        0x100,
        0x8000,
    ])
}

/// `HTTP_BOTH`, `HTTP_REQUEST`, `HTTP_RESPONSE` and a byte no `llhttp_type` names.
const TYPES: [u8; 4] = [0, 1, 2, 7];

/// The status codes http.c tells apart, and their neighbours.
const STATUSES: [u16; 16] = [
    0, 1, 99, 100, 101, 102, 103, 104, 199, 200, 203, 204, 205, 304, 404, 999,
];

/// `HTTP_DELETE`, `HTTP_GET`, `HTTP_CONNECT`, `HTTP_OPTIONS` and a byte no `llhttp_method`
/// names.
const METHODS: [u8; 5] = [0, 1, 5, 6, 0xFF];

/// The versions http.c tells apart: 0.9, 1.0 and 1.1, and those no message names but that it
/// reads, 0.0, 0.1, 2.0 and 2.1.
const VERSIONS: [(u8, u8); 7] = [(0, 9), (1, 0), (1, 1), (0, 0), (0, 1), (2, 0), (2, 1)];

/// No upgrade, an upgrade, and a byte C reads as true.
const UPGRADES: [u8; 3] = [0, 1, 2];

/// The code `llhttp.c` switches on for `outcome`.
fn code(outcome: Result<Body, InvalidTransferEncoding>) -> c_int {
    match outcome {
        Ok(Body::Empty) => 0,
        Ok(Body::Upgrade) => 1,
        Ok(Body::Chunked) => 2,
        Ok(Body::Length) => 3,
        Ok(Body::UntilEof) => 4,
        Err(InvalidTransferEncoding) => 5,
    }
}

/// Runs `c` on one parser of `state` and `rust` on another, and compares their returns and
/// the parsers after the call; counts the comparisons in `n`.
fn compare(
    state: State,
    n: &mut usize,
    c: impl FnOnce(*mut sys::llhttp__internal_t) -> c_int,
    rust: impl FnOnce(&mut Parser) -> c_int,
) {
    let mut c_parser = state.parser();
    let mut rust_parser = state.parser();
    let c_ret = c(c_parser.as_mut().as_mut_ptr());
    let rust_ret = rust(&mut rust_parser);
    assert_eq!(c_ret, rust_ret, "{state:?}");
    assert_eq!(fields(&c_parser), fields(&rust_parser), "{state:?}");
    *n += 1;
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn message_needs_eof_is_llhttp_message_needs_eof() {
    let mut n = 0;
    for parser_type in TYPES {
        for flags in all_flags() {
            for status_code in STATUSES {
                let state = State {
                    parser_type,
                    flags,
                    status_code,
                    ..State::default()
                };
                compare(
                    state,
                    &mut n,
                    // SAFETY: `p` addresses a live parser, which nothing else accesses during
                    // the call.
                    |p| unsafe { sys::llhttp_message_needs_eof(p) },
                    |p| c_int::from(p.as_ref().message_needs_eof()),
                );
            }
        }
    }
    assert_eq!(n, 4 * 2048 * 16);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn should_keep_alive_is_llhttp_should_keep_alive() {
    let mut n = 0;
    for parser_type in TYPES {
        for flags in all_flags() {
            for status_code in [0, 100, 101, 200, 204, 304] {
                for version in VERSIONS {
                    let state = State {
                        parser_type,
                        flags,
                        status_code,
                        version,
                        ..State::default()
                    };
                    compare(
                        state,
                        &mut n,
                        // SAFETY: `p` addresses a live parser, which nothing else accesses
                        // during the call.
                        |p| unsafe { sys::llhttp_should_keep_alive(p) },
                        |p| c_int::from(p.as_ref().should_keep_alive()),
                    );
                }
            }
        }
    }
    assert_eq!(n, 4 * 2048 * 6 * 7);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn before_headers_complete_is_llhttps_hook() {
    let mut n = 0;
    let flags = subsets(&[
        sys::F_UPGRADE,
        sys::F_CONNECTION_UPGRADE,
        sys::F_CONNECTION_KEEP_ALIVE,
        sys::F_CHUNKED,
    ]);
    for parser_type in TYPES {
        for &flags in &flags {
            for status_code in STATUSES {
                for method in METHODS {
                    for upgrade in UPGRADES {
                        let state = State {
                            parser_type,
                            flags,
                            status_code,
                            method,
                            upgrade,
                            ..State::default()
                        };
                        compare(
                            state,
                            &mut n,
                            // SAFETY: `p` addresses a live parser, which nothing else accesses
                            // during the call; the hook does not read `p` and `endp`.
                            |p| unsafe {
                                llhttp__before_headers_complete(
                                    p,
                                    std::ptr::null(),
                                    std::ptr::null(),
                                )
                            },
                            |p| {
                                p.as_mut().before_headers_complete();
                                0
                            },
                        );
                    }
                }
            }
        }
    }
    assert_eq!(n, 4 * 16 * 16 * 5 * 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn after_headers_complete_is_llhttps_hook() {
    let mut n = 0;
    let flags = subsets(&[
        sys::F_CHUNKED,
        sys::F_SKIPBODY,
        sys::F_TRANSFER_ENCODING,
        sys::F_CONTENT_LENGTH,
        sys::F_UPGRADE,
    ]);
    let lenient = subsets(&[
        sys::LENIENT_CHUNKED_LENGTH,
        sys::LENIENT_TRANSFER_ENCODING,
        sys::LENIENT_HEADERS,
    ]);
    for parser_type in TYPES {
        for &flags in &flags {
            for status_code in STATUSES {
                for method in [0, 5, 0xFF] {
                    for content_length in [0, 1, u64::MAX] {
                        for &lenient_flags in &lenient {
                            for upgrade in UPGRADES {
                                let state = State {
                                    parser_type,
                                    flags,
                                    status_code,
                                    method,
                                    content_length,
                                    lenient_flags,
                                    upgrade,
                                    ..State::default()
                                };
                                compare(
                                    state,
                                    &mut n,
                                    // SAFETY: `p` addresses a live parser, which nothing else
                                    // accesses during the call; the hook does not read `p`
                                    // and `endp`.
                                    |p| unsafe {
                                        llhttp__after_headers_complete(
                                            p,
                                            std::ptr::null(),
                                            std::ptr::null(),
                                        )
                                    },
                                    |p| code(p.as_ref().after_headers_complete()),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(n, 4 * 32 * 16 * 3 * 3 * 8 * 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn after_message_complete_is_llhttps_hook() {
    let mut n = 0;
    for parser_type in TYPES {
        for flags in all_flags() {
            for status_code in [0, 100, 200, 204] {
                for version in [(1, 0), (1, 1), (2, 0)] {
                    for finish in [0, 1, 2] {
                        let state = State {
                            parser_type,
                            flags,
                            status_code,
                            version,
                            finish,
                            upgrade: 1,
                            ..State::default()
                        };
                        compare(
                            state,
                            &mut n,
                            // SAFETY: `p` addresses a live parser, which nothing else accesses
                            // during the call; the hook does not read `p` and `endp`.
                            |p| unsafe {
                                llhttp__after_message_complete(
                                    p,
                                    std::ptr::null(),
                                    std::ptr::null(),
                                )
                            },
                            |p| c_int::from(p.as_mut().after_message_complete()),
                        );
                    }
                }
            }
        }
    }
    assert_eq!(n, 4 * 2048 * 4 * 3 * 3);
}

/// A sequence over one message's life, as `llhttp.c` calls the hooks: before and after its
/// headers, then at its end, the two parsers compared after each step, and the predicates
/// asked between them, as libgit2's HTTP client asks `llhttp_should_keep_alive` at
/// `on_headers_complete`.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_hooks_in_sequence_are_llhttps() {
    let messages = [
        (2, sys::F_CONTENT_LENGTH, 200, 1, (1, 1), 10),
        (
            2,
            sys::F_CHUNKED | sys::F_TRANSFER_ENCODING,
            200,
            1,
            (1, 1),
            0,
        ),
        (
            2,
            sys::F_UPGRADE | sys::F_CONNECTION_UPGRADE,
            101,
            1,
            (1, 1),
            0,
        ),
        (2, sys::F_CONNECTION_KEEP_ALIVE, 200, 1, (1, 0), 0),
        (1, 0, 0, 5, (1, 1), 0),
        (
            1,
            sys::F_UPGRADE | sys::F_CONNECTION_UPGRADE | sys::F_CONTENT_LENGTH,
            0,
            3,
            (1, 1),
            2,
        ),
        (1, sys::F_TRANSFER_ENCODING, 0, 3, (1, 1), 0),
    ];
    for (parser_type, flags, status_code, method, version, content_length) in messages {
        let state = State {
            parser_type,
            flags: u16::try_from(flags).unwrap(),
            status_code,
            method,
            version,
            content_length,
            finish: 1,
            ..State::default()
        };
        let mut c = state.parser();
        let mut rust = state.parser();
        // Calls `$c` with C's parser and `$rust` with the Rust's, and compares what they
        // return and the parsers after the call.
        macro_rules! step {
            ($c:expr, $rust:expr) => {{
                let c_ret = $c(c.as_mut().as_mut_ptr());
                let rust_ret = $rust(rust.as_mut());
                assert_eq!(c_ret, rust_ret, "{state:?}");
                assert_eq!(fields(&c), fields(&rust), "{state:?}");
            }};
        }
        let null = std::ptr::null();
        step!(
            // SAFETY: `p` addresses the live parser `c`, which nothing else accesses during
            // the call; the hook does not read `p` and `endp`.
            |p| unsafe { llhttp__before_headers_complete(p, null, null) },
            |mut r: ParserMut<'_>| {
                r.before_headers_complete();
                0
            }
        );
        step!(
            // SAFETY: as above.
            |p| unsafe { sys::llhttp_should_keep_alive(p) },
            |r: ParserMut<'_>| c_int::from(r.as_ref().should_keep_alive())
        );
        step!(
            // SAFETY: as above.
            |p| unsafe { llhttp__after_headers_complete(p, null, null) },
            |r: ParserMut<'_>| code(r.as_ref().after_headers_complete())
        );
        step!(
            // SAFETY: as above.
            |p| unsafe { sys::llhttp_message_needs_eof(p) },
            |r: ParserMut<'_>| c_int::from(r.as_ref().message_needs_eof())
        );
        step!(
            // SAFETY: as above.
            |p| unsafe { llhttp__after_message_complete(p, null, null) },
            |mut r: ParserMut<'_>| c_int::from(r.after_message_complete())
        );
        step!(
            // SAFETY: as above.
            |p| unsafe { sys::llhttp_should_keep_alive(p) },
            |r: ParserMut<'_>| c_int::from(r.as_ref().should_keep_alive())
        );
    }
}
