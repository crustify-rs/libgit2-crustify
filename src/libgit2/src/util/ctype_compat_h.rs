//! `src/util/ctype_compat.h`: the locale-independent character classes libgit2 names
//! `git__is*` and `git__to*`, ported to Rust.
//!
//! The header has no translation unit and no state: its eight items are native Rust, reached
//! by no C, so the module has no unsafe regions and no exports. The macros stay in the header
//! for the C that still expands them, in 33 files of `src/` (`config.c`, `date.c`, `parse.c`,
//! `str.c`, `util.c` and `trailer.c` among them) and three C tests
//! (`tests/util/sortedcache.c`, `tests/libgit2/clone/local.c` and `tests/libgit2/repo/open.c`),
//! where they name libc's `<ctype.h>`, as porting's Macros section keeps a header's macro.
//!
//! # The header's branches
//!
//! This build is not `GIT_WIN32`, so each macro is `<ctype.h>`'s function over its argument
//! cast to `unsigned char`, its predicates normalised to `0` or `1` by `!!`:
//! `#define git__isalpha(a) (!!isalpha((unsigned char)(a)))`. Under `GIT_WIN32`, where the C
//! runtime takes a lock on the locale in each call, the header defines them instead as
//! `GIT_INLINE` functions that compare their `int` with ASCII ranges.
//!
//! The functions here take that `unsigned char`, a [`u8`], and return what both branches
//! return for it: a C caller's `git__isspace(c)` is `is_space(c as u8)`, the cast its own.
//! For every byte, the `GIT_WIN32` comparisons and `<ctype.h>` in the C locale, which libgit2
//! never leaves, give the same result. The branches part only on what the cast hides: an
//! `int` outside a byte's range, such as `EOF` or a `char` past ASCII where `char` is
//! signed, which `GIT_WIN32` compares as given (`git__tolower(-1)` is `-1`) and every other
//! build reduces to its byte first (`255`), and a process locale that a program embedding
//! libgit2 sets with `setlocale`, which `<ctype.h>` follows, and these functions do not.
//! Under glibc's `C.UTF-8` the classes and case maps are the C locale's for every byte, since
//! no byte past ASCII is a character there; a single-byte locale, as `ISO-8859-1`'s, may
//! class some of those bytes as letters, and map their case.
//!
//! Each predicate is the standard library's ASCII class of [`u8`] where it is the C locale's
//! class, as `crustify/artifacts/dependencies.json` routes `<ctype.h>`: `isspace` adds the
//! vertical tab to [`u8::is_ascii_whitespace`], and `isprint` the space to
//! [`u8::is_ascii_graphic`].
#![forbid(unsafe_code)]

/// Whether `c` is an ASCII letter or digit: `A`-`Z`, `a`-`z` or `0`-`9`.
///
/// C's `git__isalnum(c)` is `!!isalnum((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_alnum;
///
/// assert!(is_alnum(b'a') && is_alnum(b'Z') && is_alnum(b'7'));
/// assert!(!is_alnum(b'-') && !is_alnum(b' ') && !is_alnum(0xE9));
/// ```
///
/// Replaces `mac::git__isalnum`
#[doc(alias = "git__isalnum")]
#[must_use]
#[inline]
pub const fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// Whether `c` is an ASCII letter: `A`-`Z` or `a`-`z`.
///
/// C's `git__isalpha(c)` is `!!isalpha((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_alpha;
///
/// assert!(is_alpha(b'a') && is_alpha(b'Z'));
/// assert!(!is_alpha(b'0') && !is_alpha(b'_') && !is_alpha(0xC0));
/// ```
///
/// Replaces `mac::git__isalpha`
#[doc(alias = "git__isalpha")]
#[must_use]
#[inline]
pub const fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// Whether `c` is an ASCII decimal digit, `0`-`9`.
///
/// C's `git__isdigit(c)` is `!!isdigit((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_digit;
///
/// assert!(is_digit(b'0') && is_digit(b'9'));
/// assert!(!is_digit(b'a') && !is_digit(b'/') && !is_digit(b':'));
/// ```
///
/// Replaces `mac::git__isdigit`
#[doc(alias = "git__isdigit")]
#[must_use]
#[inline]
pub const fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// Whether `c` is a printable ASCII character, the space included: `0x20` to `0x7E`.
///
/// C's `git__isprint(c)` is `!!isprint((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes. It is [`u8::is_ascii_graphic`] with the
/// space, which `isgraph` leaves out and `isprint` counts.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_print;
///
/// assert!(is_print(b' ') && is_print(b'~') && is_print(b'a'));
/// assert!(!is_print(b'\t') && !is_print(0x7F) && !is_print(0x80));
/// ```
///
/// Replaces `mac::git__isprint`
#[doc(alias = "git__isprint")]
#[must_use]
#[inline]
pub const fn is_print(c: u8) -> bool {
    c == b' ' || c.is_ascii_graphic()
}

/// Whether `c` is C's white space: the space, `\t`, `\n`, the vertical tab `\v` (`0x0B`),
/// the form feed `\f` (`0x0C`) or `\r`.
///
/// C's `git__isspace(c)` is `!!isspace((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes. It is [`u8::is_ascii_whitespace`] with
/// the vertical tab, which that function, after WHATWG's definition, leaves out.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_space;
///
/// assert!(is_space(b' ') && is_space(b'\n') && is_space(0x0B) && is_space(0x0C));
/// assert!(!is_space(b'\0') && !is_space(0x85) && !is_space(0xA0));
/// ```
///
/// Replaces `mac::git__isspace`
#[doc(alias = "git__isspace")]
#[must_use]
#[inline]
pub const fn is_space(c: u8) -> bool {
    c == 0x0B || c.is_ascii_whitespace()
}

/// Whether `c` is an ASCII hexadecimal digit: `0`-`9`, `a`-`f` or `A`-`F`.
///
/// C's `git__isxdigit(c)` is `!!isxdigit((unsigned char)(c))`: this is its result for that
/// byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::is_xdigit;
///
/// assert!(is_xdigit(b'0') && is_xdigit(b'f') && is_xdigit(b'F'));
/// assert!(!is_xdigit(b'g') && !is_xdigit(b'G') && !is_xdigit(b'x'));
/// ```
///
/// Replaces `mac::git__isxdigit`
#[doc(alias = "git__isxdigit")]
#[must_use]
#[inline]
pub const fn is_xdigit(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// `c` in lower case: an ASCII capital, `A`-`Z`, becomes its small letter, and every other
/// byte is returned unchanged.
///
/// C's `git__tolower(c)` is `tolower((unsigned char)(c))`, an `int` that holds a byte for
/// every argument: this is that byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::to_lower;
///
/// assert_eq!(to_lower(b'A'), b'a');
/// assert_eq!(to_lower(b'z'), b'z');
/// assert_eq!(to_lower(b'@'), b'@');
/// assert_eq!(to_lower(0xC9), 0xC9);
/// ```
///
/// Replaces `mac::git__tolower`
#[doc(alias = "git__tolower")]
#[must_use]
#[inline]
pub const fn to_lower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

/// `c` in upper case: an ASCII small letter, `a`-`z`, becomes its capital, and every other
/// byte is returned unchanged.
///
/// C's `git__toupper(c)` is `toupper((unsigned char)(c))`, an `int` that holds a byte for
/// every argument: this is that byte, as the module's documentation describes.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::ctype_compat_h::to_upper;
///
/// assert_eq!(to_upper(b'a'), b'A');
/// assert_eq!(to_upper(b'Z'), b'Z');
/// assert_eq!(to_upper(b'`'), b'`');
/// assert_eq!(to_upper(0xE9), 0xE9);
/// ```
///
/// Replaces `mac::git__toupper`
#[doc(alias = "git__toupper")]
#[must_use]
#[inline]
pub const fn to_upper(c: u8) -> u8 {
    c.to_ascii_uppercase()
}
