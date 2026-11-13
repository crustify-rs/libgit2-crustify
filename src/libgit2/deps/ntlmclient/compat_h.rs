//! `deps/ntlmclient/compat.h`: the NTLM client's compatibility macros, ported to Rust.
//!
//! The header has no translation unit and no state: its one scheduled item, `MIN`, is native
//! Rust, reached by no C, so the module has no unsafe regions and no exports. The macro stays
//! in the header for the C that still expands it, `ntlm.c`'s `des_key_from_password` and
//! `generate_lm_hash`, as porting's Macros section keeps a header's macro. The header's
//! `bool`, `true` and `false` are defined only under `_MSC_VER`, which this build is not;
//! elsewhere it includes `<stdbool.h>`, whose `bool` is Rust's.
#![forbid(unsafe_code)]

/// The lesser of `x` and `y`: `x` when `x < y`, and `y` otherwise.
///
/// C's `MIN(x, y)` expands to `(((x) < (y)) ? (x) : (y))`; this is its body, over any
/// [`PartialOrd`] type. Ties give `y`, and so does a pair `<` cannot order, such as a NaN
/// on either side, as the C's comparison does. For an [`Ord`] type the result is
/// [`core::cmp::min`]'s, which the plan names as the standard equivalent: the two differ only
/// in which of two equal values they return.
///
/// It differs from the macro in two ways its types require:
///
/// - the macro converts its operands to a common type, so that `MIN(-1, (size_t)5)` compares
///   `SIZE_MAX` with `5`; here both are one type, which a caller converts to explicitly, as
///   `generate_lm_hash`'s `MIN(7, password_len)` has both `usize`; and
/// - the macro evaluates the operand it returns twice, and the other once; here each is evaluated
///   once, before the call. Every C use passes operands without side effects, `plaintext_len`,
///   `password_len` and constants, whose results are the same either way.
///
/// It is not a `const fn`: a generic comparison cannot be called in a constant on stable
/// Rust, whose const traits are unstable.
///
/// ```
/// use libgit2_crustify::libgit2::deps::ntlmclient::compat_h::min;
///
/// assert_eq!(min(3_usize, 7), 3);
/// assert_eq!(min(14_usize, 9), 9);
/// assert_eq!(min(-0.0_f64, 0.0).to_bits(), 0.0_f64.to_bits());
/// assert_eq!(min(f64::NAN, 1.0).to_bits(), 1.0_f64.to_bits());
/// ```
///
/// Replaces `mac::MIN`
#[doc(alias = "MIN")]
#[must_use]
#[inline]
pub fn min<T: PartialOrd>(x: T, y: T) -> T {
    if x < y { x } else { y }
}
