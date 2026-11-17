//! Argument types for the C ABI (PORTING.md §4.5, §4.7).
//!
//! An export converts its raw pointer arguments in one justified place. The
//! types here make that place the export's signature: each has the ABI of
//! the C pointer it replaces (`#[repr(transparent)]` over it, or Rust's
//! guaranteed `Option<&T>` / `*const T` equivalence), so C passes the same
//! bits, and the export's `# Safety` section, which C's callers satisfy,
//! is the proof that the value is valid. The body then reads the argument
//! with safe code.
//!
//! | C parameter | Export parameter | Read with |
//! |---|---|---|
//! | `const char *s` (NULL or a string) | [`CStrArg<'_>`] | [`get`](CStrArg::get), [`to_bytes`](CStrArg::to_bytes) |
//! | `T *out` written, not read | [`Out<'_, T>`] | [`write`](Out::write) |
//! | `const T *x` read, nothing writes it during the call | `Option<&T>` | [`git_arg!`](crate::git_arg) |
//! | `T *x`, the call has the only access | `Option<&mut T>` | [`git_arg!`](crate::git_arg) |
//! | `const T *v, size_t n`, read, nothing writes it during the call | `*const T`, `usize` | [`slice`](fn@slice) (`unsafe`) |
//!
//! Choose `Option<&T>` only when a shared reference may cover the whole
//! pointee for the whole call: nothing (another thread, a callback, the
//! export itself through another pointer) writes it except through
//! `UnsafeCell`, and nothing frees it, during the call. A reference
//! argument is protected for the call under Stacked/Tree Borrows, so a free
//! or a foreign write before the export returns is undefined behaviour
//! even if the reference is not used again. That includes the export's
//! own writes: an id C lets the caller pass as both input and `out`
//! (`git_odb_exists_prefix(&id, db, &id, n)`), an id that may be the
//! field the call updates (`git_treebuilder_insert` with the entry's own
//! id) or an element it shifts (`git_oidarray__remove`), stays a raw
//! pointer read once up front. `Option<&mut T>` adds that
//! nothing else reads it either: no other argument may alias it (a
//! `git_str` whose own buffer is also passed as data stays a raw pointer).
//! Where any of this fails, keep the raw pointer and convert it at the
//! point of use.
//!
//! [`CStrArg`] and [`Out`] form no reference to the pointee until it is
//! read or written, and then only for that read or write, so they keep the
//! raw pointer's aliasing behaviour: they apply wherever the export reads
//! the string or writes the out-parameter at a single point, as the code
//! before them did with `CStr::from_ptr` / `ptr.write`. A string the call
//! itself frees or rewrites (a path in the output `git_str`'s buffer, an
//! entry's name in the block the call frees) is read or copied before
//! that point, and no `&CStr` of it is passed to a fn that frees it (a
//! reference argument is protected for that fn's whole call).
//!
//! Rust code obtains these values safely only from Rust references
//! (`From<&CStr>`, `From<&mut T>`); from a raw pointer only through the
//! `unsafe` constructors, whose contract is the C caller's.

use core::ffi::{CStr, c_char};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr;

/// A `const char *` argument: NULL, or a NUL-terminated string that stays
/// readable and unwritten for `'a`.
///
/// It has the ABI of `*const c_char`. In an export's signature `'a` is the
/// call, and the export's `# Safety` states the requirement above as the
/// C's contract. Nothing is read until [`get`](Self::get) or
/// [`to_bytes`](Self::to_bytes) (no `strlen` up front, as the C reads).
///
/// Invariant (established by every constructor): `ptr` is NULL or points
/// to a NUL-terminated string that is readable, and written by nothing,
/// for `'a`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CStrArg<'a> {
    ptr: *const c_char,
    _life: PhantomData<&'a CStr>,
}

impl<'a> CStrArg<'a> {
    /// The NULL string.
    pub const NULL: CStrArg<'static> = CStrArg {
        ptr: ptr::null(),
        _life: PhantomData,
    };

    /// `p` as a string argument.
    ///
    /// # Safety
    ///
    /// `p` is NULL, or points to a NUL-terminated string that stays
    /// readable, and that nothing writes, for `'a`.
    #[inline(always)]
    pub const unsafe fn from_ptr(p: *const c_char) -> Self {
        CStrArg {
            ptr: p,
            _life: PhantomData,
        }
    }

    /// The pointer C passed.
    #[inline(always)]
    pub const fn as_ptr(self) -> *const c_char {
        self.ptr
    }

    /// Whether C passed NULL.
    #[inline(always)]
    pub fn is_null(self) -> bool {
        self.ptr.is_null()
    }

    /// The string, or `None` for NULL.
    #[inline(always)]
    pub fn get(self) -> Option<&'a CStr> {
        if self.ptr.is_null() {
            None
        } else {
            // SAFETY: the type's invariant: a non-null `ptr` is a
            // NUL-terminated string readable and unwritten for `'a`.
            Some(unsafe { CStr::from_ptr(self.ptr) })
        }
    }

    /// The string, or `""` for NULL.
    #[inline(always)]
    pub fn or_empty(self) -> &'a CStr {
        self.get().unwrap_or(c"")
    }

    /// The string's bytes without the NUL; empty for NULL
    /// ([`export::cstr_bytes`](super::export::cstr_bytes)).
    #[inline(always)]
    pub fn to_bytes(self) -> &'a [u8] {
        match self.get() {
            Some(s) => s.to_bytes(),
            None => &[],
        }
    }
}

impl<'a> From<&'a CStr> for CStrArg<'a> {
    #[inline(always)]
    fn from(s: &'a CStr) -> Self {
        CStrArg {
            ptr: s.as_ptr(),
            _life: PhantomData,
        }
    }
}

impl<'a> From<Option<&'a CStr>> for CStrArg<'a> {
    #[inline(always)]
    fn from(s: Option<&'a CStr>) -> Self {
        s.map_or(CStrArg::NULL, CStrArg::from)
    }
}

impl fmt::Debug for CStrArg<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// A `T *out` argument that the export writes and does not read: NULL, or
/// valid for writes of a `T` for `'a`.
///
/// It has the ABI of `*mut T`. [`write`](Self::write) stores through the
/// pointer, as `out.write(v)` did: the old value is neither read nor
/// dropped, and no reference to it is formed, so `T` may be uninitialised
/// storage and the pointer may alias other arguments (each write happens
/// where the export writes it).
///
/// Invariant (established by every constructor): `ptr` is NULL, or
/// non-null, aligned for `T`, and valid for writes of a `T` for `'a`, with
/// no reference to the pointee live while this value writes it.
#[repr(transparent)]
pub struct Out<'a, T> {
    ptr: *mut T,
    _life: PhantomData<&'a mut T>,
}

impl<'a, T> Out<'a, T> {
    /// `p` as an out-parameter.
    ///
    /// # Safety
    ///
    /// `p` is NULL, or aligned and valid for writes of a `T` for `'a`, and
    /// no reference to `*p` is live while the result writes it.
    #[inline(always)]
    pub const unsafe fn from_ptr(p: *mut T) -> Self {
        Out {
            ptr: p,
            _life: PhantomData,
        }
    }

    /// The NULL out-parameter (nothing is written).
    #[inline(always)]
    pub const fn null() -> Self {
        Out {
            ptr: ptr::null_mut(),
            _life: PhantomData,
        }
    }

    /// The pointer C passed.
    #[inline(always)]
    pub const fn as_ptr(&self) -> *mut T {
        self.ptr
    }

    /// Whether C passed NULL.
    #[inline(always)]
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    /// Store `value` (C's `*out = value`). Nothing happens for a NULL `out`:
    /// an export whose C requires it rejects NULL first
    /// (`git_assert_arg!(!out.is_null(), …)`), and C's optional
    /// out-parameters are skipped, as [`write_opt`](super::export::write_opt).
    #[inline(always)]
    pub fn write(&mut self, value: T) {
        if !self.ptr.is_null() {
            // SAFETY: the type's invariant: a non-null `ptr` is aligned and
            // valid for writes of a `T` for `'a`, which this borrow of
            // `self` is within, and no reference covers the pointee.
            unsafe { self.ptr.write(value) };
        }
    }

    /// A shorter-lived out-parameter over the same pointer, to hand on.
    #[inline(always)]
    pub fn reborrow(&mut self) -> Out<'_, T> {
        Out {
            ptr: self.ptr,
            _life: PhantomData,
        }
    }
}

impl<'a, T> From<&'a mut T> for Out<'a, T> {
    #[inline(always)]
    fn from(r: &'a mut T) -> Self {
        Out {
            ptr: r,
            _life: PhantomData,
        }
    }
}

impl<'a, T> From<&'a mut MaybeUninit<T>> for Out<'a, T> {
    #[inline(always)]
    fn from(r: &'a mut MaybeUninit<T>) -> Self {
        Out {
            ptr: r.as_mut_ptr(),
            _life: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Out<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Out").field(&self.ptr).finish()
    }
}

/// The `len` elements at `ptr` as a slice, for a C `(const T *ptr, size_t
/// len)` pair: empty when `len` is 0 or `ptr` is NULL, whatever the other
/// is (C passes NULL with 0, and `from_raw_parts` needs a non-null, aligned
/// pointer even then).
///
/// A `len` whose byte size exceeds `isize::MAX` (which no readable range
/// can have, but a C `size_t` can carry) panics rather than form the slice:
/// an export's barrier turns it into an internal error. This is
/// [`export::bytes`](super::export::bytes) for any element type.
///
/// # Safety
///
/// When `len > 0` and `ptr` is not NULL: `ptr` is aligned for `T` and valid
/// for reads of `len` initialized `T`s for `'a`, and nothing writes them
/// during `'a` (except inside an `UnsafeCell` of `T`).
#[inline(always)]
pub unsafe fn slice<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if len == 0 || ptr.is_null() {
        return &[];
    }
    assert!(
        len <= isize::MAX as usize / size_of::<T>().max(1),
        "length beyond isize::MAX bytes"
    );
    // SAFETY: `ptr` is non-null, and the caller guarantees it aligned and
    // valid for reads of `len` initialized `T`s, unwritten for `'a`; their
    // size, `len * size_of::<T>()`, is at most `isize::MAX` (checked).
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// `GIT_ASSERT_ARG(x)` for an `Option<&T>` / `Option<&mut T>` argument:
/// the reference, or set `invalid argument: '<text>'` and return `$ret`
/// (`Err(GENERIC)` without it), as [`git_assert_arg!`](crate::git_assert_arg).
/// The failure of [`git_arg!`]: set `invalid argument: '<text>'` and give
/// back `ret`. Cold and out of line, and returning its argument, so that
/// the check costs the caller a test and a tail jump, not a stack frame.
///
/// It is `extern "C"` so that it cannot unwind into its caller (a panic
/// in it aborts, as in an export): the caller then needs no landing pad,
/// so no frame either. Setting the error does not panic in practice.
#[cold]
#[inline(never)]
#[doc(hidden)]
#[allow(improper_ctypes_definitions)] // called from Rust only
pub extern "C" fn arg_failed<R>(text: &str, ret: R) -> R {
    let _ = super::errors::assert_arg_failed(text);
    ret
}

/// `GIT_ASSERT_ARG(x)` for an `Option<&T>` / `Option<&mut T>` argument:
/// the reference, or set `invalid argument: '<text>'` and return `$ret`
/// (`Err(GENERIC)` without it), as [`git_assert_arg!`](crate::git_assert_arg).
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths
macro_rules! git_arg {
    ($opt:expr, $text:literal) => {
        match $opt {
            ::core::option::Option::Some(v) => v,
            ::core::option::Option::None => {
                return ::core::result::Result::Err($crate::util::errors::assert_arg_failed($text));
            }
        }
    };
    ($opt:expr, $text:literal, ()) => {
        match $opt {
            ::core::option::Option::Some(v) => v,
            ::core::option::Option::None => {
                return $crate::util::cabi::arg_failed($text, ());
            }
        }
    };
    ($opt:expr, $text:literal, $ret:expr) => {
        match $opt {
            ::core::option::Option::Some(v) => v,
            ::core::option::Option::None => {
                return $crate::util::cabi::arg_failed($text, $ret);
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstr_arg_reads_lazily_and_null_is_empty() {
        let s = CStrArg::from(c"abc");
        assert_eq!(s.to_bytes(), b"abc");
        assert_eq!(s.get(), Some(c"abc"));
        assert!(CStrArg::NULL.is_null());
        assert_eq!(CStrArg::NULL.get(), None);
        assert_eq!(CStrArg::NULL.to_bytes(), b"");
        assert_eq!(CStrArg::from(None).as_ptr(), ptr::null());
    }

    #[test]
    fn out_writes_once_and_skips_null() {
        let mut x = 1u32;
        Out::from(&mut x).write(7);
        assert_eq!(x, 7);
        let mut slot = MaybeUninit::<u64>::uninit();
        let mut o = Out::from(&mut slot);
        o.reborrow().write(3);
        o.write(4);
        // SAFETY: written just above.
        assert_eq!(unsafe { slot.assume_init() }, 4);
        // SAFETY: NULL is allowed.
        let mut n = unsafe { Out::<u32>::from_ptr(ptr::null_mut()) };
        assert!(n.is_null());
        n.write(5);
    }

    #[test]
    fn slice_maps_null_and_zero_to_empty() {
        let v = [1u32, 2, 3];
        // SAFETY: `v` is readable for 3 elements and unwritten.
        assert_eq!(unsafe { slice(v.as_ptr(), 3) }, &[1, 2, 3]);
        // SAFETY: NULL and 0 form no slice of the pointee.
        assert!(unsafe { slice::<u32>(ptr::null(), 5) }.is_empty());
        // SAFETY: as above.
        assert!(unsafe { slice::<u32>(ptr::dangling(), 0) }.is_empty());
    }

    #[test]
    #[should_panic(expected = "isize::MAX")]
    fn slice_rejects_lengths_beyond_isize_max() {
        let v = [0u64; 1];
        // SAFETY: the length check panics before any slice is formed.
        let _ = unsafe { slice(v.as_ptr(), usize::MAX / 8) };
    }

    // The ABI the exports rely on: C's pointer passes as these types (Miri
    // checks the call's ABI compatibility).
    unsafe extern "C" fn takes(s: CStrArg<'_>, mut o: Out<'_, usize>, r: Option<&u8>) -> usize {
        o.write(s.to_bytes().len());
        r.map_or(0, |v| usize::from(*v))
    }

    #[test]
    fn abi_matches_raw_pointers() {
        type Raw = unsafe extern "C" fn(*const c_char, *mut usize, *const u8) -> usize;
        // SAFETY: each argument type has the ABI of the raw pointer
        // (`#[repr(transparent)]` over it; `Option<&u8>` is guaranteed to be
        // `*const u8`).
        let f: Raw =
            unsafe { core::mem::transmute::<_, Raw>(takes as unsafe extern "C" fn(_, _, _) -> _) };
        let mut n = 0usize;
        let b = 9u8;
        // SAFETY: a string, a writable `usize`, a readable `u8`.
        let r = unsafe { f(c"four".as_ptr(), &raw mut n, &raw const b) };
        assert_eq!((n, r), (4, 9));
        // SAFETY: NULL string and reference are allowed; `n` writable.
        let r = unsafe { f(ptr::null(), &raw mut n, ptr::null()) };
        assert_eq!((n, r), (0, 0));
    }
}
