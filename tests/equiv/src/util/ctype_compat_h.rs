//! Equivalence of `src/util/ctype_compat.h`'s eight macros with their ports.
//!
//! On this build each macro's replacement list is `<ctype.h>`'s function over its argument
//! cast to `unsigned char`, as `!!isalpha((unsigned char)(a))`. The raw bindings reach no
//! macro, so each is compared twice:
//!
//! - with its replacement list, glibc's function called on the byte the cast gives, for every `int`
//!   from below `CHAR_MIN` to past `UCHAR_MAX`: `EOF`, the negative `char`s a signed `char` holds
//!   and the `int`s the cast reduces. Neither the raw bindings nor `glibc-bindings` bind
//!   `<ctype.h>`, so its eight functions are declared here, as glibc exports them; and
//! - where the C expands it, through the functions whose result a single byte's class decides:
//!   `git__isspace` through `git_blob_data_is_binary` (`git_str_is_binary`, for the control bytes
//!   it does not count as printable) and `git_config_parse_int64` (`git__strntol64`'s leading white
//!   space), `git__isalnum` through `git_message_trailers` (a trailer key's characters),
//!   `git__tolower` through `git_path_is_gitfile` (`path.c`'s HFS+ folding of each code point), and
//!   `git__isxdigit` and `git__isdigit` through `git_diff_from_buffer` (`patch_parse.c`'s object
//!   ids of an `index` line, and `parse.c`'s `git_parse_advance_digit` in a hunk header).
//!
//! The other sites are not compared. `git__isprint` expands only in `smart_pkt.c`'s
//! `parse_len`, which a fetch over a smart transport reaches, and no offline test drives one.
//! `git__toupper` expands only in `date.c`'s parser, and `git__isalpha` there, in `fs_path.c`'s
//! `git_fs_path_resolve_relative` and in `diff_driver.c`'s function-context lines: libgit2 runs
//! the parser on a date from the environment (`git_signature_default_from_env`) or a reflog
//! selector (`git_revparse_single`), whose results a byte decides only together with the
//! current time and the reflog, and the other two on a path's directory and on a hunk's
//! preceding lines, which no function the bindings give a single byte to returns. They stay
//! for a review that sets up that state, in a child process, a reflog or a local remote: the
//! comparison with the replacement list checks the macros, not those sites.
//!
//! The test process never calls `setlocale`, so `<ctype.h>` runs in the C locale, as in
//! libgit2.

use std::ffi::{CString, c_int};

use libgit2_crustify::libgit2::src::libgit2::blob::data_is_binary;
use libgit2_crustify::libgit2::src::libgit2::config::parse_i64;
use libgit2_crustify::libgit2::src::libgit2::diff::Diff;
use libgit2_crustify::libgit2::src::libgit2::path::{Filesystem, GitFile, is_gitfile};
use libgit2_crustify::libgit2::src::libgit2::trailer::MessageTrailerArray;
use libgit2_crustify::libgit2::src::util::ctype_compat_h::{
    is_alnum, is_alpha, is_digit, is_print, is_space, is_xdigit, to_lower, to_upper,
};

unsafe extern "C" {
    fn isalnum(c: c_int) -> c_int;
    fn isalpha(c: c_int) -> c_int;
    fn isdigit(c: c_int) -> c_int;
    fn isprint(c: c_int) -> c_int;
    fn isspace(c: c_int) -> c_int;
    fn isxdigit(c: c_int) -> c_int;
    fn tolower(c: c_int) -> c_int;
    fn toupper(c: c_int) -> c_int;
}

/// `(unsigned char)(a)`: the low byte of `a`'s two's complement.
fn as_unsigned_char(a: c_int) -> u8 {
    a.to_le_bytes()[0]
}

/// `f((unsigned char)(a))`, as the macro's replacement list calls it.
fn c_call(f: unsafe extern "C" fn(c_int) -> c_int, a: c_int) -> c_int {
    // SAFETY: `<ctype.h>`'s functions are defined for every value of `unsigned char` and for
    // `EOF`, read only the calling thread's locale tables, and keep nothing; the cast makes the
    // argument an `unsigned char`'s value.
    unsafe { f(c_int::from(as_unsigned_char(a))) }
}

/// Every `int` the comparison passes: below `CHAR_MIN`, through every `char` and byte value,
/// and past `UCHAR_MAX`.
fn arguments() -> impl Iterator<Item = c_int> {
    -300..=600
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_predicates_match_the_replacement_lists() {
    type Pair = (
        &'static str,
        fn(u8) -> bool,
        unsafe extern "C" fn(c_int) -> c_int,
    );
    let pairs: [Pair; 6] = [
        ("git__isalnum", is_alnum, isalnum),
        ("git__isalpha", is_alpha, isalpha),
        ("git__isdigit", is_digit, isdigit),
        ("git__isprint", is_print, isprint),
        ("git__isspace", is_space, isspace),
        ("git__isxdigit", is_xdigit, isxdigit),
    ];
    for (name, rust, c) in pairs {
        for a in arguments() {
            // `!!f(...)`: the C normalises the class to 0 or 1.
            let c_class = c_call(c, a) != 0;
            assert_eq!(rust(as_unsigned_char(a)), c_class, "{name}({a})");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_case_maps_match_the_replacement_lists() {
    type Pair = (
        &'static str,
        fn(u8) -> u8,
        unsafe extern "C" fn(c_int) -> c_int,
    );
    let pairs: [Pair; 2] = [
        ("git__tolower", to_lower, tolower),
        ("git__toupper", to_upper, toupper),
    ];
    for (name, rust, c) in pairs {
        for a in arguments() {
            let mapped = rust(as_unsigned_char(a));
            assert_eq!(c_int::from(mapped), c_call(c, a), "{name}({a})");
        }
    }
}

/// `git_str_is_binary` counts a byte as printable above `0x1F` but for DEL, and for `\b`,
/// ESC and the form feed; of the others, a NUL makes the data binary, and any byte
/// `git__isspace` rejects counts against it. A single byte of those is binary exactly when it
/// is not white space.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_space_matches_git_str_is_binary() {
    for c in u8::MIN..=u8::MAX {
        let printable = (c > 0x1F && c != 0x7F) || matches!(c, 0x08 | 0x1B | 0x0C);
        let expected = c == 0 || (!printable && !is_space(c));
        assert_eq!(data_is_binary(&[c]), expected, "{c:#04x}");
    }
}

/// `git__strntol64` skips the white space `git__isspace` finds before the number:
/// `<c><c>7` reads as 7 exactly when `c` is white space, or the octal prefix `0`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_space_matches_git_config_parse_int64() {
    for c in 1..=u8::MAX {
        let value = CString::new([c, c, b'7']).expect("no NUL");
        let seven = parse_i64(Some(&value)).ok() == Some(7);
        assert_eq!(seven, is_space(c) || c == b'0', "{c:#04x}");
    }
}

/// A trailer key is read while `git__isalnum` or `-` accepts its characters: `K<c>y: v` is
/// the trailer `K<c>y` exactly when `c` is a letter, a digit or `-`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_alnum_matches_git_message_trailers() {
    for c in 1..=u8::MAX {
        let mut message = b"Subject\n\nK".to_vec();
        message.extend_from_slice(&[c, b'y', b':', b' ', b'v', b'\n']);
        let message = CString::new(message).expect("no NUL");
        let array = MessageTrailerArray::parse(&message).expect("git_message_trailers");
        let trailers: Vec<_> = array
            .as_ref()
            .trailers()
            .iter()
            .map(|t| (t.key().to_bytes().to_vec(), t.value().to_bytes().to_vec()))
            .collect();
        let is_key = trailers == [(vec![b'K', c, b'y'], b"v".to_vec())];
        assert_eq!(is_key, is_alnum(c) || c == b'-', "{c:#04x}");
    }
}

/// `path.c`'s HFS+ check folds each code point of a name with `git__tolower` and compares it
/// with the file's name: `.` and the name with one letter replaced by the code point `c` is the
/// file exactly when `c` folds to that letter. Every code point up to `0xFF` is tried in place
/// of every letter of the three names; none of them is one HFS+ ignores.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn to_lower_matches_git_path_is_gitfile() {
    let files = [
        (GitFile::Gitignore, "gitignore"),
        (GitFile::Gitmodules, "gitmodules"),
        (GitFile::Gitattributes, "gitattributes"),
    ];
    for (file, name) in files {
        for (at, letter) in name.bytes().enumerate() {
            for c in 1..=u8::MAX {
                let mut component = String::from(".");
                component.push_str(&name[..at]);
                component.push(char::from(c));
                component.push_str(&name[at + 1..]);
                let component = CString::new(component).expect("no NUL");
                let matched = is_gitfile(&component, file, Filesystem::Hfs);
                assert_eq!(matched, to_lower(c) == letter, "{name}[{at}] = U+{c:04X}");
            }
        }
    }
}

/// A patch whose `index` line gives `git_diff_from_buffer` six hex digits and `c` before `..`:
/// `patch_parse.c` reads the old id while `git__isxdigit` accepts its digits, so the patch
/// parses, with an id of seven digits, exactly when `c` is a hex digit; otherwise the id ends
/// at `c`, where `..` is due.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_xdigit_matches_git_diff_from_buffer_index_ids() {
    for c in 1..=u8::MAX {
        let mut patch = b"diff --git a/f b/f\nindex 123456".to_vec();
        patch.push(c);
        patch.extend_from_slice(b"..89abcde 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n");
        let abbrev = Diff::from_buffer(&patch)
            .ok()
            .and_then(|diff| Some(diff.as_ref().get(0)?.old_file().id_abbrev()));
        let expected = is_xdigit(c).then_some(7);
        assert_eq!(abbrev, expected, "{c:#04x}");
    }
}

/// A hunk header `@@ -<c>1 +1 @@`: `git_parse_advance_digit` reads the old start only when
/// `git__isdigit` accepts its first character, so `git_diff_from_buffer` parses the patch
/// exactly when `c` is a decimal digit.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_digit_matches_git_diff_from_buffer_hunk_headers() {
    for c in 1..=u8::MAX {
        let mut patch =
            b"diff --git a/f b/f\nindex 1234567..89abcde 100644\n--- a/f\n+++ b/f\n@@ -".to_vec();
        patch.push(c);
        patch.extend_from_slice(b"1 +1 @@\n-a\n+b\n");
        let parsed = Diff::from_buffer(&patch).is_ok();
        assert_eq!(parsed, is_digit(c), "{c:#04x}");
    }
}
