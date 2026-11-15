//! `src/util/cc-compat.h`: `unused`, `PRI_DZ`, `PRI_UZ`, `PRI_XZ` and `va_copy`, the ports of
//! `GIT_UNUSED`, `PRIdZ`, `PRIuZ`, `PRIxZ` and `va_copy`, checked against the replacement
//! lists this build's branches of the header give them. The module has no C tests to carry
//! over. None reaches C, so Miri runs them all.

use std::cell::Cell;
use std::fmt::{self, Write as _};

use libgit2_crustify::libgit2::src::util::cc_compat_h::{PRI_DZ, PRI_UZ, PRI_XZ, unused, va_copy};

/// A value that counts its drops, so that a test sees whether `unused` dropped it.
struct Dropped<'a>(&'a Cell<usize>);

impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

/// `GIT_UNUSED(x)` evaluates `x` and discards it: `unused` neither moves nor drops what it is
/// given, which stays usable after the call.
#[test]
fn unused_neither_moves_nor_drops() {
    let drops = Cell::new(0);
    let value = Dropped(&drops);
    unused(&value);
    unused(&value);
    assert_eq!(drops.get(), 0);
    drop(value);
    assert_eq!(drops.get(), 1);

    let mut payload = vec![1_u8, 2, 3];
    unused(&payload);
    let borrowed = &mut payload;
    unused(&borrowed);
    borrowed.push(4);
    assert_eq!(payload, [1, 2, 3, 4]);
}

/// `GIT_UNUSED` takes a parameter of any type, the callbacks' `void *` payloads among them:
/// `unused` takes any type, unsized ones and trait objects included.
#[test]
fn unused_takes_any_type() {
    unused(&());
    unused(&0_i32);
    unused("refs/heads/main");
    unused(&b"bytes"[..]);
    unused(&std::ptr::null::<u8>());
    let callback: &dyn Fn(usize) -> usize = &|n| n + 1;
    unused(callback);
    assert_eq!(callback(1), 2);
}

/// `unused` is a `const fn`: it is called where a constant is computed.
#[test]
fn unused_evaluates_in_constants() {
    const LEN: usize = {
        let name = "HEAD";
        unused(name);
        name.len()
    };
    assert_eq!(LEN, 4);
}

/// The header's branch for this build, neither MSVC nor MinGW, is C99's `z` modifier with the
/// conversion each name gives: `d` for `PRIdZ`, `u` for `PRIuZ` and `x` for `PRIxZ`.
#[test]
fn the_formats_are_c99s_size_modifier() {
    assert_eq!(PRI_DZ, "zd");
    assert_eq!(PRI_UZ, "zu");
    assert_eq!(PRI_XZ, "zx");
    for format in [PRI_DZ, PRI_UZ, PRI_XZ] {
        assert!(format.is_ascii());
        assert!(format.starts_with('z'));
        assert_eq!(format.len(), 2);
    }
}

/// A C format string writes each after `%`, as `"%" PRIuZ`: built from the constants, it is
/// the one C's string-literal concatenation gives, ready to hand to C as a `CStr`.
#[test]
fn the_formats_build_c_format_strings() {
    let format = format!("%{PRI_UZ} of %{PRI_XZ}, %{PRI_DZ} left");
    assert_eq!(format, "%zu of %zx, %zd left");
    let c_format = std::ffi::CString::new(format).expect("no NUL in the formats");
    assert_eq!(c_format.as_bytes(), b"%zu of %zx, %zd left");
}

/// A copy formats as the original does, and leaves the original to be formatted again, as
/// `git_str_vprintf` formats a fresh `va_copy` of its list each time it grows its buffer.
#[test]
fn va_copy_formats_as_the_original() {
    let (name, count, mask) = ("refs/heads/main", 3_usize, 0x1F_usize);
    let args = format_args!("{name}: {count} of {mask:x}, {}", -2_isize);
    let first = va_copy(&args);
    let second = va_copy(&first);
    assert_eq!(first.to_string(), "refs/heads/main: 3 of 1f, -2");
    assert_eq!(second.to_string(), first.to_string());
    assert_eq!(args.to_string(), first.to_string());
    assert_eq!(first.to_string(), "refs/heads/main: 3 of 1f, -2");
}

/// An empty list, a list of no arguments and one of many each copy to themselves.
#[test]
fn va_copy_copies_every_list() {
    assert_eq!(va_copy(&format_args!("")).to_string(), "");
    assert_eq!(
        va_copy(&format_args!("no arguments")).to_string(),
        "no arguments"
    );
    assert_eq!(
        va_copy(&format_args!("no arguments")).as_str(),
        Some("no arguments")
    );
    let (a, b, c, d) = (1_u8, 'x', 2.5_f64, "end");
    assert_eq!(
        va_copy(&format_args!("{a}{b}{c}{d}{a}")).to_string(),
        "1x2.5end1"
    );
}

/// A writer that refuses every write past `room` bytes, as a `vsnprintf` into a buffer too
/// small stops, so that the list is formatted again into a larger one.
struct Bounded {
    out: String,
    room: usize,
}

impl fmt::Write for Bounded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.out.len() + s.len() > self.room {
            return Err(fmt::Error);
        }
        self.out.push_str(s);
        Ok(())
    }
}

/// `git_str_vprintf`'s loop: each attempt formats a fresh copy into a buffer, and one that
/// does not fit is retried, from the start of the list, into a larger one. Each copy gives
/// the whole output, however many attempts came before.
#[test]
fn va_copy_restarts_each_attempt() {
    let long = "x".repeat(300);
    let args = format_args!("{long}-{}-{:x}", usize::MAX, usize::MAX);
    let expected = format!("{long}-{}-{:x}", usize::MAX, usize::MAX);
    let mut room = 4;
    let mut attempts = 0;
    let out = loop {
        attempts += 1;
        let mut buf = Bounded {
            out: String::new(),
            room,
        };
        if buf.write_fmt(va_copy(&args)).is_ok() {
            break buf.out;
        }
        assert!(expected.starts_with(&buf.out));
        room = room * 3 / 2 + 8;
    };
    assert_eq!(out, expected);
    assert!(attempts > 1);
    assert_eq!(args.to_string(), expected);
}
