//! `src/util/integer.h`: the checked integer conversions and overflow-checked arithmetic
//! libgit2 names `git__is_*` and `git__*_overflow`, ported to Rust.
//!
//! The header has no translation unit and no state. Its six functions and six macros are
//! native Rust here. The header stays for the C that still includes it: `git2_util.h`, which
//! every translation unit of libgit2 includes, and `vector.c`. That C keeps expanding the
//! macros from it, as porting's Macros section keeps a header's macros, and none of them
//! calls the Rust. The functions were `static inline`, so each C caller compiled its own copy
//! of their bodies, and an export alone did not reach it. Since the module's switch, as
//! porting's Static inline functions section has it, the header declares the six by their
//! prototypes in place of its `GIT_INLINE` definitions, and the remaining C calls this
//! module's exports, its `ffi_export` gateway, under the functions' bare names.
//!
//! # The range checks
//!
//! Each `git__is_<type>(p)` converts `p` to `<type>` and back, and reports whether the round
//! trip kept it. Their C callers pass `int64_t`, `uint64_t` (`git_object_size_t`) and
//! `size_t` values alike through the one parameter, converted at the call. The ports take
//! each value at its own type and report whether it is in `<type>`'s range, which is what
//! their documentation states and what every caller asks. The round trip departs from that
//! range only where the conversion keeps the width, and each such departure is a C defect
//! the ports do not copy:
//!
//! - `git__is_ssizet` converts `size_t` to `ssize_t` and back, which keeps every value: it never
//!   returns false, so its callers' guards (`p_read`, `git_futils_readbuffer_fd`, and `p_mmap`
//!   under `NO_MMAP`, which this build does not define) accept a count past `SSIZE_MAX`.
//!   [`fits_isize`] refuses it.
//! - `git__is_sizet` and `git__is_ulong` on a target whose `size_t` and `unsigned long` are 64 bits
//!   wide, as this build's are, keep every `int64_t`, a negative one included: a negative
//!   configured size passes as a huge one (`pack-objects.c`'s `config_get` of
//!   `pack.deltaCacheSize`, which then converts it to `size_t`). [`fits_usize`] and
//!   [`fits_c_ulong`] refuse negative values; for an unsigned 64-bit value, which a C caller passes
//!   through a negative `int64_t`, they give C's answer.
//!
//! `git__is_uint16`, `git__is_uint32` and `git__is_int` narrow, so their round trips keep
//! exactly the range, and the ports give C's answer for every value of the parameter's
//! type, the one their C callers pass.
//!
//! # The overflow checks
//!
//! Each `git__<op>_<type>_overflow(out, one, two)` returns true on overflow, and otherwise
//! false with `one <op> two` in `*out`. The ports return that result, `None` on overflow, as
//! the standard library's `checked_*` operations do, over which they are written. On this
//! build, which clang compiles for a 64-bit target in every variant, the macros expand to
//! the compiler's `__builtin_*_overflow` intrinsics (`__builtin_uaddl_overflow` and
//! `__builtin_umull_overflow` for `size_t`, `unsigned long` here), as gcc's would. The header's
//! other branches, Microsoft's `<intsafe.h>` functions under `_MSC_VER` and the `GIT_INLINE`
//! fallbacks for a compiler with neither, report the same overflows and results, but for
//! `git__multiply_int64_overflow`'s fallback, which the build takes only where neither applies or
//! under clang on a 32-bit target (`GIT_ARCH_32`): as its comment and `tests/util/integer.c`'s edge
//! cases record, it reports `INT64_MIN * -1` and `-1 * INT64_MIN` as fitting, with `INT64_MIN` as
//! the product, and `INT64_MIN * 1` and `1 * INT64_MIN` as overflowing; [`checked_mul_i64`] gives
//! this build's answer, which is the mathematical one.
//!
//! What the branches leave in `*out` on overflow differs, and the ports return nothing
//! then: the intrinsics store the wrapped result, `<intsafe.h>` an error value and the
//! fallbacks nothing. One C caller would keep it, `pack-objects.c`'s `try_delta`, whose
//! `git__add_sizet_overflow(&pb->delta_cache_size, pb->delta_cache_size, delta_size)` would
//! leave the wrapped sum in the cache's size on this build before it fails. No input reaches
//! that failure: under the same cache lock, `delta_cacheable` has just computed the same sum
//! with the same macro and returned 0 on its overflow, so `try_delta` adds only a sum that
//! fits. No caller relies on what an overflow leaves.
//!
//! # The exports
//!
//! The gateway's six exports keep the C functions' signatures, `int64_t` or `size_t` in and
//! `int` out, and serve the C callers that pass `int64_t`, `uint64_t` and `size_t` values
//! through them. `git__is_sizet` and `git__is_ulong` take an `int64_t` that is a
//! `uint64_t` for most of their callers: an object's or a blob's size. A negative value is
//! then a size past `INT64_MAX`, and the exports give C's answer for every `int64_t`, as
//! [`fits_usize`] and [`fits_c_ulong`] give it for the `uint64_t` the bits hold. A caller
//! that passes a negative `int64_t` keeps C's acceptance, which the gateway cannot tell from
//! a large size. That is `pack-objects.c`'s configured sizes, whose ports refuse it.
//! `git__is_ssizet` returns [`fits_isize`]'s answer, so that the guards of `p_read` and
//! `git_futils_readbuffer_fd`, and `p_mmap`'s under `NO_MMAP`, refuse a count past
//! `SSIZE_MAX`, as they are written to. The C function accepts every count. The other three
//! narrow, and give C's answer.
use core::ffi::{c_int, c_ulong};

/// Whether `value` is in `usize`'s range: whether it converts to `usize` without loss.
///
/// C's `git__is_sizet(p)` takes an `int64_t` and reports whether it survives the round trip
/// through `size_t`. On this build, whose `size_t` is 64 bits wide, that keeps every value,
/// so C accepts a negative `p`; this function refuses it, as the module's docs describe. For
/// every other value, and for every unsigned value a C caller converted to `int64_t`, it
/// gives C's answer.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_usize;
///
/// assert!(fits_usize(0_i64));
/// assert!(fits_usize(u64::MAX));
/// assert!(!fits_usize(-1_i64));
/// ```
///
/// Replaces `fn::git__is_sizet`
#[doc(alias = "git__is_sizet")]
#[must_use]
#[inline]
pub fn fits_usize<T: TryInto<usize>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// Whether `value` is in `isize`'s range, `0..=isize::MAX` for a `usize`.
///
/// C's `git__is_ssizet(p)` converts its `size_t` to `ssize_t` and back, which keeps every
/// value: it is true for every `p`. This function refuses a `p` past `isize::MAX`, which the
/// C's callers mean it to, as the module's docs describe.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_isize;
///
/// assert!(fits_isize(isize::MAX.unsigned_abs()));
/// assert!(!fits_isize(usize::MAX));
/// ```
///
/// Replaces `fn::git__is_ssizet`
#[doc(alias = "git__is_ssizet")]
#[must_use]
#[inline]
pub fn fits_isize<T: TryInto<isize>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// Whether `value` is in `u16`'s range.
///
/// C's `git__is_uint16(p)` takes a `size_t`, whose round trip through `uint16_t` keeps
/// exactly the values in that range: for a `usize`, this is its answer.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_u16;
///
/// assert!(fits_u16(65_535_usize));
/// assert!(!fits_u16(65_536_usize));
/// ```
///
/// Replaces `fn::git__is_uint16`
#[doc(alias = "git__is_uint16")]
#[must_use]
#[inline]
pub fn fits_u16<T: TryInto<u16>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// Whether `value` is in `u32`'s range.
///
/// C's `git__is_uint32(p)` takes a `size_t`, whose round trip through `uint32_t` keeps
/// exactly the values in that range: for a `usize`, this is its answer.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_u32;
///
/// assert!(fits_u32(4_294_967_295_usize));
/// assert!(!fits_u32(4_294_967_296_usize));
/// ```
///
/// Replaces `fn::git__is_uint32`
#[doc(alias = "git__is_uint32")]
#[must_use]
#[inline]
pub fn fits_u32<T: TryInto<u32>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// Whether `value` is in C's `unsigned long`'s range, [`c_ulong`]'s.
///
/// C's `git__is_ulong(p)` takes an `int64_t` and reports whether it survives the round trip
/// through `unsigned long`, which on this build is 64 bits wide and keeps every value, a
/// negative one included; this function refuses a negative `value`, as the module's docs
/// describe. Where `unsigned long` is 32 bits wide, as on Windows, the round trip keeps
/// exactly the range, which is this function's answer there too.
///
/// ```
/// use core::ffi::c_ulong;
///
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_c_ulong;
///
/// assert!(fits_c_ulong(c_ulong::MAX));
/// assert!(!fits_c_ulong(-1_i64));
/// ```
///
/// Replaces `fn::git__is_ulong`
#[doc(alias = "git__is_ulong")]
#[must_use]
#[inline]
pub fn fits_c_ulong<T: TryInto<c_ulong>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// Whether `value` is in C's `int`'s range, [`c_int`]'s.
///
/// C's `git__is_int(p)` takes an `int64_t`, whose round trip through `int` keeps exactly the
/// values in that range: for an `i64`, this is its answer. A value of another type is taken
/// as it is, where C would convert it to `int64_t` first, and accept `UINT64_MAX` as `-1`.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::fits_c_int;
///
/// assert!(fits_c_int(-2_147_483_648_i64));
/// assert!(!fits_c_int(2_147_483_648_i64));
/// assert!(!fits_c_int(u64::MAX));
/// ```
///
/// Replaces `fn::git__is_int`
#[doc(alias = "git__is_int")]
#[must_use]
#[inline]
pub fn fits_c_int<T: TryInto<c_int>>(value: T) -> bool {
    value.try_into().is_ok()
}

/// `one + two`, or `None` when the sum overflows `usize`.
///
/// C's `git__add_sizet_overflow(&out, one, two)` returns true on overflow, and otherwise
/// false with the sum in `out`: this is its result. On this build it is
/// `__builtin_uaddl_overflow`; the module's docs name the other branches, which compute the
/// same, and what each leaves in `out` on overflow.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_add_usize;
///
/// assert_eq!(checked_add_usize(42, 69), Some(111));
/// assert_eq!(checked_add_usize(usize::MAX - 1, 1), Some(usize::MAX));
/// assert_eq!(checked_add_usize(usize::MAX, 1), None);
/// ```
///
/// Replaces `mac::git__add_sizet_overflow`
#[doc(alias = "git__add_sizet_overflow")]
#[must_use]
#[inline]
pub const fn checked_add_usize(one: usize, two: usize) -> Option<usize> {
    one.checked_add(two)
}

/// `one * two`, or `None` when the product overflows `usize`.
///
/// C's `git__multiply_sizet_overflow(&out, one, two)` returns true on overflow, and
/// otherwise false with the product in `out`: this is its result. On this build it is
/// `__builtin_umull_overflow`; the module's docs name the other branches, which compute the
/// same, and what each leaves in `out` on overflow.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_mul_usize;
///
/// assert_eq!(checked_mul_usize(0, usize::MAX), Some(0));
/// assert_eq!(checked_mul_usize(usize::MAX, 2), None);
/// ```
///
/// Replaces `mac::git__multiply_sizet_overflow`
#[doc(alias = "git__multiply_sizet_overflow")]
#[must_use]
#[inline]
pub const fn checked_mul_usize(one: usize, two: usize) -> Option<usize> {
    one.checked_mul(two)
}

/// `one + two`, or `None` when the sum overflows C's `int`.
///
/// C's `git__add_int_overflow(&out, one, two)` returns true on overflow, and otherwise false
/// with the sum in `out`: this is its result. On this build it is `__builtin_sadd_overflow`;
/// the module's docs name the other branches, which compute the same, and what each leaves
/// in `out` on overflow.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_add_c_int;
///
/// assert_eq!(checked_add_c_int(i32::MAX, -1), Some(i32::MAX - 1));
/// assert_eq!(checked_add_c_int(i32::MAX, 1), None);
/// ```
///
/// Replaces `mac::git__add_int_overflow`
#[doc(alias = "git__add_int_overflow")]
#[must_use]
#[inline]
pub const fn checked_add_c_int(one: c_int, two: c_int) -> Option<c_int> {
    one.checked_add(two)
}

/// `one - two`, or `None` when the difference overflows C's `int`.
///
/// C's `git__sub_int_overflow(&out, one, two)` returns true on overflow, and otherwise false
/// with the difference in `out`: this is its result. On this build it is
/// `__builtin_ssub_overflow`; the module's docs name the other branches, which compute the
/// same, and what each leaves in `out` on overflow.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_sub_c_int;
///
/// assert_eq!(checked_sub_c_int(-1, i32::MAX), Some(i32::MIN));
/// assert_eq!(checked_sub_c_int(0, i32::MIN), None);
/// ```
///
/// Replaces `mac::git__sub_int_overflow`
#[doc(alias = "git__sub_int_overflow")]
#[must_use]
#[inline]
pub const fn checked_sub_c_int(one: c_int, two: c_int) -> Option<c_int> {
    one.checked_sub(two)
}

/// `one + two`, or `None` when the sum overflows `i64`.
///
/// C's `git__add_int64_overflow(&out, one, two)` returns true on overflow, and otherwise
/// false with the sum in `out`: this is its result. On this build it is
/// `__builtin_add_overflow`, which takes operands of any integer type and checks the exact
/// sum against `out`'s; every C caller passes `int64_t` operands, as this function takes.
/// The module's docs name the other branches, which compute the same, and what each leaves
/// in `out` on overflow.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_add_i64;
///
/// assert_eq!(checked_add_i64(i64::MIN, i64::MAX), Some(-1));
/// assert_eq!(checked_add_i64(i64::MIN, -1), None);
/// ```
///
/// Replaces `mac::git__add_int64_overflow`
#[doc(alias = "git__add_int64_overflow")]
#[must_use]
#[inline]
pub const fn checked_add_i64(one: i64, two: i64) -> Option<i64> {
    one.checked_add(two)
}

/// `one * two`, or `None` when the product overflows `i64`.
///
/// C's `git__multiply_int64_overflow(&out, one, two)` returns true on overflow, and otherwise
/// false with the product in `out`: this is its result. On this build it is
/// `__builtin_mul_overflow`, which checks the exact product against `out`'s type; every C
/// caller passes an `int64_t` and an `int64_t` or an `int` constant, as this function takes
/// them. The fallback the build does not take differs from it on four inputs, which the
/// module's docs name: `checked_mul_i64(i64::MIN, -1)` is `None`, and
/// `checked_mul_i64(i64::MIN, 1)` is `Some(i64::MIN)`.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::integer_h::checked_mul_i64;
///
/// assert_eq!(checked_mul_i64(-0x4000_0000_0000_0000, 2), Some(i64::MIN));
/// assert_eq!(checked_mul_i64(0x4000_0000_0000_0000, 2), None);
/// assert_eq!(checked_mul_i64(i64::MIN, -1), None);
/// assert_eq!(checked_mul_i64(i64::MIN, 1), Some(i64::MIN));
/// ```
///
/// Replaces `mac::git__multiply_int64_overflow`
#[doc(alias = "git__multiply_int64_overflow")]
#[must_use]
#[inline]
pub const fn checked_mul_i64(one: i64, two: i64) -> Option<i64> {
    one.checked_mul(two)
}

/// The gateway through which the C that remains calls this module: the six `git__is_*`
/// functions, under their bare names, which the header declares by their prototypes in
/// place of the `GIT_INLINE` definitions it gave every caller until the module's switch.
/// Each is checked against its raw binding's signature, so that a C declaration that drifts
/// fails to build. None reads memory or can panic, so none is `unsafe` to call or needs a
/// barrier.
#[expect(
    unsafe_code,
    reason = "exports integer.h's static inline functions under their C names"
)]
mod ffi_export {
    use core::ffi::c_int;

    use super::{fits_c_int, fits_c_ulong, fits_isize, fits_u16, fits_u32, fits_usize};
    use crate::sys::libgit2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::git__is_sizet, git__is_sizet];
        let _ = [sys::git__is_ssizet, git__is_ssizet];
        let _ = [sys::git__is_uint16, git__is_uint16];
        let _ = [sys::git__is_uint32, git__is_uint32];
        let _ = [sys::git__is_ulong, git__is_ulong];
        let _ = [sys::git__is_int, git__is_int];
    };

    /// `git__is_sizet`: whether the `uint64_t` whose bits `p` holds is in `size_t`'s range,
    /// which is C's answer for every `int64_t`: a negative `p` passes where `size_t` is 64
    /// bits wide, as the module's docs describe, and fails where it is 32.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_sizet(p: i64) -> c_int {
        c_int::from(fits_usize(p.cast_unsigned()))
    }

    /// `git__is_ssizet`: [`fits_isize`], which refuses a `p` past `SSIZE_MAX`, where C's
    /// round trip accepts it, as the module's docs describe.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_ssizet(p: usize) -> c_int {
        c_int::from(fits_isize(p))
    }

    /// `git__is_uint16`: [`fits_u16`], C's answer.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_uint16(p: usize) -> c_int {
        c_int::from(fits_u16(p))
    }

    /// `git__is_uint32`: [`fits_u32`], C's answer.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_uint32(p: usize) -> c_int {
        c_int::from(fits_u32(p))
    }

    /// `git__is_ulong`: whether the `uint64_t` whose bits `p` holds is in `unsigned long`'s
    /// range, which is C's answer for every `int64_t`, as for `git__is_sizet`.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_ulong(p: i64) -> c_int {
        c_int::from(fits_c_ulong(p.cast_unsigned()))
    }

    /// `git__is_int`: [`fits_c_int`], C's answer.
    #[unsafe(no_mangle)]
    extern "C" fn git__is_int(p: i64) -> c_int {
        c_int::from(fits_c_int(p))
    }

    /// What each export returns, without the C: 1 where its type holds the value, 0
    /// elsewhere, at the bounds.
    #[cfg(test)]
    mod unit_priv {
        use super::{
            git__is_int, git__is_sizet, git__is_ssizet, git__is_uint16, git__is_uint32,
            git__is_ulong,
        };

        /// The range each export reports, at its bounds and past them.
        #[test]
        fn exports_report_their_ranges() {
            assert_eq!(git__is_sizet(0), 1);
            assert_eq!(git__is_sizet(i64::MAX), 1);
            assert_eq!(git__is_sizet(-1), 1, "u64::MAX, a 64-bit size_t's");
            assert_eq!(
                git__is_ulong(i64::MIN),
                1,
                "1 << 63, a 64-bit unsigned long's"
            );
            assert_eq!(git__is_ssizet(isize::MAX.unsigned_abs()), 1);
            assert_eq!(git__is_ssizet(isize::MAX.unsigned_abs() + 1), 0);
            assert_eq!(git__is_ssizet(usize::MAX), 0);
            assert_eq!(git__is_uint16(0xFFFF), 1);
            assert_eq!(git__is_uint16(0x1_0000), 0);
            assert_eq!(git__is_uint32(0xFFFF_FFFF), 1);
            assert_eq!(git__is_uint32(0x1_0000_0000), 0);
            assert_eq!(git__is_int(i64::from(i32::MIN)), 1);
            assert_eq!(git__is_int(i64::from(i32::MAX)), 1);
            assert_eq!(git__is_int(i64::from(i32::MAX) + 1), 0);
            assert_eq!(git__is_int(i64::from(i32::MIN) - 1), 0);
        }
    }
}
