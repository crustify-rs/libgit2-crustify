//! `src/util/cc-compat.h`: the compiler compatibility macros libgit2 builds with, ported to
//! Rust.
//!
//! The header has no translation unit and no state: its five scheduled items are native
//! Rust, reached by no C, so the module has no unsafe regions and no exports. Each macro
//! stays in the header for the C that still expands it, as porting's Macros section keeps a
//! header's macro, and its C and Rust forms are not linked:
//!
//! - `GIT_UNUSED`, in the bodies of 338 functions of the library, the CLI and the tests, whose
//!   parameters a signature requires and the body ignores: [`unused`];
//! - `PRIdZ`, `PRIuZ` and `PRIxZ`, in the format strings of 26 TUs under `src/`, among them
//!   `odb.c`, `tree-cache.c`, `futils.c`, `util.c` and the CLI's, of the clar harness's
//!   `clar_libgit2.c` and of 54 test files: [`PRI_DZ`], [`PRI_UZ`] and [`PRI_XZ`]; and
//! - `va_copy`, in `str.c`'s `git_str_vprintf`, which every printf-style setter, such as
//!   `git_error_set`, formats through: [`va_copy`].
//!
//! Each is defined, on this build, by the branch the header takes under clang on Linux at
//! `-std=c90`: `__GNUC__` is defined, `_MSC_VER` and `__MINGW32__` are not, and `<stdarg.h>`
//! defines `__va_copy` but, in strict C90, not `va_copy`. Every item documents the branches
//! the build does not take, which compute what this one does.
//!
//! The header's other definitions are not API, and no module owns them: the attribute
//! macros `GIT_FLEX_ARRAY`, `GIT_ALIGN`, `GIT_UNUSED_ARG` and `GIT_UNUSED_FUNCTION`, MSVC's
//! warning pragmas and its `bool`, `true` and `false`, which elsewhere are `<stdbool.h>`'s,
//! that is Rust's `bool`. `PRIXZ` and `PRId64`, which no C of this build expands, are not
//! scheduled.
#![forbid(unsafe_code)]

use core::fmt;

/// Evaluates nothing more than its argument, and uses it, so that a parameter a signature
/// requires and a body ignores raises no warning.
///
/// C's `GIT_UNUSED(x)` copies `x` into an unused local of its type (`__typeof__(x) _unused =
/// (x);`) where the compiler is GNU C, as clang is, and elsewhere casts it to `void`
/// (`((void)(x))`); both evaluate `x` once and discard it. This takes the value by reference,
/// so that it neither moves nor drops what it is given, whatever its type: it is Rust's
/// `let _ = &x;`. Native Rust marks an ignored parameter by its name instead, `_x`, which
/// needs no call.
///
/// It computes on its argument alone, so it is a `const fn`.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::cc_compat_h::unused;
///
/// fn ignores_its_payload(payload: &mut Vec<u8>) -> i32 {
///     unused(payload);
///     0
/// }
///
/// let mut payload = vec![1, 2, 3];
/// assert_eq!(ignores_its_payload(&mut payload), 0);
/// assert_eq!(payload, [1, 2, 3]);
/// ```
///
/// Replaces `mac::GIT_UNUSED`
#[doc(alias = "GIT_UNUSED")]
#[inline]
pub const fn unused<T: ?Sized>(_value: &T) {}

/// The printf length modifier and conversion of a `ssize_t` in decimal, `"zd"`: a C format
/// string writes it after `%`, as `"%" PRIdZ`.
///
/// It is the C99 `z` modifier on this build, which is neither MSVC nor MinGW. There the
/// header names Microsoft's `I64d` where `size_t` is 64 bits wide and `Id` elsewhere, which
/// format the same values. Native Rust formats an `isize` with `{}`.
///
/// Replaces `mac::PRIdZ`
#[doc(alias = "PRIdZ")]
pub const PRI_DZ: &str = "zd";

/// The printf length modifier and conversion of a `size_t` in decimal, `"zu"`: a C format
/// string writes it after `%`, as `"%" PRIuZ`.
///
/// It is the C99 `z` modifier on this build, which is neither MSVC nor MinGW. There the
/// header names Microsoft's `I64u` where `size_t` is 64 bits wide and `Iu` elsewhere, which
/// format the same values. Native Rust formats a `usize` with `{}`.
///
/// Replaces `mac::PRIuZ`
#[doc(alias = "PRIuZ")]
pub const PRI_UZ: &str = "zu";

/// The printf length modifier and conversion of a `size_t` in lowercase hexadecimal, `"zx"`:
/// a C format string writes it after `%`, as `"%" PRIxZ`.
///
/// It is the C99 `z` modifier on this build, which is neither MSVC nor MinGW. There the
/// header names Microsoft's `I64x` where `size_t` is 64 bits wide and `Ix` elsewhere, which
/// format the same values. Native Rust formats a `usize` with `{:x}`.
///
/// Replaces `mac::PRIxZ`
#[doc(alias = "PRIxZ")]
pub const PRI_XZ: &str = "zx";

/// A copy of a list of formatting arguments, which formats as the original does and leaves
/// the original to be formatted again.
///
/// C's `va_copy(dst, src)` copies the traversal state of a variadic argument list, so that
/// `git_str_vprintf` can hand a fresh copy to each `vsnprintf` it makes while it grows its
/// buffer, and still hold the list it was given. Rust formats a list of arguments built by
/// [`format_args!`], [`fmt::Arguments`], where C formats a `va_list`: formatting reads it
/// without consuming it, so a copy is the value itself, which is `Copy`. Rust's own
/// `va_list`, `core::ffi::VaList`, whose `Clone` is `va_copy`, is unstable (`c_variadic`),
/// and no stable Rust receives a C `va_list`.
///
/// On this build the header defines `va_copy` as `__va_copy`, which clang's `<stdarg.h>`
/// defines as `__builtin_va_copy`: at `-std=c90` it defines `__va_copy` but not `va_copy`.
/// The header leaves a `va_copy` the C library defines as it is, and defines it elsewhere as
/// an assignment, `((dst) = (src))`, for an ABI whose `va_list` is a pointer or a scalar,
/// which copies it as the builtin does. All three give a copy that traverses the same
/// arguments from the start.
///
/// It computes on its argument alone, so it is a `const fn`.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::cc_compat_h::va_copy;
///
/// let args = format_args!("{}-{:x}", 12, 255_usize);
/// let copy = va_copy(&args);
/// assert_eq!(copy.to_string(), "12-ff");
/// assert_eq!(args.to_string(), "12-ff");
/// ```
///
/// Replaces `mac::va_copy`
#[must_use]
#[inline]
pub const fn va_copy<'a>(args: &fmt::Arguments<'a>) -> fmt::Arguments<'a> {
    *args
}
