//! `deps/reftable/basics.c` and `basics.h`: reftable's allocator hooks, its growable
//! buffer and the helpers the rest of the library builds on. A foundational module:
//! its lifecycle routines are bound as strategies and stay C.
//!
//! A [`Buf`] is a `struct reftable_buf`: `len` bytes at `buf`, in an allocation of
//! `alloc` bytes from reftable's allocator that the struct owns. reftable holds it by
//! value, on the stack and embedded in its writers, iterators and records, and grows,
//! trims and releases it through `basics.c`'s routines, which keep two states:
//!
//! - unallocated, `alloc == 0`: the struct owns nothing. `buf` is NULL and `len` 0, as
//!   `REFTABLE_BUF_INIT` (all zero), `reftable_buf_init`, `reftable_buf_release` and
//!   `reftable_buf_detach` leave it; and
//! - allocated, `alloc > 0`: `buf` is a live allocation of `alloc` bytes that the struct owns,
//!   `len` is below `alloc`, and `buf[0..=len]` have been written. `buf[len]` is the NUL every
//!   `basics.c` routine writes after the contents, but not one every writer keeps:
//!   `block_writer_init` stores `last_key.len = 0` and leaves `buf[0]` as it was.
//!
//! Every handle upholds that invariant, which a crate-internal `from_ptr` over a struct C
//! owns must vouch for. The contents may hold NUL bytes, since `reftable_buf_add` takes
//! arbitrary data: [`as_bytes`](BufRef::as_bytes) reads them all, and
//! [`as_c_str`](BufRef::as_c_str) reads them as a C string only where the terminator is
//! there and no NUL comes before it. An unallocated buffer reads as empty whatever `buf`
//! and `len` hold.
//!
//! An all-zero `Buf` is the unallocated state, so [`Buf`] is `CZeroable` and
//! `Buf::zeroed()` builds a value, which owns nothing: the form a parent C struct embeds and
//! disposes itself. A buffer Rust owns is a [`BufVal`], which [`Buf::new`]
//! (`reftable_buf_init`) builds and which releases what the buffer comes to own through
//! `reftable_buf_release` when dropped ([`BufRelease`]). A bare value that comes to own an
//! allocation leaks it when dropped, as a `Vec` given to `mem::forget` does. The exclusive
//! handle appends ([`add`](BufMut::add), [`add_str`](BufMut::add_str)), trims
//! ([`truncate`](BufMut::truncate), [`reset`](BufMut::reset)) and hands the allocation out
//! ([`detach`](BufMut::detach)); the shared one compares
//! ([`Ord`], [`common_prefix_len`](BufRef::common_prefix_len)). A routine that would follow
//! `buf` of a buffer that owns nothing, whatever `buf` and `len` hold, is given the
//! unallocated state instead, so that the handles' tolerance of such a struct never reaches
//! C.
//!
//! `block_source_from_buf` keeps a buffer's address in the block source it fills, and
//! reads it for the source's life: a binding of it ties the source to the buffer's
//! borrow, so that the buffer neither moves nor changes while the source lives.
//!
//! # reftable's allocator
//!
//! reftable allocates through three process-wide hooks, `basics.c`'s statics, which
//! `reftable_set_alloc` replaces; NULL hooks fall back to libc. libgit2's first
//! initialisation installs its own allocator there (`git_reftable_global_init`, from
//! `git_libgit2_init`), so the hooks forward to `git__allocator` from then on. Each
//! allocation must be released by the family that made it, so the owners of reftable's
//! memory release through `basics.c`'s routines, and allocate only once libgit2's runtime
//! is pinned (`runtime_pinned`, as `util::errors` keeps it), which orders their reads of the
//! hooks after libgit2 wrote them. Without the runtime, an allocation fails.
//!
//! - [`ReftableFree`] holds one object ([`ReftableBox`]): `reftable_malloc`, `reftable_calloc` and
//!   `reftable_free`;
//! - [`ReftableArrayFree`] a counted run of elements ([`ReftableVec`]), which it also
//!   zero-allocates (`reftable_calloc`), resizes (`reftable_realloc`) and grows as
//!   `REFTABLE_ALLOC_GROW` does;
//! - [`ReftableStrFree`] a NUL-terminated string ([`ReftableStr`]), copied with `reftable_strdup`;
//!   and
//! - [`NamesFree`] the NULL-terminated array of strings `parse_names` builds and `free_names`
//!   releases ([`Names`]).
//!
//! [`set_alloc`] and [`set_default_alloc`] replace the hooks: `unsafe`, since replacing them
//! while another thread allocates through them races, and storage made under one family
//! and released under another is freed by the wrong allocator.
//!
//! # Errors
//!
//! reftable's routines return a negative `enum reftable_error` code; `basics.c`'s return
//! [`Error::OutOfMemory`] and [`Error::Format`] alone.

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use core::ffi::{CStr, c_int};
use core::fmt;

pub use self::ffi_import::{
    Alloc, Buf, BufMut, BufRef, BufRelease, BufVal, Names, NamesFree, ReftableArrayFree,
    ReftableBox, ReftableFree, ReftableStr, ReftableStrFree, ReftableVec, binsearch, parse_names,
    set_alloc, set_default_alloc,
};
use crate::ffi;

/// A hash function the reftable library understands (`enum reftable_hash`): the one that
/// produced the object ids a table stores, which fixes their size.
///
/// Its values are arbitrary, chosen so that a hash id left unset is easy to detect. reftable
/// reads `0` as unset, which its routines take for SHA-1: `hash_size` and the writer's
/// format version, and a stack's options, which replace it by SHA-1 when the stack is
/// opened; a closed table's id is reset to `0`. [`from_raw`](Self::from_raw) gives `None`
/// for it, as for every value without a variant, and is the only way in from an integer:
///
/// ```compile_fail,E0605
/// use libgit2::libgit2::deps::reftable::basics::HashId;
///
/// let raw: u32 = 0;
/// let _ = raw as HashId;
/// ```
///
/// Wraps `enm::reftable_hash`
#[doc(alias = "reftable_hash")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum HashId {
    /// SHA-1, whose ids are 20 bytes long.
    ///
    /// Wraps `enm::REFTABLE_HASH_SHA1`
    #[doc(alias = "REFTABLE_HASH_SHA1")]
    Sha1 = ffi::REFTABLE_HASH_SHA1,
    /// SHA-256, whose ids are 32 bytes long.
    ///
    /// Wraps `enm::REFTABLE_HASH_SHA256`
    #[doc(alias = "REFTABLE_HASH_SHA256")]
    Sha256 = ffi::REFTABLE_HASH_SHA256,
}

impl HashId {
    /// The variant whose C value is `raw`; `None` for a value `reftable-basics.h` does not
    /// name, `0`, reftable's unset id, among them.
    #[must_use]
    pub const fn from_raw(raw: ffi::reftable_hash) -> Option<Self> {
        match raw {
            ffi::REFTABLE_HASH_SHA1 => Some(Self::Sha1),
            ffi::REFTABLE_HASH_SHA256 => Some(Self::Sha256),
            _ => None,
        }
    }

    /// The C value, as reftable's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::reftable_hash {
        self as ffi::reftable_hash
    }
}

impl TryFrom<ffi::reftable_hash> for HashId {
    type Error = UnnamedHashId;

    fn try_from(raw: ffi::reftable_hash) -> Result<Self, UnnamedHashId> {
        Self::from_raw(raw).ok_or(UnnamedHashId(raw))
    }
}

impl From<HashId> for ffi::reftable_hash {
    fn from(value: HashId) -> ffi::reftable_hash {
        value.raw()
    }
}

/// A hash id that `reftable-basics.h` does not name: `0`, which reftable reads as unset, or
/// a value no table carries. [`raw`](Self::raw) has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedHashId(ffi::reftable_hash);

impl UnnamedHashId {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::reftable_hash {
        self.0
    }
}

impl fmt::Display for UnnamedHashId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reftable hash id ")?;
        fmt::Display::fmt(&self.0, f)?;
        f.write_str(" is not named by reftable-basics.h")
    }
}

impl std::error::Error for UnnamedHashId {}

impl fmt::Debug for BufRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buf")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .field("bytes", &Escaped(self.as_bytes()))
            .finish()
    }
}

/// Bytes shown as a quoted string, each byte that is not printable ASCII escaped.
struct Escaped<'a>(&'a [u8]);

impl fmt::Debug for Escaped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        fmt::Display::fmt(&self.0.escape_ascii(), f)?;
        f.write_str("\"")
    }
}

impl fmt::Debug for BufMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for Buf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// A reftable error code (`enum reftable_error`): the negative status a reftable routine
/// returns.
///
/// `basics.c`'s routines return [`OutOfMemory`](Self::OutOfMemory) and
/// [`Format`](Self::Format); [`Api`](Self::Api) is what the wrappers refuse before C sees an
/// argument it cannot take, and [`Other`](Self::Other) carries any other code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// `REFTABLE_FORMAT_ERROR`: the data is not in reftable's format.
    Format,
    /// `REFTABLE_API_ERROR`: the reftable API was misused.
    Api,
    /// `REFTABLE_OUT_OF_MEMORY_ERROR`: an allocation failed, or libgit2's runtime, which
    /// reftable's allocator needs, could not be initialised.
    OutOfMemory,
    /// Another code, as the routine returned it.
    Other(c_int),
}

impl Error {
    /// The error a reftable routine reports with the negative `code`.
    #[must_use]
    pub const fn from_code(code: c_int) -> Self {
        match code {
            ffi::REFTABLE_FORMAT_ERROR => Self::Format,
            ffi::REFTABLE_API_ERROR => Self::Api,
            ffi::REFTABLE_OUT_OF_MEMORY_ERROR => Self::OutOfMemory,
            other => Self::Other(other),
        }
    }

    /// The C code, as reftable's routines return it.
    #[must_use]
    pub const fn code(self) -> c_int {
        match self {
            Self::Format => ffi::REFTABLE_FORMAT_ERROR,
            Self::Api => ffi::REFTABLE_API_ERROR,
            Self::OutOfMemory => ffi::REFTABLE_OUT_OF_MEMORY_ERROR,
            Self::Other(code) => code,
        }
    }

    /// reftable's status convention: `0` for success, a negative code for failure.
    fn check(code: c_int) -> Result<(), Self> {
        if code == 0 {
            Ok(())
        } else {
            Err(Self::from_code(code))
        }
    }
}

impl fmt::Display for Error {
    /// The text `reftable_error_str` gives the code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format => f.write_str("corrupt reftable file"),
            Self::Api => f.write_str("misuse of the reftable API"),
            Self::OutOfMemory => f.write_str("out of memory"),
            Self::Other(code) => {
                f.write_str("reftable error code ")?;
                fmt::Display::fmt(code, f)
            }
        }
    }
}

impl std::error::Error for Error {}

/// [`BufMut::truncate`] was asked for a length beyond the buffer's contents
/// (`reftable_buf_setlen`'s `-1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LenOutOfRange;

impl fmt::Display for LenOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("length beyond the buffer's contents")
    }
}

impl std::error::Error for LenOutOfRange {}

impl Names {
    /// The names, in the order the input listed them.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CStr> + '_ {
        (0..self.len()).map(|i| self.name(i))
    }

    /// The name at `index`; `None` past the last.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&CStr> {
        (index < self.len()).then(|| self.name(index))
    }
}

impl fmt::Debug for Names {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// The C layout of [`Buf`] and its handles, the owners and policies over reftable's
/// allocator, and the calls into `basics.c`.
#[expect(
    unsafe_code,
    reason = "reads reftable_buf's fields through its handles' pointers and calls basics.c"
)]
mod ffi_import {
    use core::cmp::Ordering;
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::marker::PhantomData;
    use core::mem::{self, MaybeUninit};
    use core::num::NonZeroUsize;
    use core::ptr::{self, NonNull};
    use core::slice;

    use ffibox::callback::CClosure;
    use ffibox::{
        CAlloc, CAllocZeroed, CBox, CCell, CDispose, CDrop, CDupClone, CLenAlloc, CLenClone,
        CLenDrop, CMutSeam, CPlainElem, CRefSeam, CStrBox, CVal, CVec, CZeroable, define_ctype,
    };

    use super::{Error, HashId, LenOutOfRange};
    use crate::ffi;
    use crate::libgit2::src::util::errors::runtime_pinned;

    define_ctype!(
        /// A growable byte buffer of reftable's (`struct reftable_buf`): [`len`](BufRef::len)
        /// bytes, usually NUL-terminated, in an allocation of reftable's allocator that the
        /// struct owns, or nothing.
        ///
        /// reftable holds it by value: on the stack, and embedded in its writers, iterators
        /// and records, which reach it through [`BufRef`] and [`BufMut`]. `Buf::zeroed()`
        /// (`REFTABLE_BUF_INIT`) is an empty buffer owning nothing. Read it through
        /// [`as_ref`](Self::as_ref): the module documentation describes the states the
        /// handles read.
        ///
        /// Two exclusive handles to one buffer cannot coexist:
        ///
        /// ```compile_fail,E0499
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::zeroed();
        /// let a = buf.as_mut();
        /// let b = buf.as_mut();
        /// let _ = (a, b);
        /// ```
        ///
        /// nor a shared one beside an exclusive one:
        ///
        /// ```compile_fail,E0502
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::zeroed();
        /// let m = buf.as_mut();
        /// let r = buf.as_ref();
        /// let _ = (m, r.len());
        /// ```
        ///
        /// Wraps `st::reftable_buf`
        #[doc(alias = "reftable_buf")]
        Buf,
        BufRef,
        BufMut,
        ffi::reftable_buf
    );

    // SAFETY: all-zero is `REFTABLE_BUF_INIT`, the unallocated state: `alloc == 0`, which
    // every getter reads as empty without reading `buf`, and which `reftable_buf_release`,
    // the type's disposer, accepts (`reftable_free(NULL)`). A value points into nothing, so
    // it moves freely; the one routine that keeps a buffer's address,
    // `block_source_from_buf`, is bound to the buffer's borrow (module documentation).
    unsafe impl CZeroable for Buf {}

    impl<'a> BufRef<'a> {
        /// The `alloc` field.
        fn raw_alloc(&self) -> usize {
            let raw = self.as_ptr();
            // SAFETY: the handle addresses a live `reftable_buf` for `'a` (its invariant);
            // the projection forms no reference and reads nothing.
            let field = unsafe { &raw const (*raw).alloc };
            // SAFETY: `field` is in bounds of that struct, aligned and initialised, and no
            // exclusive handle writes it while this shared one lives.
            unsafe { field.read() }
        }

        /// The `len` field.
        fn raw_len(&self) -> usize {
            let raw = self.as_ptr();
            // SAFETY: as in `raw_alloc`.
            let field = unsafe { &raw const (*raw).len };
            // SAFETY: as in `raw_alloc`.
            unsafe { field.read() }
        }

        /// The `buf` field, NULL when the buffer is unallocated: the start of its
        /// allocation otherwise.
        fn raw_data(&self) -> *const u8 {
            if self.raw_alloc() == 0 {
                return ptr::null();
            }
            let raw = self.as_ptr();
            // SAFETY: as in `raw_alloc`.
            let field = unsafe { &raw const (*raw).buf };
            // SAFETY: as in `raw_alloc`.
            unsafe { field.read() }.cast_const().cast::<u8>()
        }

        /// The number of bytes the buffer holds, `len`: `0` when it is unallocated.
        ///
        /// Wraps `fld::reftable_buf.len`
        #[doc(alias = "reftable_buf.len")]
        #[must_use]
        pub fn len(&self) -> usize {
            if self.raw_data().is_null() {
                0
            } else {
                self.raw_len()
            }
        }

        /// Whether the buffer holds no bytes, allocated or not.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// The size of the buffer's allocation in bytes, `alloc`: room for
        /// [`len`](Self::len) bytes and their terminator at least, or `0` when the buffer
        /// owns nothing.
        ///
        /// Wraps `fld::reftable_buf.alloc`
        #[doc(alias = "reftable_buf.alloc")]
        #[must_use]
        pub fn capacity(&self) -> usize {
            if self.raw_data().is_null() {
                0
            } else {
                self.raw_alloc()
            }
        }

        /// The bytes the buffer holds, `buf[..len]`, NUL bytes included; empty when it is
        /// unallocated.
        ///
        /// Borrowed from the buffer for the handle's lifetime, while no exclusive handle can
        /// grow, trim or release it. The bytes cannot outlive the buffer:
        ///
        /// ```compile_fail,E0597
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let bytes = {
        ///     let buf = Buf::zeroed();
        ///     buf.as_ref().as_bytes()
        /// };
        /// let _ = bytes.len();
        /// ```
        ///
        /// nor the exclusive handle a shared reborrow read them through, after which the
        /// buffer can be written again:
        ///
        /// ```compile_fail,E0597
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::zeroed();
        /// let bytes = {
        ///     let m = buf.as_mut();
        ///     m.as_ref().as_bytes()
        /// };
        /// let _ = bytes.len();
        /// ```
        ///
        /// and no exclusive handle can be formed while they live:
        ///
        /// ```compile_fail,E0502
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::zeroed();
        /// let bytes = buf.as_ref().as_bytes();
        /// let m = buf.as_mut();
        /// let _ = (m, bytes.len());
        /// ```
        ///
        /// Wraps `fld::reftable_buf.buf`
        #[doc(alias = "reftable_buf.buf")]
        #[must_use]
        pub fn as_bytes(&self) -> &'a [u8] {
            let data = self.raw_data();
            if data.is_null() {
                return &[];
            }
            // SAFETY: an allocated buffer's `buf` is a live allocation of `alloc` bytes that
            // it owns, with `len < alloc` and `buf[..len]` written (the handles' invariant),
            // so the `len` bytes are readable, initialised and within one allocation, which
            // is at most `isize::MAX` bytes long. The buffer is borrowed shared for `'a`, so
            // no exclusive handle grows, trims or releases it while the slice lives.
            unsafe { slice::from_raw_parts(data, self.raw_len()) }
        }

        /// The bytes the buffer holds as a C string, `buf[..=len]`: `None` unless `buf[len]`
        /// is the NUL that ends them and none comes before it. An unallocated buffer reads
        /// as the empty string.
        ///
        /// Borrowed as [`as_bytes`](Self::as_bytes) is. reftable passes a buffer it built
        /// from strings to C routines as one, such as a table's path.
        ///
        /// ```compile_fail,E0502
        /// use ffibox::CZeroable;
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::zeroed();
        /// let c_str = buf.as_ref().as_c_str();
        /// let m = buf.as_mut();
        /// let _ = (m, c_str);
        /// ```
        ///
        /// Wraps `fld::reftable_buf.buf`
        #[doc(alias = "reftable_buf.buf")]
        #[must_use]
        pub fn as_c_str(&self) -> Option<&'a CStr> {
            let data = self.raw_data();
            if data.is_null() {
                return Some(c"");
            }
            // `len < alloc`, so the terminator's index fits.
            let with_nul = self.raw_len() + 1;
            // SAFETY: as in `as_bytes`, for `len + 1 <= alloc` bytes: `buf[len]` is within
            // the allocation and was written, as every byte up to the longest length the
            // buffer has held was.
            let bytes = unsafe { slice::from_raw_parts(data, with_nul) };
            CStr::from_bytes_with_nul(bytes).ok()
        }
    }

    // --- reftable's allocator -------------------------------------------------------------

    /// The alignment reftable's allocator vouches for: `malloc`'s, which every hook keeps
    /// ([`Alloc`]'s contract, and libgit2's `git_allocator`), for any object that fits in the
    /// request. The owners reject an element type aligned beyond it at compile time.
    const ALIGN: usize = {
        let (word, wide) = (align_of::<usize>(), align_of::<u64>());
        if word > wide { word } else { wide }
    };

    /// `reftable_malloc` for `len` bytes; `None` on failure, for `len == 0` (which C answers
    /// with NULL), or when libgit2's runtime cannot be pinned.
    fn malloc(len: usize) -> Option<NonNull<u8>> {
        if len == 0 || !runtime_pinned() {
            return None;
        }
        // SAFETY: `reftable_malloc` takes any size and returns fresh storage of reftable's
        // allocator, or NULL. The runtime is pinned, so libgit2 installed its hooks before
        // this read of them (`OnceLock` orders the read after the initialisation).
        let data = unsafe { ffi::reftable_malloc(len) };
        NonNull::new(data.cast())
    }

    /// `reftable_calloc` for `count` elements of `size` bytes, zeroed; `None` as for
    /// [`malloc`], and when the product overflows, which C refuses with NULL.
    fn calloc(count: usize, size: usize) -> Option<NonNull<u8>> {
        if count == 0 || size == 0 || !runtime_pinned() {
            return None;
        }
        // SAFETY: as in `malloc`; `reftable_calloc` checks `count * size` for overflow
        // before it allocates.
        let data = unsafe { ffi::reftable_calloc(count, size) };
        NonNull::new(data.cast())
    }

    /// The byte length of `len` elements of `T`, which a slice of them can span: at most
    /// `isize::MAX`. `None` beyond it.
    fn byte_len<T>(len: usize) -> Option<usize> {
        len.checked_mul(size_of::<T>())
            .filter(|&n| isize::try_from(n).is_ok())
    }

    /// One object in reftable's allocator, released through `reftable_free`.
    ///
    /// A storage policy: its free never reads the object, so it settles storage both before
    /// and after it holds a `T` ([`CBox::new_uninit`], [`CBox::new_zeroed`]), which
    /// `reftable_malloc` and `reftable_calloc` allocate. It releases no sub-resources, so a
    /// `T` owning any is promoted to a fuller destructor with `with_policy` before it
    /// acquires them.
    ///
    /// Types aligned beyond what the allocator vouches for are rejected:
    ///
    /// ```compile_fail,E0080
    /// use libgit2::libgit2::deps::reftable::basics::ReftableBox;
    ///
    /// #[repr(C, align(64))]
    /// pub struct wide_st { pub x: u8 }
    /// ffibox::define_ctype!(Wide, WideRef, WideMut, wide_st);
    ///
    /// let _ = ReftableBox::<Wide>::new_uninit();
    /// ```
    ///
    /// [`CBox::new_uninit`]: ffibox::CBox::new_uninit
    /// [`CBox::new_zeroed`]: ffibox::CBox::new_zeroed
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ReftableFree;

    /// One `T` in reftable's allocator.
    pub type ReftableBox<T> = CBox<T, ReftableFree>;

    // SAFETY: `reftable_free` releases the allocation exactly once, through the hook that
    // matches the `reftable_malloc` that made it, without reading it; `self` has no state.
    unsafe impl<T> CDrop<T> for ReftableFree {
        /// Wraps `fn::reftable_free`
        #[doc(alias = "reftable_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<T>) {
            // SAFETY: per `c_drop`'s contract `ptr` is a live allocation the caller owns;
            // every owner under this policy holds storage of reftable's allocator, made
            // after the runtime was pinned.
            unsafe { ffi::reftable_free(ptr.as_ptr().cast()) }
        }
    }

    // SAFETY: `c_alloc` returns fresh, uniquely-owned storage of at least `size_of::<T>()`
    // bytes, aligned to `ALIGN >= align_of::<T>()` (checked at compile time), which `c_drop`
    // releases without reading.
    unsafe impl<T: CCell> CAlloc<T> for ReftableFree {
        /// Wraps `fn::reftable_malloc`
        #[doc(alias = "reftable_malloc")]
        #[inline]
        fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
            const {
                assert!(
                    align_of::<T>() <= ALIGN,
                    "reftable's allocator cannot align T"
                );
            };
            malloc(size_of::<T>().max(1)).map(NonNull::cast)
        }
    }

    // SAFETY: as `CAlloc`, with every byte zeroed by `reftable_calloc`, a state
    // `T: CZeroable` vouches for; `c_drop` accepts any `T` since it never reads it.
    unsafe impl<T: CZeroable> CAllocZeroed<T> for ReftableFree {
        /// Wraps `fn::reftable_calloc`
        #[doc(alias = "reftable_calloc")]
        #[inline]
        fn c_alloc_zeroed(&self) -> Option<NonNull<T>> {
            const {
                assert!(
                    align_of::<T>() <= ALIGN,
                    "reftable's allocator cannot align T"
                );
            };
            calloc(1, size_of::<T>().max(1)).map(NonNull::cast)
        }
    }

    /// A counted run of elements in reftable's allocator, released through `reftable_free`:
    /// the arrays reftable grows with `REFTABLE_ALLOC_GROW`.
    ///
    /// [`CVec::from_slice`] and `Clone` copy into the allocator; [`CVec::new_uninit`] and
    /// [`new_zeroed`](Self::new_zeroed) allocate in place, and [`resize`](Self::resize) and
    /// [`grow`](Self::grow) move a run into a larger or smaller allocation. An empty run is
    /// NULL and allocates nothing.
    ///
    /// Element types aligned beyond what the allocator vouches for are rejected, as are
    /// zero-sized ones, which need no storage:
    ///
    /// ```compile_fail,E0080
    /// use libgit2::libgit2::deps::reftable::basics::ReftableVec;
    ///
    /// #[derive(Clone, Copy)]
    /// #[repr(align(64))]
    /// struct Wide(u8);
    ///
    /// let _ = ReftableVec::<Wide>::from_slice(&[Wide(0)]);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// let _ = libgit2::libgit2::deps::reftable::basics::ReftableArrayFree::new_zeroed::<()>(4);
    /// ```
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ReftableArrayFree;

    /// A counted run of `T` in reftable's allocator.
    pub type ReftableVec<T> = CVec<T, ReftableArrayFree>;

    // SAFETY: `reftable_free` releases the buffer exactly once without reading it, and
    // takes no size, so any `byte_len` within the allocation is accepted.
    unsafe impl CLenDrop for ReftableArrayFree {
        /// Wraps `fn::reftable_free`
        #[doc(alias = "reftable_free")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, _byte_len: usize) {
            // SAFETY: per `c_drop_len`'s contract `ptr` is a live allocation the caller
            // owns, made by reftable's allocator after the runtime was pinned.
            unsafe { ffi::reftable_free(ptr.cast()) }
        }
    }

    // SAFETY: `c_alloc_len` returns fresh, uniquely-owned storage of `byte_len > 0` bytes
    // from `reftable_malloc`, aligned to `ALIGN`, which `c_drop_len` releases.
    unsafe impl CLenAlloc for ReftableArrayFree {
        const ALIGN: usize = ALIGN;

        /// Wraps `fn::reftable_malloc`
        #[doc(alias = "reftable_malloc")]
        #[inline]
        fn c_alloc_len(&self, byte_len: usize) -> Option<NonNull<u8>> {
            malloc(byte_len)
        }
    }

    // SAFETY: `c_clone_len` copies `byte_len` bytes into fresh `reftable_malloc` storage of
    // at least that size, aligned to `ALIGN`, reading `src` only; `c_drop_len` releases it.
    // reftable's allocator has no copy routine of its own.
    unsafe impl CLenClone for ReftableArrayFree {
        const ALIGN: usize = ALIGN;

        /// Wraps `fn::reftable_malloc`
        #[doc(alias = "reftable_malloc")]
        #[inline]
        unsafe fn c_clone_len(&self, src: *mut u8, byte_len: usize) -> Option<NonNull<u8>> {
            let copy = malloc(byte_len.max(1))?;
            // SAFETY: the caller guarantees `byte_len` readable bytes at `src`; `copy` is a
            // fresh allocation of at least `byte_len` bytes, so the two do not overlap.
            unsafe { ptr::copy_nonoverlapping(src, copy.as_ptr(), byte_len) };
            Some(copy)
        }
    }

    impl ReftableArrayFree {
        /// `len` zeroed elements; `None` when the allocation fails, its size overflows or
        /// passes `isize::MAX` bytes, or libgit2's runtime cannot be pinned. An empty
        /// request allocates nothing.
        ///
        /// Wraps `fn::reftable_calloc`
        #[doc(alias = "reftable_calloc")]
        #[must_use]
        pub fn new_zeroed<T: CPlainElem>(len: usize) -> Option<ReftableVec<T>> {
            const {
                assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
                assert!(
                    align_of::<T>() <= ALIGN,
                    "reftable's allocator cannot align T"
                );
            };
            if len == 0 {
                return Some(CVec::empty());
            }
            byte_len::<T>(len)?;
            let data = calloc(len, size_of::<T>())?;
            // SAFETY: `data` is fresh, uniquely-owned `reftable_calloc` storage of
            // `len * size_of::<T>()` zero bytes, aligned for `T`, each element a valid `T`
            // since `T: CPlainElem` admits every bit pattern; `reftable_free` releases it.
            unsafe { CVec::from_raw_parts(data.as_ptr().cast::<T>(), len) }
        }

        /// Moves `run` into an allocation of `len` elements, as `realloc` does: the first
        /// `min(old, len)` keep their bytes, the rest are uninitialised. `len == 0` releases
        /// the allocation and leaves `run` empty.
        ///
        /// Wraps `fn::reftable_realloc`
        ///
        /// # Errors
        ///
        /// [`Error::OutOfMemory`] when the allocation fails, the size overflows or passes
        /// `isize::MAX` bytes, or libgit2's runtime cannot be pinned; `run` is left as it
        /// was.
        #[doc(alias = "reftable_realloc")]
        pub fn resize<T>(run: &mut ReftableVec<MaybeUninit<T>>, len: usize) -> Result<(), Error> {
            const {
                assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
                assert!(
                    align_of::<T>() <= ALIGN,
                    "reftable's allocator cannot align T"
                );
            };
            let Some(size) = byte_len::<T>(len) else {
                return Err(Error::OutOfMemory);
            };
            if !runtime_pinned() {
                return Err(Error::OutOfMemory);
            }
            let (data, old_len) = mem::replace(run, CVec::empty()).into_raw_parts();
            // SAFETY: `data` is NULL or the run's live allocation of reftable's allocator,
            // which the run gave up: `reftable_realloc` takes it on success and for
            // `size == 0`, and leaves it live on failure. The runtime is pinned.
            let resized = unsafe { ffi::reftable_realloc(data.cast(), size) };
            if size == 0 {
                // Released, and NULL returned: the run stays empty.
                return Ok(());
            }
            let (data, len, outcome) = match NonNull::new(resized) {
                Some(resized) => (resized.as_ptr().cast(), len, Ok(())),
                None => (data, old_len, Err(Error::OutOfMemory)),
            };
            // SAFETY: `data` is the resized allocation of at least `len` elements, or the run's
            // own, still live and unchanged, of `old_len`; reftable's allocator aligns it
            // for `T` and `reftable_free` releases it. `MaybeUninit<T>` needs no
            // initialisation.
            if let Some(restored) = unsafe { CVec::from_raw_parts_or_empty(data, len) } {
                *run = restored;
            }
            outcome
        }

        /// Makes room for at least `min_len` elements, as `REFTABLE_ALLOC_GROW` does: a run
        /// already that long is left alone, and a shorter one moves into an allocation of
        /// `max(2 * len + 1, min_len)` elements, its elements keeping their bytes.
        ///
        /// Replaces `fn::reftable_alloc_grow`, a `static inline` function of `basics.h`,
        /// which calls `reftable_realloc` as [`resize`](Self::resize) does: the wrapper
        /// bindgen generates for it includes internal headers, which a build from the
        /// installed headers does not have.
        ///
        /// # Errors
        ///
        /// As [`resize`](Self::resize); C sets `errno` to `ENOMEM` when the size overflows,
        /// which this does not.
        #[doc(alias = "reftable_alloc_grow")]
        pub fn grow<T>(run: &mut ReftableVec<MaybeUninit<T>>, min_len: usize) -> Result<(), Error> {
            if min_len <= run.len() {
                return Ok(());
            }
            // `size_t` arithmetic, as C's: the doubling wraps around, and `min_len` wins.
            let len = run.len().wrapping_mul(2).wrapping_add(1).max(min_len);
            Self::resize(run, len)
        }
    }

    /// One NUL-terminated string in reftable's allocator, released through `reftable_free`
    /// and copied through `reftable_strdup`.
    ///
    /// [`CStrBox::from_c_str`] and `Clone` copy a string into the allocator;
    /// [`BufMut::detach`] hands a buffer's allocation out as one.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ReftableStrFree;

    /// One NUL-terminated string in reftable's allocator.
    pub type ReftableStr = CStrBox<ReftableStrFree>;

    // SAFETY: `reftable_free` releases the string exactly once without reading it.
    unsafe impl CDrop<c_char> for ReftableStrFree {
        /// Wraps `fn::reftable_free`
        #[doc(alias = "reftable_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<c_char>) {
            // SAFETY: per `c_drop`'s contract `ptr` is a live string the caller owns, from
            // reftable's allocator.
            unsafe { ffi::reftable_free(ptr.as_ptr().cast()) }
        }
    }

    // SAFETY: `reftable_strdup` returns a fresh `reftable_malloc` copy of the string and its
    // terminator, or NULL, reading the source only; `c_drop` releases it.
    unsafe impl CDupClone<c_char> for ReftableStrFree {
        /// Wraps `fn::reftable_strdup`
        #[doc(alias = "reftable_strdup")]
        #[inline]
        unsafe fn c_dup(&self, ptr: NonNull<c_char>) -> Option<NonNull<c_char>> {
            if !runtime_pinned() {
                return None;
            }
            // SAFETY: per `c_dup`'s contract `ptr` is a live NUL-terminated string, which
            // `reftable_strdup` reads for the call. The runtime is pinned.
            let copy = unsafe { ffi::reftable_strdup(ptr.as_ptr()) };
            NonNull::new(copy)
        }
    }

    /// Releases a [`Names`] array through `free_names`: each string, then the array, with
    /// `reftable_free`. The array's elements are therefore bare pointers, which the run does
    /// not drop itself.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct NamesFree;

    // SAFETY: `free_names` releases every string up to the NULL terminator, then the array,
    // each exactly once through `reftable_free`, and takes no size.
    unsafe impl CLenDrop for NamesFree {
        /// Wraps `fn::free_names`
        #[doc(alias = "free_names")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, _byte_len: usize) {
            // SAFETY: per `c_drop_len`'s contract `ptr` is a live array the caller owns: a
            // `Names` run, NULL-terminated, of strings of reftable's allocator.
            unsafe { ffi::free_names(ptr.cast()) }
        }
    }

    /// The names `parse_names` reads from a newline-separated list, each a NUL-terminated
    /// string in reftable's allocator, in a NULL-terminated array of reftable's allocator,
    /// which `free_names` releases with them on drop.
    ///
    /// Read through [`len`](Self::len), [`get`](Self::get) and [`iter`](Self::iter); the
    /// names borrow the array, which cannot be released while one is read:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::deps::reftable::basics::parse_names;
    ///
    /// let names = parse_names(b"a\n").unwrap();
    /// let first = names.get(0).unwrap();
    /// drop(names);
    /// let _ = first.count_bytes();
    /// ```
    pub struct Names(CVec<*mut c_char, NamesFree>);

    impl Names {
        /// The number of names.
        #[must_use]
        pub fn len(&self) -> usize {
            // The run holds the names and their NULL terminator.
            self.0.len() - 1
        }

        /// Whether the list held no name.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// The name at `index < len()`, borrowed from the array.
        pub(super) fn name(&self, index: usize) -> &CStr {
            let name = self.0.as_slice()[index];
            // SAFETY: the elements before the terminator are live NUL-terminated strings the
            // array owns (`parse_names`' result), which nothing changes or frees while it is
            // borrowed.
            unsafe { CStr::from_ptr(name) }
        }
    }

    /// Reads the names of a newline-separated list, as reftable reads a stack's
    /// `tables.list`: each line, ended by `\n`, is a name, and empty lines are dropped.
    ///
    /// `input` is copied into a NUL-terminated buffer of Rust's for the call, since C writes
    /// over each newline.
    ///
    /// Wraps `fn::parse_names`
    ///
    /// # Errors
    ///
    /// [`Error::Format`] when a line is not ended by `\n`, the last one included, or holds a
    /// NUL byte before its newline; [`Error::OutOfMemory`] when an allocation fails or
    /// libgit2's runtime cannot be pinned; [`Error::Api`], without calling C, for an input
    /// longer than `c_int::MAX` bytes, which C's `int` size cannot hold.
    pub fn parse_names(input: &[u8]) -> Result<Names, Error> {
        let size = c_int::try_from(input.len()).map_err(|_| Error::Api)?;
        if !runtime_pinned() {
            return Err(Error::OutOfMemory);
        }
        let mut buf = Vec::with_capacity(input.len() + 1);
        buf.extend_from_slice(input);
        buf.push(0);
        let mut out: *mut *mut c_char = ptr::null_mut();
        // SAFETY: `buf` holds `size` writable bytes and the NUL after them, at which
        // `strchr` stops, live for the call; C keeps no pointer into it (each name is a
        // copy). `out` is a writable slot, written only on success. The runtime is pinned.
        let rc = unsafe { ffi::parse_names(buf.as_mut_ptr().cast(), size, &raw mut out) };
        Error::check(rc)?;
        let mut count = 0;
        loop {
            // SAFETY: on success `out` is a NULL-terminated array, so every slot up to the
            // terminator is in bounds.
            let slot = unsafe { out.add(count) };
            // SAFETY: as above; the slot is initialised.
            if unsafe { slot.read() }.is_null() {
                break;
            }
            count += 1;
        }
        // SAFETY: `out` is a fresh array of reftable's allocator, owned by the caller, of
        // `count` names and the NULL after them, which `free_names` releases with the names.
        let names = unsafe { CVec::from_raw_parts(out, count + 1) };
        names.map(Names).ok_or(Error::OutOfMemory)
    }

    /// A memory allocator reftable can run on, installed by [`set_alloc`]: reftable's
    /// counterpart of [`GlobalAlloc`](std::alloc::GlobalAlloc), without layouts, since C
    /// passes only sizes.
    ///
    /// reftable's hooks carry no state, so an implementation is a type whose associated
    /// functions reach any state they need through statics. reftable never passes them a
    /// zero size: `reftable_malloc(0)` answers NULL itself, and `reftable_realloc(p, 0)`
    /// releases `p` through [`free`](Self::free).
    ///
    /// # Safety
    ///
    /// reftable trusts these functions as it trusts C's `malloc`, `realloc` and `free`:
    ///
    /// - `malloc(len)` returns `None`, or the start of a fresh allocation of at least `len` bytes,
    ///   aligned for any object that fits in `len` bytes as `malloc` aligns, overlapping no other
    ///   live allocation of the implementation;
    /// - `realloc(None, len)` behaves as `malloc`. `realloc(Some(ptr), len)`, with `ptr` a live
    ///   allocation of the implementation, either returns `None` and leaves `ptr` live and
    ///   unchanged, or releases `ptr` and returns an allocation as `malloc` would whose first
    ///   `min(old, len)` bytes are those `ptr` held (it may be `ptr` itself);
    /// - `free(ptr)` releases `ptr`, a live allocation of the implementation; and
    /// - all three may run concurrently on any threads, and an allocation may be resized or
    ///   released on a thread other than the one that made it.
    ///
    /// None of them may call into reftable or libgit2, which may allocate through them. A
    /// panic in one aborts the process, since it cannot unwind into C.
    pub unsafe trait Alloc {
        /// Allocates at least `len` bytes; `None` on failure.
        fn malloc(len: NonZeroUsize) -> Option<NonNull<u8>>;

        /// Resizes `ptr` to `len` bytes, or allocates afresh for `None`; `None` on failure,
        /// leaving `ptr` live.
        ///
        /// # Safety
        ///
        /// `ptr` is `None` or a live allocation of this implementation that the caller
        /// owns and, on success, gives up.
        unsafe fn realloc(ptr: Option<NonNull<u8>>, len: NonZeroUsize) -> Option<NonNull<u8>>;

        /// Releases `ptr`.
        ///
        /// # Safety
        ///
        /// `ptr` is a live allocation of this implementation that the caller owns and does
        /// not use again.
        unsafe fn free(ptr: NonNull<u8>);
    }

    /// reftable's `malloc` hook over [`Alloc::malloc`].
    extern "C" fn malloc_with<A: Alloc>(len: usize) -> *mut c_void {
        NonZeroUsize::new(len)
            .and_then(A::malloc)
            .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
    }

    /// reftable's `realloc` hook over [`Alloc::realloc`]; a zero size, which
    /// `reftable_realloc` never passes, fails.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live allocation of `A` that the caller owns and, on success,
    /// gives up.
    unsafe extern "C" fn realloc_with<A: Alloc>(ptr: *mut c_void, len: usize) -> *mut c_void {
        let Some(len) = NonZeroUsize::new(len) else {
            return ptr::null_mut();
        };
        // SAFETY: per this function's contract `ptr` is NULL or a live allocation of `A`,
        // which the caller gives up on success.
        let resized = unsafe { A::realloc(NonNull::new(ptr.cast()), len) };
        resized.map_or(ptr::null_mut(), |p| p.as_ptr().cast())
    }

    /// reftable's `free` hook over [`Alloc::free`]; `free(NULL)` stops here.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live allocation of `A` that the caller gives up.
    unsafe extern "C" fn free_with<A: Alloc>(ptr: *mut c_void) {
        if let Some(ptr) = NonNull::new(ptr.cast()) {
            // SAFETY: per this function's contract a non-NULL `ptr` is a live allocation of
            // `A`, which the caller gives up.
            unsafe { A::free(ptr) }
        }
    }

    /// Makes reftable allocate through `A`: its three hooks become trampolines into `A`.
    ///
    /// Wraps `fn::reftable_set_alloc`
    ///
    /// # Safety
    ///
    /// - No other thread may allocate, resize or release reftable memory during the call: the hooks
    ///   are statics C writes without synchronisation.
    /// - Every allocation of reftable's allocator that is live when the hooks change — a
    ///   [`ReftableBox`], [`ReftableVec`], [`ReftableStr`], [`Names`] or allocated [`Buf`], or
    ///   reftable memory C holds — must be released or resized only by the family that made it:
    ///   none may be live unless `A` and the hooks it replaces release each other's allocations.
    /// - libgit2's runtime must be initialised for as long as `A` stays installed: its first
    ///   initialisation, and the next one after a shutdown that ends it, install libgit2's own
    ///   allocator over `A` (`git_reftable_global_init`).
    #[doc(alias = "reftable_set_alloc")]
    pub unsafe fn set_alloc<A: Alloc>() {
        // SAFETY: the trampolines meet the hooks' contracts, as `A` meets `Alloc`'s; the
        // caller excludes concurrent use of the hooks and allocations of another family.
        unsafe {
            ffi::reftable_set_alloc(
                Some(malloc_with::<A>),
                Some(realloc_with::<A>),
                Some(free_with::<A>),
            );
        }
    }

    /// Makes reftable allocate through libc's `malloc`, `realloc` and `free`: NULL hooks.
    ///
    /// Wraps `fn::reftable_set_alloc`
    ///
    /// # Safety
    ///
    /// As [`set_alloc`], with libc's allocator as the new family.
    #[doc(alias = "reftable_set_alloc")]
    pub unsafe fn set_default_alloc() {
        // SAFETY: NULL hooks select libc's allocator; the caller excludes concurrent use of
        // the hooks and allocations of another family.
        unsafe { ffi::reftable_set_alloc(None, None, None) }
    }

    // --- Helpers -----------------------------------------------------------------------

    /// Finds the smallest index `i < len` at which `f(i)` is `true`, assuming `f` ascends:
    /// `false` at every index below it and `true` from it on; `len` when `f` is `false`
    /// everywhere. A binary search, so `f` sees `O(log len)` indices, each below `len`.
    ///
    /// `f` stops the search with an error, which is returned as it is. C's final probe of
    /// index 0 reads a failure as `true` and answers 0; the error is returned there too.
    /// A panic in `f` stops the search too, and resumes once C has returned.
    ///
    /// Wraps `fn::binsearch`
    ///
    /// # Errors
    ///
    /// The first error `f` returns.
    pub fn binsearch<E>(
        len: usize,
        mut f: impl FnMut(usize) -> Result<bool, E>,
    ) -> Result<usize, E> {
        let mut error = None;
        let mut probe = |k: usize| -> c_int {
            match f(k) {
                Ok(found) => c_int::from(found),
                Err(e) => {
                    error = Some(e);
                    -1
                }
            }
        };
        // SAFETY: `binsearch` calls `f` synchronously, on the calling thread only, and
        // keeps neither it nor its context after returning.
        let callback = unsafe { CClosure::new_local(&mut probe, -1) };
        // SAFETY: the trampoline and context belong to `callback`, which outlives the call;
        // `binsearch` passes them back with indices below `len` alone.
        let found = unsafe { ffi::binsearch(len, Some(callback.trampoline()), callback.ctx()) };
        callback.finish();
        match error {
            Some(e) => Err(e),
            None => Ok(found),
        }
    }

    impl HashId {
        /// The size of the object ids the hash function produces, in bytes: 20 for SHA-1, 32
        /// for SHA-256. reftable reads its unset id, `0`, which no variant names, as SHA-1.
        ///
        /// Wraps `fn::hash_size`
        #[doc(alias = "hash_size")]
        #[must_use]
        pub fn size(self) -> u32 {
            // SAFETY: `hash_size` reads its argument alone, and every `HashId` is a value
            // its switch names, so it never reaches its `abort()`.
            unsafe { ffi::hash_size(self.raw()) }
        }
    }

    // --- The buffer's lifecycle and routines -----------------------------------------

    /// Releases what a buffer owns through `reftable_buf_release`, which hands `buf` to
    /// `reftable_free` and reinitialises the struct, Rust's storage.
    ///
    /// A buffer that owns nothing is not passed to C: there is nothing to free, and the call
    /// would be `reftable_free(NULL)`, which reads the free hook without synchronisation
    /// against the first `git_libgit2_init` another thread may be running.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct BufRelease;

    // SAFETY: `reftable_buf_release` frees `buf` exactly once and never the struct. It is
    // called only for an allocated buffer, whose allocation reftable's allocator made after
    // the runtime was pinned; one that owns nothing is released without C. The all-zero
    // state owns nothing, as `CZeroable` requires of the teardown.
    unsafe impl CDispose<Buf> for BufRelease {
        /// Wraps `fn::reftable_buf_release`
        #[doc(alias = "reftable_buf_release")]
        #[inline]
        unsafe fn c_dispose(&self, ptr: NonNull<Buf>) {
            // SAFETY: per `c_dispose`'s contract `ptr` addresses a live `Buf`, which the
            // caller gives up; the handle reads it in a state its invariant admits.
            let mut handle = unsafe { BufMut::from_ptr(ptr.as_ptr().cast()) }
                .expect("a NonNull pointer adopts a handle");
            if handle.as_ref().raw_data().is_null() {
                return;
            }
            // SAFETY: an allocated buffer owns `buf`, an allocation of reftable's allocator;
            // the caller gives the value up, so it is released once.
            unsafe { ffi::reftable_buf_release(handle.as_mut_ptr()) }
        }
    }

    /// An owned [`Buf`], held inline; what it comes to own is released by
    /// `reftable_buf_release` on drop.
    ///
    /// Not `Clone`: a byte copy would release the allocation twice.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::deps::reftable::basics::{Buf, BufVal};
    ///
    /// let buf = Buf::new();
    /// let copy: BufVal = buf.clone();
    /// ```
    pub type BufVal = CVal<Buf, BufRelease>;

    impl Buf {
        /// An empty buffer owning nothing, as `reftable_buf_init` leaves one: the storage
        /// that [`add`](BufMut::add) fills.
        ///
        /// Wraps `fn::reftable_buf_init`
        #[doc(alias = "reftable_buf_init")]
        pub fn new() -> BufVal {
            let mut raw = MaybeUninit::<ffi::reftable_buf>::uninit();
            // SAFETY: `reftable_buf_init` writes `REFTABLE_BUF_INIT` over the whole struct
            // without reading it.
            unsafe { ffi::reftable_buf_init(raw.as_mut_ptr()) };
            // SAFETY: initialised by the call above.
            let raw = unsafe { raw.assume_init() };
            // SAFETY: an unallocated buffer owns nothing, which `BufRelease` accepts.
            unsafe { CVal::new(Self(raw, PhantomData)) }
        }
    }

    impl BufRef<'_> {
        /// The buffer as C reads it: a copy of the struct in the state its handles read,
        /// unallocated (`REFTABLE_BUF_INIT`) for one that owns nothing, so that C never
        /// follows a `buf` the buffer does not own.
        fn view(&self) -> ffi::reftable_buf {
            ffi::reftable_buf {
                alloc: self.capacity(),
                len: self.len(),
                buf: self.raw_data().cast_mut().cast(),
            }
        }

        /// The length of the longest prefix the two buffers' contents share.
        ///
        /// Wraps `fn::common_prefix_size`
        #[doc(alias = "common_prefix_size")]
        #[must_use]
        pub fn common_prefix_len(&self, other: BufRef<'_>) -> usize {
            let (mut a, mut b) = (self.view(), other.view());
            // SAFETY: `a` and `b` are live structs for the call, each unallocated or over
            // `len` readable bytes the buffers own and that no exclusive handle changes
            // while these shared ones live; C only reads them.
            unsafe { ffi::common_prefix_size(&raw mut a, &raw mut b) }
        }
    }

    impl Ord for BufRef<'_> {
        /// Compares the contents bytewise, a prefix before what extends it, as `memcmp`
        /// orders them.
        ///
        /// Wraps `fn::reftable_buf_cmp`
        #[doc(alias = "reftable_buf_cmp")]
        fn cmp(&self, other: &Self) -> Ordering {
            let (a, b) = (self.view(), other.view());
            // SAFETY: as in `common_prefix_len`.
            let order = unsafe { ffi::reftable_buf_cmp(&raw const a, &raw const b) };
            order.cmp(&0)
        }
    }

    impl PartialOrd for BufRef<'_> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl PartialEq for BufRef<'_> {
        /// Whether the contents are equal, as [`Ord`] compares them.
        fn eq(&self, other: &Self) -> bool {
            self.cmp(other) == Ordering::Equal
        }
    }

    impl Eq for BufRef<'_> {}

    impl BufMut<'_> {
        /// Brings a buffer that owns nothing to `REFTABLE_BUF_INIT`, so that a routine that
        /// writes it never follows a `buf` it does not own. An allocated one is left alone.
        fn settle(&mut self) {
            if self.as_ref().raw_data().is_null() {
                // SAFETY: `reftable_buf_init` overwrites the struct without reading it; the
                // buffer owns nothing, so nothing is lost.
                unsafe { ffi::reftable_buf_init(self.as_mut_ptr()) }
            }
        }

        /// Whether the contents can grow by `extra` bytes and stay within a slice's
        /// `isize::MAX` bytes, their terminator included.
        fn fits(&self, extra: usize) -> bool {
            self.as_ref()
                .len()
                .checked_add(extra)
                .is_some_and(|n| n < isize::MAX.cast_unsigned())
        }

        /// Empties the buffer, keeping its allocation for what comes next.
        ///
        /// Wraps `fn::reftable_buf_reset`
        #[doc(alias = "reftable_buf_reset")]
        pub fn reset(&mut self) {
            self.settle();
            // SAFETY: the handle addresses a live buffer, unallocated or owning `buf`;
            // `reftable_buf_reset` writes `len` and `buf[0]` of an allocated one alone.
            unsafe { ffi::reftable_buf_reset(self.as_mut_ptr()) }
        }

        /// Shortens the contents to their first `len` bytes, writing the terminator after
        /// them; the allocation is kept.
        ///
        /// Wraps `fn::reftable_buf_setlen`
        ///
        /// # Errors
        ///
        /// [`LenOutOfRange`], leaving the buffer as it was, when `len` passes the contents.
        #[doc(alias = "reftable_buf_setlen")]
        pub fn truncate(&mut self, len: usize) -> Result<(), LenOutOfRange> {
            self.settle();
            // SAFETY: as in `reset`; `reftable_buf_setlen` writes `buf[len]` and `len` only
            // for `len < self.len`, which is within an allocated buffer (`len < alloc`).
            let rc = unsafe { ffi::reftable_buf_setlen(self.as_mut_ptr(), len) };
            if rc == 0 { Ok(()) } else { Err(LenOutOfRange) }
        }

        /// Appends `data`, NUL bytes included, and the terminator after it, growing the
        /// allocation as needed.
        ///
        /// Wraps `fn::reftable_buf_add`
        ///
        /// # Errors
        ///
        /// [`Error::OutOfMemory`], leaving the buffer as it was, when the allocation fails,
        /// the contents would pass `isize::MAX` bytes, or libgit2's runtime cannot be pinned.
        #[doc(alias = "reftable_buf_add")]
        pub fn add(&mut self, data: &[u8]) -> Result<(), Error> {
            if !self.fits(data.len()) || !runtime_pinned() {
                return Err(Error::OutOfMemory);
            }
            self.settle();
            // SAFETY: the handle addresses a live buffer, unallocated or owning `buf`, which
            // `reftable_buf_add` grows through reftable's allocator (the runtime is pinned);
            // `data` is `data.len()` readable bytes that cannot overlap the buffer, which
            // this exclusive handle borrows.
            let rc = unsafe {
                ffi::reftable_buf_add(self.as_mut_ptr(), data.as_ptr().cast(), data.len())
            };
            Error::check(rc)
        }

        /// Appends `s`, without its terminator, and the terminator after it, as
        /// [`add`](Self::add) does.
        ///
        /// Wraps `fn::reftable_buf_addstr`
        ///
        /// # Errors
        ///
        /// As [`add`](Self::add).
        #[doc(alias = "reftable_buf_addstr")]
        pub fn add_str(&mut self, s: &CStr) -> Result<(), Error> {
            if !self.fits(s.count_bytes()) || !runtime_pinned() {
                return Err(Error::OutOfMemory);
            }
            self.settle();
            // SAFETY: as in `add`, with `s` a NUL-terminated string `strlen` reads for the
            // call.
            let rc = unsafe { ffi::reftable_buf_addstr(self.as_mut_ptr(), s.as_ptr()) };
            Error::check(rc)
        }

        /// Hands the buffer's allocation out as a string, leaving the buffer empty and owning
        /// nothing; `None`, for a buffer that owned nothing.
        ///
        /// The string reads up to the first NUL byte: all of the contents unless they hold
        /// one, after which the rest is released with the string, unread. A terminator the
        /// buffer lacks is written at `buf[len]` first.
        ///
        /// The contents cannot be read past it, through a borrow taken before:
        ///
        /// ```compile_fail,E0502
        /// use libgit2::libgit2::deps::reftable::basics::Buf;
        ///
        /// let mut buf = Buf::new();
        /// buf.as_mut().add(b"name").unwrap();
        /// let bytes = buf.as_ref().as_bytes();
        /// let detached = buf.as_mut().detach();
        /// let _ = (bytes.len(), detached);
        /// ```
        ///
        /// Wraps `fn::reftable_buf_detach`
        #[doc(alias = "reftable_buf_detach")]
        pub fn detach(&mut self) -> Option<ReftableStr> {
            self.settle();
            let data = self.as_ref().raw_data().cast_mut();
            if !data.is_null() {
                let end = self.as_ref().raw_len();
                // SAFETY: an allocated buffer has `len < alloc`, so `buf[len]` is within its
                // allocation.
                let terminator = unsafe { data.add(end) };
                // SAFETY: the byte is the buffer's, which this exclusive handle may write.
                unsafe { terminator.write(0) };
            }
            // SAFETY: the handle addresses a live buffer, unallocated or owning `buf`;
            // `reftable_buf_detach` returns `buf` and reinitialises the struct.
            let detached = unsafe { ffi::reftable_buf_detach(self.as_mut_ptr()) };
            // SAFETY: `detached` is NULL for an unallocated buffer, or the allocation of
            // reftable's allocator it owned, NUL-terminated at `buf[len]` at the latest,
            // which the caller now owns and `reftable_free` releases.
            unsafe { ReftableStr::from_raw(detached) }
        }
    }

    #[cfg(test)]
    mod unit_priv {
        //! The getters over allocated buffers, which only `basics.c`'s routines make: the
        //! fixture builds the C struct over Rust storage, as one of them leaves it, and
        //! reads it through a handle.

        use core::cmp::Ordering;
        use core::ffi::{CStr, c_char, c_void};
        use core::mem::MaybeUninit;
        use core::num::NonZeroUsize;
        use core::ptr::{self, NonNull};
        use core::sync::atomic::AtomicIsize;
        use core::sync::atomic::Ordering::SeqCst;

        use ffibox::{CDispose, CMutSeam, CRefSeam, CVec, CZeroable};

        use super::{
            Alloc, Buf, BufMut, BufRef, BufRelease, Error, LenOutOfRange, Names, ReftableArrayFree,
            ReftableBox, ReftableStr, ReftableVec, parse_names, runtime_pinned, set_alloc,
        };
        use crate::ffi;

        /// A `reftable_buf` over `storage`, as reftable leaves one: `alloc` bytes, of which
        /// `len` hold the contents. The caller keeps `storage` alive and unmoved while it
        /// reads the result.
        fn raw_over(storage: &mut [u8], len: usize) -> ffi::reftable_buf {
            assert!(len < storage.len(), "an allocated buffer has len < alloc");
            ffi::reftable_buf {
                alloc: storage.len(),
                len,
                buf: storage.as_mut_ptr().cast::<c_char>(),
            }
        }

        /// Reads `raw` through a shared handle.
        fn with_ref<R>(raw: &mut ffi::reftable_buf, read: impl FnOnce(BufRef<'_>) -> R) -> R {
            let ptr: *mut ffi::reftable_buf = raw;
            // SAFETY: `raw` is a live `reftable_buf` borrowed for this call, in a state its
            // handles accept (`raw_over`'s), over storage the caller keeps alive.
            let handle = unsafe { BufRef::from_ptr(ptr) }.expect("non-null");
            read(handle)
        }

        #[test]
        fn allocated_buffer_reads_its_contents_and_terminator() {
            let mut storage = *b"refs/heads/main\0\xAA\xAA";
            let mut raw = raw_over(&mut storage, 15);
            with_ref(&mut raw, |b| {
                assert_eq!(b.len(), 15);
                assert!(!b.is_empty());
                assert_eq!(b.capacity(), 18);
                assert_eq!(b.as_bytes(), b"refs/heads/main");
                assert_eq!(b.as_c_str(), Some(c"refs/heads/main"));
            });
        }

        #[test]
        fn interior_nul_is_kept_in_bytes_and_refused_as_c_string() {
            let mut storage = *b"a\0b\0";
            let mut raw = raw_over(&mut storage, 3);
            with_ref(&mut raw, |b| {
                assert_eq!(b.as_bytes(), b"a\0b");
                assert_eq!(b.as_c_str(), None);
                assert_eq!(
                    format!("{b:?}"),
                    r#"Buf { len: 3, capacity: 4, bytes: "a\x00b" }"#
                );
            });
        }

        #[test]
        fn missing_terminator_is_refused_as_c_string() {
            // `block_writer_init` trims `last_key` to 0 without writing `buf[0]`.
            let mut storage = *b"key\0";
            let mut raw = raw_over(&mut storage, 0);
            with_ref(&mut raw, |b| {
                assert_eq!(b.len(), 0);
                assert!(b.is_empty());
                assert_eq!(b.capacity(), 4);
                assert_eq!(b.as_bytes(), b"");
                assert_eq!(b.as_c_str(), None);
            });
            let mut storage = *b"abcd";
            let mut raw = raw_over(&mut storage, 2);
            with_ref(&mut raw, |b| {
                assert_eq!(b.as_bytes(), b"ab");
                assert_eq!(b.as_c_str(), None);
            });
        }

        #[test]
        fn allocated_empty_buffer_reads_as_the_empty_string() {
            let mut storage = [0u8; 1];
            let mut raw = raw_over(&mut storage, 0);
            with_ref(&mut raw, |b| {
                assert_eq!(b.len(), 0);
                assert_eq!(b.capacity(), 1);
                assert_eq!(b.as_bytes(), b"");
                assert_eq!(b.as_c_str(), Some(c""));
            });
        }

        #[test]
        fn unallocated_buffer_reads_as_empty_whatever_its_other_fields_hold() {
            let mut storage = *b"stale\0";
            let mut raw = ffi::reftable_buf {
                alloc: 0,
                len: 5,
                buf: storage.as_mut_ptr().cast::<c_char>(),
            };
            with_ref(&mut raw, |b| {
                assert_eq!(b.len(), 0);
                assert_eq!(b.capacity(), 0);
                assert_eq!(b.as_bytes(), b"");
                assert_eq!(b.as_c_str(), Some(c""));
            });
            let mut raw = ffi::reftable_buf {
                alloc: 0,
                len: 7,
                buf: ptr::null_mut(),
            };
            with_ref(&mut raw, |b| assert_eq!(b.as_bytes(), b""));
        }

        #[test]
        fn allocated_size_with_a_null_buffer_reads_as_empty() {
            let mut raw = ffi::reftable_buf {
                alloc: 8,
                len: 3,
                buf: ptr::null_mut(),
            };
            with_ref(&mut raw, |b| {
                assert_eq!(b.len(), 0);
                assert_eq!(b.capacity(), 0);
                assert_eq!(b.as_bytes(), b"");
                assert_eq!(b.as_c_str(), Some(c""));
            });
        }

        #[test]
        fn handles_address_the_value_and_null_adopts_nothing() {
            let mut value = Buf::zeroed();
            let expected: *const ffi::reftable_buf = ptr::from_ref(&value).cast();
            assert_eq!(value.as_ref().as_ptr(), expected);
            assert_eq!(value.as_mut().as_ref().as_ptr(), expected);
            // SAFETY: a null pointer adopts nothing.
            assert!(unsafe { BufRef::from_ptr(ptr::null_mut()) }.is_none());
        }

        #[test]
        fn bytes_borrow_the_storage_they_read() {
            let mut storage = *b"xyz\0";
            let start = storage.as_ptr();
            let mut raw = raw_over(&mut storage, 3);
            let (bytes, c_str) = with_ref(&mut raw, |b| {
                (b.as_bytes().as_ptr(), b.as_c_str().map(CStr::as_ptr))
            });
            assert_eq!(bytes, start);
            assert_eq!(c_str.map(<*const c_char>::cast::<u8>), Some(start));
        }

        // --- The routines over states only C or a crate-internal adoption builds -------

        /// A handle over `raw`, as a crate-internal adoption of a C-owned buffer makes one.
        fn with_mut<R>(raw: &mut ffi::reftable_buf, write: impl FnOnce(BufMut<'_>) -> R) -> R {
            let ptr: *mut ffi::reftable_buf = raw;
            // SAFETY: `raw` is a live `reftable_buf` borrowed for this call; the tests below
            // build the states the handles tolerate, over storage the caller keeps alive.
            let handle = unsafe { BufMut::from_ptr(ptr) }.expect("non-null");
            write(handle)
        }

        /// Releases what a raw buffer owns, as `BufRelease` does for a `BufVal`.
        fn release(raw: &mut ffi::reftable_buf) {
            let ptr = NonNull::from(raw).cast::<Buf>();
            // SAFETY: `ptr` addresses a live buffer the caller gives up.
            unsafe { BufRelease.c_dispose(ptr) };
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn new_buffer_is_reftable_buf_init() {
            let value = Buf::new();
            let ptr = value.as_ref().as_ptr();
            // SAFETY: the handle addresses the owner's live value, read once.
            let raw = unsafe { ptr.read() };
            assert_eq!((raw.alloc, raw.len), (0, 0));
            assert!(raw.buf.is_null());
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn writers_settle_a_struct_that_owns_nothing_before_c_sees_it() {
            // `alloc == 0` owns nothing, whatever `buf` and `len` hold: C must not follow
            // them (it would write into, or reallocate, storage the buffer does not own).
            let mut storage = *b"stale\0";
            let stale = |storage: &mut [u8; 6]| ffi::reftable_buf {
                alloc: 0,
                len: 5,
                buf: storage.as_mut_ptr().cast::<c_char>(),
            };
            let mut raw = stale(&mut storage);
            assert_eq!(
                with_mut(&mut raw, |mut b| b.truncate(2)),
                Err(LenOutOfRange)
            );
            assert!(raw.buf.is_null(), "settled to REFTABLE_BUF_INIT");
            let mut raw = stale(&mut storage);
            with_mut(&mut raw, |mut b| b.reset());
            assert_eq!((raw.alloc, raw.len), (0, 0));
            let mut raw = stale(&mut storage);
            assert_eq!(with_mut(&mut raw, |mut b| b.detach()), None);
            let mut raw = stale(&mut storage);
            with_mut(&mut raw, |mut b| b.add(b"new")).unwrap();
            assert_ne!(
                raw.buf.cast::<u8>(),
                storage.as_mut_ptr(),
                "a fresh allocation"
            );
            with_ref(&mut raw, |b| assert_eq!(b.as_bytes(), b"new"));
            release(&mut raw);
            // A size with a NULL `buf` owns nothing either.
            let mut raw = ffi::reftable_buf {
                alloc: 8,
                len: 3,
                buf: ptr::null_mut(),
            };
            with_mut(&mut raw, |mut b| b.add_str(c"x")).unwrap();
            with_ref(&mut raw, |b| assert_eq!(b.as_c_str(), Some(c"x")));
            release(&mut raw);
            assert_eq!(&storage, b"stale\0", "never written");
        }

        #[test]
        fn release_leaves_storage_it_does_not_own_to_its_owner() {
            let mut storage = *b"not reftable's\0";
            let mut raw = ffi::reftable_buf {
                alloc: 0,
                len: 3,
                buf: storage.as_mut_ptr().cast::<c_char>(),
            };
            // `reftable_free` of Rust's storage would be an invalid free (ASan).
            release(&mut raw);
            assert_eq!(
                raw.buf.cast::<u8>(),
                storage.as_mut_ptr(),
                "not passed to C"
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn comparisons_read_a_struct_that_owns_nothing_as_empty() {
            let mut a = ffi::reftable_buf {
                alloc: 0,
                len: 4,
                buf: ptr::null_mut(),
            };
            let mut b = ffi::reftable_buf {
                alloc: 9,
                len: 2,
                buf: ptr::null_mut(),
            };
            let mut c = Buf::new();
            c.as_mut().add(b"ab").unwrap();
            let c = c.as_ref();
            with_ref(&mut a, |a| {
                with_ref(&mut b, |b| {
                    assert_eq!(a.cmp(&b), Ordering::Equal);
                    assert_eq!(a.common_prefix_len(b), 0);
                    assert_eq!(a.cmp(&c), Ordering::Less);
                    assert_eq!(c.cmp(&b), Ordering::Greater);
                    assert_eq!(c.common_prefix_len(a), 0);
                });
            });
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn detach_writes_a_terminator_the_buffer_lacks() {
            let mut value = Buf::new();
            value.as_mut().add(b"key").unwrap();
            let ptr = value.as_mut().as_mut_ptr();
            // `block_writer_init` trims `last_key` to 0 without writing `buf[0]`.
            // SAFETY: `ptr` addresses the owner's live value; `len = 0 < alloc` keeps it a
            // state the handles accept.
            let len = unsafe { &raw mut (*ptr).len };
            // SAFETY: as above.
            unsafe { len.write(0) };
            assert_eq!(value.as_ref().as_c_str(), None, "no terminator at buf[0]");
            let s = value.as_mut().detach().unwrap();
            assert_eq!(s.as_c_str(), c"");
            assert_eq!(value.as_ref().capacity(), 0);
        }

        #[test]
        fn writers_refuse_contents_past_isize_max_before_c() {
            // Never dereferenced: the size check comes first.
            let mut raw = ffi::reftable_buf {
                alloc: usize::MAX,
                len: isize::MAX.cast_unsigned() - 2,
                buf: ptr::dangling_mut(),
            };
            with_mut(&mut raw, |mut b| {
                assert_eq!(b.add(b"xy"), Err(Error::OutOfMemory));
                assert_eq!(b.add_str(c"xy"), Err(Error::OutOfMemory));
            });
            assert_eq!(raw.len, isize::MAX.cast_unsigned() - 2);
        }

        #[test]
        fn names_count_and_borrow_their_strings() {
            // An array as `parse_names` leaves one, over Rust's storage, read and given up
            // without `free_names`.
            let mut strings = [
                c"a".as_ptr().cast_mut(),
                c"bc".as_ptr().cast_mut(),
                ptr::null_mut(),
            ];
            // SAFETY: three initialised elements, the last the terminator; `NamesFree` is
            // never run on them (the run is given up below).
            let run = unsafe { CVec::from_raw_parts(strings.as_mut_ptr(), 3) }.unwrap();
            let names = Names(run);
            assert_eq!(names.len(), 2);
            assert!(!names.is_empty());
            assert_eq!(names.name(1), c"bc");
            assert_eq!(names.get(2), None);
            assert_eq!(format!("{names:?}"), r#"["a", "bc"]"#);
            let _ = names.0.into_raw_parts();
        }

        // --- Allocation failures, in child processes: reftable's hooks are global --------

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, where nothing has initialised libgit2.
        fn isolated(test: &str, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// How many more requests [`Countdown`] grants; negative grants all.
        static GRANTS: AtomicIsize = AtomicIsize::new(-1);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut c_void;
            fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
            fn free(p: *mut c_void);
        }

        /// libc's allocator, which refuses every request once `GRANTS` runs out.
        struct Countdown;

        impl Countdown {
            fn grant() -> bool {
                GRANTS
                    .try_update(SeqCst, SeqCst, |n| match n {
                        0 => None,
                        n if n < 0 => Some(n),
                        n => Some(n - 1),
                    })
                    .is_ok()
            }
        }

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Countdown {
            fn malloc(len: NonZeroUsize) -> Option<NonNull<u8>> {
                if !Self::grant() {
                    return None;
                }
                // SAFETY: a plain libc allocation.
                NonNull::new(unsafe { malloc(len.get()) }.cast())
            }

            unsafe fn realloc(p: Option<NonNull<u8>>, len: NonZeroUsize) -> Option<NonNull<u8>> {
                if !Self::grant() {
                    return None;
                }
                let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
                // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
                NonNull::new(unsafe { realloc(p, len.get()) }.cast())
            }

            unsafe fn free(p: NonNull<u8>) {
                // SAFETY: `p` is a live libc block the caller gives up.
                unsafe { free(p.as_ptr().cast()) }
            }
        }

        /// Pins the runtime, which installs libgit2's hooks, then installs `Countdown`.
        fn install_countdown() {
            assert!(runtime_pinned());
            // SAFETY: the child runs this test alone, before anything allocates through
            // reftable, and keeps libgit2's runtime pinned; `Countdown` is libc's family.
            unsafe { set_alloc::<Countdown>() };
        }

        /// Runs `attempt` with the first `n` requests granted, for `n = 0, 1, ...`, until it
        /// succeeds; returns how many requests it took.
        fn sweep(mut attempt: impl FnMut() -> bool) -> isize {
            for n in 0.. {
                GRANTS.store(n, SeqCst);
                let done = attempt();
                GRANTS.store(-1, SeqCst);
                if done {
                    return n;
                }
                assert!(n < 64, "never succeeded");
            }
            unreachable!()
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn every_allocation_failure_is_reported_and_leaves_the_owner_as_it_was() {
            isolated(
                concat!(
                    module_path!(),
                    "::every_allocation_failure_is_reported_and_leaves_the_owner_as_it_was"
                ),
                || {
                    install_countdown();
                    let mut value = Buf::new();
                    value.as_mut().add(b"kept").unwrap();
                    let steps = sweep(|| {
                        let big = [b'x'; 64];
                        match value.as_mut().add(&big) {
                            Ok(()) => true,
                            Err(e) => {
                                assert_eq!(e, Error::OutOfMemory);
                                assert_eq!(value.as_ref().as_bytes(), b"kept");
                                false
                            }
                        }
                    });
                    assert_eq!(steps, 1, "one reallocation");
                    let mut value = Buf::new();
                    assert_eq!(
                        sweep(|| value.as_mut().add_str(c"s").is_ok()),
                        1,
                        "one allocation"
                    );
                    // `parse_names` frees what it built at each failure (`LSan`).
                    let steps = sweep(|| match parse_names(b"a\nb\nc\n") {
                        Ok(names) => names.len() == 3,
                        Err(e) => {
                            assert_eq!(e, Error::OutOfMemory);
                            false
                        }
                    });
                    assert!(
                        steps >= 4,
                        "the array grows and each name is copied: {steps}"
                    );
                    assert_eq!(sweep(|| ReftableStr::from_c_str(c"dup").is_some()), 1);
                    assert_eq!(sweep(|| ReftableVec::<u8>::from_slice(b"run").is_some()), 1);
                    assert_eq!(
                        sweep(|| ReftableArrayFree::new_zeroed::<u32>(3).is_some()),
                        1
                    );
                    assert_eq!(sweep(|| ReftableBox::<Buf>::new_zeroed().is_some()), 1);
                    assert_eq!(sweep(|| ReftableBox::<Buf>::new_uninit().is_some()), 1);
                    let mut run = ReftableVec::<u16>::new_uninit(2).unwrap();
                    run.as_mut_slice()[0].write(7);
                    let steps = sweep(|| match ReftableArrayFree::grow(&mut run, 3) {
                        Ok(()) => true,
                        Err(e) => {
                            assert_eq!((e, run.len()), (Error::OutOfMemory, 2));
                            false
                        }
                    });
                    assert_eq!((steps, run.len()), (1, 5));
                    // SAFETY: written above; a resize keeps the bytes it moves.
                    assert_eq!(unsafe { run.as_slice()[0].assume_init() }, 7);
                },
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn nothing_allocates_without_the_runtime() {
            isolated(
                concat!(module_path!(), "::nothing_allocates_without_the_runtime"),
                || {
                    use crate::libgit2::src::util::alloc::Allocator;
                    /// libgit2's allocator refusing everything, so that its
                    /// initialisation fails.
                    struct Refusing;
                    // SAFETY: every request fails, which `Alloc` allows.
                    unsafe impl crate::libgit2::src::util::alloc::Alloc for Refusing {
                        fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                            None
                        }

                        unsafe fn realloc(
                            _: Option<NonNull<u8>>,
                            _: usize,
                            _: Option<&CStr>,
                            _: i32,
                        ) -> Option<NonNull<u8>> {
                            None
                        }

                        unsafe fn free(_: NonNull<u8>) {}
                    }
                    let allocator = Allocator::new::<Refusing>();
                    // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`,
                    // every member set; libgit2 copies it before anything is allocated.
                    let rc = unsafe {
                        ffi::git_libgit2_opts(
                            ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                            allocator.as_ref().as_ptr(),
                        )
                    };
                    assert_eq!(rc, 0);
                    let mut value = Buf::new();
                    assert_eq!(value.as_mut().add(b"x"), Err(Error::OutOfMemory));
                    assert_eq!(value.as_mut().add_str(c"x"), Err(Error::OutOfMemory));
                    assert_eq!(parse_names(b"a\n").unwrap_err(), Error::OutOfMemory);
                    assert!(ReftableStr::from_c_str(c"x").is_none());
                    assert!(ReftableVec::<u8>::from_slice(b"x").is_none());
                    assert!(ReftableArrayFree::new_zeroed::<u8>(1).is_none());
                    assert!(ReftableBox::<Buf>::new_zeroed().is_none());
                    let mut run = ReftableVec::<MaybeUninit<u8>>::empty();
                    assert_eq!(
                        ReftableArrayFree::resize(&mut run, 1),
                        Err(Error::OutOfMemory)
                    );
                    // Nothing to release: dropping the empty owners never reaches C.
                    drop(value);
                    assert!(!runtime_pinned());
                },
            );
        }
    }
}
