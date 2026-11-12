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
//! `Buf::zeroed()` builds a value, which owns nothing. The routines that allocate,
//! release and detach the buffer, and the owner that releases what a value comes to own,
//! are bound with this module's symbols. `libgit2.so` exports none of `basics.c`'s
//! functions, which it builds with hidden visibility, so they cannot be linked yet.
//!
//! `block_source_from_buf` keeps a buffer's address in the block source it fills, and
//! reads it for the source's life: a binding of it ties the source to the buffer's
//! borrow, so that the buffer neither moves nor changes while the source lives.

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use core::fmt;

pub use self::ffi_import::{Buf, BufMut, BufRef};
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

/// The C layout of [`Buf`], its handles and the field getters, which read through them.
#[expect(
    unsafe_code,
    reason = "reads reftable_buf's fields through its handles' pointers"
)]
mod ffi_import {
    use core::ffi::CStr;
    use core::slice;

    use ffibox::{CRefSeam, CZeroable, define_ctype};

    use crate::ffi;

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
                return core::ptr::null();
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

    #[cfg(test)]
    mod unit_priv {
        //! The getters over allocated buffers, which only `basics.c`'s routines make: the
        //! fixture builds the C struct over Rust storage, as one of them leaves it, and
        //! reads it through a handle.

        use core::ffi::{CStr, c_char};
        use core::ptr;

        use ffibox::{CRefSeam, CZeroable};

        use super::{Buf, BufRef};
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
    }
}
