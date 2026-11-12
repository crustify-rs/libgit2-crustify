//! `src/util/strlist.c`, a foundational module: lists of NUL-terminated strings in libgit2's
//! allocator (`src/util/strlist.h`), copied, searched and released by C.
//!
//! # The lists
//!
//! libgit2 keeps a list of strings in two shapes, each copied from a counted run of borrowed
//! strings and released, strings first and then the array, by its own routine:
//!
//! | policy | owner | copies as | releases |
//! |---|---|---|---|
//! | [`StrListFree`] | [`StrList`], `len` strings and their count | `git_strlist_copy` | `git_strlist_free` |
//! | [`NullStrListFree`] | [`NullStrList`], strings ended by a NULL pointer | `git_strlist_copy_with_null` | `git_strlist_free_with_null` |
//!
//! A counted list is what libgit2's in-memory config backend keeps of the values it is built
//! from; a NULL-terminated one is the `argv` and `envp` its process runner hands to `execve`.
//! The array is a `git__calloc` allocation and every string a `git__strdup` copy, so each
//! element is an [`AllocStr`](crate::libgit2::src::util::alloc::AllocStr), the owner of one string
//! in libgit2's allocator, and the lists give their elements out as `&[AllocStr]` and `&mut
//! [AllocStr]`: an element replaced through the mutable view drops the string it held through
//! `git__free`, and the list's releaser frees whatever its elements hold when it drops. No element
//! is ever NULL, and a [`NullStrList`]'s terminator is never in view.
//!
//! The releases are C's own: each owner holds its array under its policy, which hands it to
//! the release routine on drop, and is built only by a copy or by adopting an array C made.
//! The policies release only the lists this module builds: no value of either can be made
//! outside it, so no other owner reaches their releasers.
//!
//! The copies are composed from the requests `strlist.c`'s make, in its order: the zeroed
//! array through `git__calloc`, through
//! [`ArrayFree::new_zeroed`](crate::libgit2::src::util::alloc::ArrayFree::new_zeroed),
//! then each string through `git__strdup`, through [`StrFree::dup`]. A copy first pins
//! libgit2's runtime, as every allocation of [`alloc`](crate::libgit2::src::util::alloc) does,
//! and fails, as C does, only when libgit2's allocator refuses a request, which leaves
//! `GIT_ERROR_NOMEMORY` as the thread's last error. A refused copy releases what it had
//! copied, where `git_strlist_copy` and `git_strlist_copy_with_null` leak the array and the
//! strings before the refused one (`GIT_ERROR_CHECK_ALLOC` returns without releasing them):
//! that leak is why the copies are not those routines' calls. An empty copy still asks the
//! allocator for its array, zero bytes for a counted list and one NULL pointer for a
//! NULL-terminated one; [`StrList::new`] is the empty counted list without an array, which
//! never reaches C.
//!
//! # Searching
//!
//! [`contains_prefix`] and [`contains_key`] search any run of borrowed strings, a list's
//! elements among them, through `git_strlist_contains_prefix`; they read their arguments only,
//! and allocate nothing in libgit2. [`contains_key`] compares a key's name with its delimiter,
//! where `git_strlist_contains_key` compares the name alone and so also finds it in a longer
//! name.

use core::ffi::{CStr, c_char};
use core::hash::{Hash, Hasher};

pub use ffi_import::{NullStrList, NullStrListFree, StrList, StrListFree, contains_prefix};

use crate::libgit2::src::util::alloc::{AllocStr, ArrayFree, StrFree};

/// Whether any of `strings` sets the variable of the `NAME=value` entry `key`: whether one
/// begins with `key`'s name and its first `delimiter`; `false` when `key` holds no
/// `delimiter`, without reading `strings`. This is what libgit2's process runner asks of the
/// caller's environment for each inherited entry, to let the caller's own entry replace it.
///
/// `git_strlist_contains_key` compares the name alone, without the delimiter, so it also
/// finds the name at the start of a longer one: `"FOO=1"` among `["FOOBAR=2"]`, where
/// libgit2's process runner then drops an inherited `FOO` because the caller sets `FOOBAR`.
/// Here the delimiter is compared too. A `delimiter` of 0, the terminator, is never found, so
/// the answer is then always `false`, as in C.
///
/// ```
/// use libgit2::libgit2::src::util::strlist::contains_key;
///
/// let env = [c"HOME=/root", c"FOOBAR=2"];
/// assert!(contains_key(&env, c"HOME=/home/me", b'='));
/// assert!(!contains_key(&env, c"USER=me", b'='));
/// assert!(!contains_key(&env, c"HOME", b'='));
/// // `FOO` is no variable of `FOOBAR=2`.
/// assert!(!contains_key(&env, c"FOO=1", b'='));
/// ```
///
/// Replaces `fn::git_strlist_contains_key`
#[doc(alias = "git_strlist_contains_key")]
#[must_use]
pub fn contains_key<S: AsRef<CStr>>(strings: &[S], key: &CStr, delimiter: u8) -> bool {
    let key = key.to_bytes();
    key.iter()
        .position(|&b| b == delimiter)
        .is_some_and(|end| contains_prefix(strings, &key[..=end]))
}

// --- The lists as slices ---------------------------------------------------------------
//
// Each list stands in for a `Vec` of its strings, as ffibox's `CVec` of owners does: it
// compares, hashes and iterates as its elements do.

macro_rules! slice_traits {
    ($list:ty) => {
        impl AsRef<[AllocStr]> for $list {
            fn as_ref(&self) -> &[AllocStr] {
                self
            }
        }

        impl AsMut<[AllocStr]> for $list {
            fn as_mut(&mut self) -> &mut [AllocStr] {
                self
            }
        }

        impl<'a> IntoIterator for &'a $list {
            type IntoIter = core::slice::Iter<'a, AllocStr>;
            type Item = &'a AllocStr;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        impl<'a> IntoIterator for &'a mut $list {
            type IntoIter = core::slice::IterMut<'a, AllocStr>;
            type Item = &'a mut AllocStr;

            fn into_iter(self) -> Self::IntoIter {
                self.iter_mut()
            }
        }

        /// Compared string by string, as a `Vec` of them is.
        impl PartialEq for $list {
            fn eq(&self, other: &Self) -> bool {
                self[..] == other[..]
            }
        }

        impl Eq for $list {}

        impl PartialEq<[AllocStr]> for $list {
            fn eq(&self, other: &[AllocStr]) -> bool {
                self[..] == *other
            }
        }

        impl Hash for $list {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self[..].hash(state);
            }
        }
    };
}

slice_traits!(StrList);
slice_traits!(NullStrList);

/// Copies of `strs`, in order, in the first slots of a zeroed array of `strs.len() + spare`
/// pointers: the array through `git__calloc`, then each string through `git__strdup`, as
/// `git_strlist_copy` and `git_strlist_copy_with_null` request them. `None` when a request is
/// refused, which leaves `GIT_ERROR_NOMEMORY` as the thread's last error, with every copy
/// and the array released, or when libgit2 cannot be initialised.
///
/// The array and its length are given up to the caller; the array is NULL only when it holds
/// no slot and the allocator's zero-byte block was not aligned for one.
fn copy_array<S: AsRef<CStr>>(strs: &[S], spare: usize) -> Option<(*mut *mut c_char, usize)> {
    let strs: Vec<&CStr> = strs.iter().map(AsRef::as_ref).collect();
    let mut array = ArrayFree::new_zeroed::<*mut c_char>(strs.len() + spare)?;
    let copies = strs
        .iter()
        .map(|s| StrFree::dup(s))
        .collect::<Option<Vec<AllocStr>>>()?;
    for (slot, copy) in array.iter_mut().zip(copies) {
        *slot = copy.into_raw();
    }
    Some(array.into_raw_parts())
}

/// What the module reaches in C: the search and release routines of `src/util/strlist.c`,
/// and the owners of the lists they release.
mod ffi_import {
    use core::ffi::{CStr, c_char};
    use core::mem::{ManuallyDrop, align_of, size_of};
    use core::ops::{Deref, DerefMut};
    use core::{fmt, ptr};

    use ffibox::{CLenDrop, CVec};

    use crate::ffi;
    use crate::libgit2::src::util::alloc::AllocStr;

    /// One element of a list's array: a string of libgit2's allocator that the list's
    /// releaser frees, so the array's [`CVec`] never drops it.
    type Slot = ManuallyDrop<AllocStr>;

    // A slot is the non-null `char *` C stores, so an array of them is C's `char **`, and
    // libgit2's allocator, whose storage is aligned to 8 bytes, aligns it.
    const _: () = assert!(
        size_of::<Slot>() == size_of::<*mut c_char>()
            && align_of::<Slot>() == align_of::<*mut c_char>()
            && align_of::<Slot>() <= 8
            && size_of::<AllocStr>() == size_of::<Slot>()
    );

    /// The array of thin `char *` pointers C reads a run of borrowed strings through, in
    /// Rust's allocator; each pointer borrows its string from `strs` for as long as `strs`
    /// is borrowed.
    fn thin_ptrs<S: AsRef<CStr>>(strs: &[S]) -> Vec<*const c_char> {
        strs.iter().map(|s| s.as_ref().as_ptr()).collect()
    }

    /// `ptrs` as the `const char **` C takes: NULL for an empty run, which C never reads.
    fn array_ptr(ptrs: &[*const c_char]) -> *mut *const c_char {
        if ptrs.is_empty() {
            ptr::null_mut()
        } else {
            ptrs.as_ptr().cast_mut()
        }
    }

    /// The elements of a list's array as owners of their strings.
    ///
    /// # Safety
    ///
    /// `array` and `len` are a live list's: `array` holds `len` live slots (it may be
    /// NULL, or unaligned, only when `len` is 0), which nothing else reads or writes while
    /// the slice lives.
    unsafe fn slots<'a>(array: *mut Slot, len: usize) -> &'a [AllocStr] {
        if len == 0 {
            return &[];
        }
        // SAFETY: per this function's contract, `array` holds `len` initialised slots,
        // aligned for them, and a slot is `#[repr(transparent)]` over an `AllocStr`.
        unsafe { core::slice::from_raw_parts(array.cast::<AllocStr>(), len) }
    }

    /// As [`slots`], mutably.
    ///
    /// # Safety
    ///
    /// As [`slots`], with the slice the only access to the slots while it lives.
    unsafe fn slots_mut<'a>(array: *mut Slot, len: usize) -> &'a mut [AllocStr] {
        if len == 0 {
            return &mut [];
        }
        // SAFETY: as in `slots`, exclusively.
        unsafe { core::slice::from_raw_parts_mut(array.cast::<AllocStr>(), len) }
    }

    /// Whether `len` slots from `array` are all non-NULL.
    ///
    /// # Safety
    ///
    /// `array` holds `len` initialised `char *` elements.
    unsafe fn all_non_null(array: *mut *mut c_char, len: usize) -> bool {
        (0..len).all(|i| {
            // SAFETY: `i < len`, and per this function's contract `array` holds `len`
            // elements, so the offset stays within them.
            let slot = unsafe { array.add(i) };
            // SAFETY: as above, the element is initialised.
            !unsafe { slot.read() }.is_null()
        })
    }

    // --- Counted lists -----------------------------------------------------------------

    /// The policy of a counted list of strings in libgit2's allocator: copied as
    /// `git_strlist_copy` copies one, released, every string and then the array, by
    /// `git_strlist_free`.
    ///
    /// Its array and strings are those of [`StrList`], the only owner under it: no value
    /// of the policy can be made outside this module, so its releaser only ever receives a
    /// list's array.
    ///
    /// A borrow of an element ends with its list:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::util::strlist::StrListFree;
    ///
    /// let list = StrListFree::copy(&[c"tied"]).unwrap();
    /// let first = &list[0];
    /// drop(list);
    /// let _ = first.as_bytes();
    /// ```
    ///
    /// No code outside this module makes a value of the policy:
    ///
    /// ```compile_fail,E0423
    /// let _ = libgit2::libgit2::src::util::strlist::StrListFree(());
    /// ```
    #[derive(Clone, Copy, Debug)]
    pub struct StrListFree(());

    // SAFETY: `git_strlist_free(array, n)` hands each of the first `n` elements, then the
    // array, to `git__free` exactly once. Every buffer under this policy is a `StrList`'s
    // (no other code can make a value of the policy): `git_strlist_copy`'s array of
    // `len` copies, or one adopted under `from_raw_parts`' contract, whose `byte_len` is
    // `len * size_of::<Slot>()`, whose `len` elements are live strings of libgit2's
    // allocator that `CVec` never drops (`ManuallyDrop`), and whose elements the mutable
    // view only ever replaces with other such strings.
    unsafe impl CLenDrop for StrListFree {
        /// Wraps `fn::git_strlist_free`
        #[doc(alias = "git_strlist_free")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, byte_len: usize) {
            let len = byte_len / size_of::<Slot>();
            // SAFETY: per the impl's safety comment, `ptr` is a list's array of `len` live
            // strings of libgit2's allocator, all of which the caller gives up.
            unsafe { ffi::git_strlist_free(ptr.cast(), len) }
        }
    }

    impl StrListFree {
        /// A counted list of copies of `strs`, in order, in libgit2's allocator; `None`
        /// when an allocation is refused, which leaves `GIT_ERROR_NOMEMORY` as the thread's
        /// last error, or when libgit2 cannot be initialised.
        ///
        /// An empty `strs` still allocates the list's array, zero bytes, as C does. A
        /// refused copy releases the copies made before it, where C leaks them (see the
        /// module docs).
        ///
        /// ```
        /// use libgit2::libgit2::src::util::strlist::StrListFree;
        ///
        /// let list = StrListFree::copy(&[c"core.bare=false", c"user.name=me"]).unwrap();
        /// assert_eq!(list.len(), 2);
        /// assert_eq!(list[1].as_c_str(), c"user.name=me");
        /// ```
        ///
        /// Replaces `fn::git_strlist_copy`
        #[doc(alias = "git_strlist_copy")]
        #[must_use]
        pub fn copy<S: AsRef<CStr>>(strs: &[S]) -> Option<StrList> {
            let (array, len) = super::copy_array(strs, 0)?;
            // SAFETY: `array` is NULL with `len == 0`, or a fresh `git__calloc` array of
            // `len` `git__strdup` copies, none NULL, which the list owns and
            // `git_strlist_free` releases.
            Some(unsafe { StrList::adopt(array, len) })
        }
    }

    /// A counted list of strings in libgit2's allocator, as `git_strlist_copy` makes one
    /// and `git_strlist_free` releases it.
    ///
    /// Dereferences to its elements, `&[AllocStr]`, and mutably to `&mut [AllocStr]`: an
    /// element replaced through it is released at once, and the list releases what its
    /// elements hold when it drops. [`StrListFree::copy`] builds one, and `Clone` copies one
    /// as it does; an empty one without an array is [`new`](Self::new).
    ///
    /// ```
    /// use libgit2::libgit2::src::util::alloc::StrFree;
    /// use libgit2::libgit2::src::util::strlist::StrListFree;
    ///
    /// let mut list = StrListFree::copy(&[c"a", c"b"]).unwrap();
    /// list[0] = StrFree::dup(c"z").unwrap();
    /// list.sort();
    /// assert_eq!(list[0].as_c_str(), c"b");
    /// ```
    pub struct StrList(CVec<Slot, StrListFree>);

    impl StrList {
        /// The empty list without an array: nothing is allocated, and nothing is released.
        #[must_use]
        pub const fn new() -> Self {
            Self(CVec::empty_with(StrListFree(())))
        }

        /// The owner of a copied or adopted array.
        ///
        /// # Safety
        ///
        /// `array` is NULL with `len == 0`, or a live array of libgit2's allocator holding
        /// `len` non-NULL strings of the same allocator, all of which the caller gives up.
        unsafe fn adopt(array: *mut *mut c_char, len: usize) -> Self {
            // SAFETY: per this function's contract; a slot is a non-NULL `char *` whose
            // string `git_strlist_free` releases with the array, and `byte_len` is the size
            // of the `len` slots `git_strlist_free` reads. NULL comes only with `len == 0`.
            let run =
                unsafe { CVec::from_raw_parts_or_empty_with(array.cast(), len, StrListFree(())) };
            Self(run.expect("a NULL list's array holds no string"))
        }

        /// Takes ownership of `len` strings at `strings`, as `git_strlist_free` would
        /// release them.
        ///
        /// NULL with `len == 0` is the empty list without an array. Returns `None`, leaving
        /// ownership with the caller, when `strings` is NULL with a nonzero `len` or any of
        /// its first `len` elements is NULL, which `git_strlist_free` would skip but a list
        /// never holds.
        ///
        /// # Safety
        ///
        /// A non-NULL `strings` must be an array allocated by libgit2's current allocator,
        /// aligned for a pointer, whose first `len` elements are initialised, each NULL or
        /// a NUL-terminated string allocated by the same allocator, no two of them the same
        /// string, since the list releases each element once. The caller transfers unique
        /// ownership of the array and of those strings, and does not use them again unless
        /// `None` is returned.
        pub unsafe fn from_raw_parts(strings: *mut *mut c_char, len: usize) -> Option<Self> {
            if strings.is_null() {
                return (len == 0).then(Self::new);
            }
            // SAFETY: per this function's contract, `strings` holds `len` initialised
            // elements.
            if !unsafe { all_non_null(strings, len) } {
                return None;
            }
            // SAFETY: a non-NULL array of libgit2's allocator holding `len` non-NULL strings
            // of the same allocator, which the caller gives up.
            Some(unsafe { Self::adopt(strings, len) })
        }

        /// Gives up the list without releasing it: the array and the number of strings in
        /// it, for `git_strlist_free(strings, len)`; NULL with 0 for a list without an
        /// array.
        #[must_use = "the returned pointer owns the strings and must be freed"]
        pub fn into_raw_parts(self) -> (*mut *mut c_char, usize) {
            let (array, len) = self.0.into_raw_parts();
            (array.cast(), len)
        }

        /// The array, for C to read as a `const char **` of `self.len()` strings; NULL for
        /// a list without an array. Ownership is retained.
        #[must_use]
        pub fn as_ptr(&self) -> *const *const c_char {
            self.0.as_ptr().cast_const().cast()
        }

        /// A copy of every string, as [`StrListFree::copy`] makes one; `None` when an
        /// allocation is refused, as for [`StrListFree::copy`].
        ///
        /// Replaces `fn::git_strlist_copy`
        #[doc(alias = "git_strlist_copy")]
        #[must_use]
        pub fn try_clone(&self) -> Option<Self> {
            StrListFree::copy(&self[..])
        }
    }

    impl Deref for StrList {
        type Target = [AllocStr];

        fn deref(&self) -> &[AllocStr] {
            // SAFETY: the list owns its array's `len` live slots; the view is bound by
            // `&self`, so nothing writes them while it lives.
            unsafe { slots(self.0.as_ptr(), self.0.len()) }
        }
    }

    impl DerefMut for StrList {
        fn deref_mut(&mut self) -> &mut [AllocStr] {
            // SAFETY: as in `deref`, with `&mut self` making the view exclusive; a slot
            // written through it holds another `AllocStr`, a non-NULL string of libgit2's
            // allocator, which `git_strlist_free` releases.
            unsafe { slots_mut(self.0.as_ptr(), self.0.len()) }
        }
    }

    impl Clone for StrList {
        /// Copies every string; aborts when an allocation is refused, as an
        /// [`AllocStr`]'s `Clone` does. [`try_clone`](Self::try_clone) reports the failure
        /// instead.
        fn clone(&self) -> Self {
            self.try_clone().unwrap_or_else(|| std::process::abort())
        }
    }

    impl Default for StrList {
        fn default() -> Self {
            Self::new()
        }
    }

    impl fmt::Debug for StrList {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_list()
                .entries(self.iter().map(AllocStr::as_c_str))
                .finish()
        }
    }

    // --- NULL-terminated lists ---------------------------------------------------------

    /// The policy of a NULL-terminated list of strings in libgit2's allocator: copied as
    /// `git_strlist_copy_with_null` copies one, released, every string and then the array,
    /// by `git_strlist_free_with_null`.
    ///
    /// Its arrays are those of [`NullStrList`], the only owner under it: no value of the
    /// policy can be made outside this module.
    ///
    /// A borrow of an element ends with its list:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::util::strlist::NullStrListFree;
    ///
    /// let list = NullStrListFree::copy(&[c"PATH=/bin"]).unwrap();
    /// let first = &list[0];
    /// drop(list);
    /// let _ = first.as_bytes();
    /// ```
    ///
    /// No code outside this module makes a value of the policy, nor a list without an
    /// array:
    ///
    /// ```compile_fail,E0423
    /// let _ = libgit2::libgit2::src::util::strlist::NullStrListFree(());
    /// ```
    ///
    /// ```compile_fail,E0599
    /// let _ = libgit2::libgit2::src::util::strlist::NullStrList::default();
    /// ```
    #[derive(Clone, Copy, Debug)]
    pub struct NullStrListFree(());

    // SAFETY: `git_strlist_free_with_null(array)` hands each element before the first NULL
    // one, then the array, to `git__free` exactly once, reading no further than that NULL
    // element. Every buffer under this policy is a `NullStrList`'s (no other code can make
    // a value of the policy): an array of libgit2's allocator whose first `len` elements
    // are live strings of the same allocator, which `CVec` never drops (`ManuallyDrop`)
    // and the mutable view only replaces with other such strings, followed by a NULL
    // element outside every view. `byte_len` covers the `len` strings, less than the
    // allocation; the routine takes no size.
    unsafe impl CLenDrop for NullStrListFree {
        /// Wraps `fn::git_strlist_free_with_null`
        #[doc(alias = "git_strlist_free_with_null")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, _byte_len: usize) {
            // SAFETY: per the impl's safety comment, `ptr` is a NULL-terminated list's array
            // of live strings of libgit2's allocator, all of which the caller gives up.
            unsafe { ffi::git_strlist_free_with_null(ptr.cast()) }
        }
    }

    impl NullStrListFree {
        /// A NULL-terminated list of copies of `strs`, in order, in libgit2's allocator;
        /// `None` when an allocation is refused, which leaves `GIT_ERROR_NOMEMORY` as the
        /// thread's last error, or when libgit2 cannot be initialised.
        ///
        /// The array holds one slot more than `strs`, the NULL terminator, so an empty
        /// `strs` allocates an array holding the terminator alone. A refused copy releases
        /// the copies made before it, where C leaks them (see the module docs).
        ///
        /// ```
        /// use libgit2::libgit2::src::util::strlist::NullStrListFree;
        ///
        /// let argv = NullStrListFree::copy(&[c"/bin/sh", c"-c", c"true"]).unwrap();
        /// assert_eq!(argv.len(), 3);
        /// assert_eq!(argv[0].as_c_str(), c"/bin/sh");
        /// ```
        ///
        /// Replaces `fn::git_strlist_copy_with_null`
        #[doc(alias = "git_strlist_copy_with_null")]
        #[must_use]
        pub fn copy<S: AsRef<CStr>>(strs: &[S]) -> Option<NullStrList> {
            let (array, slots) = super::copy_array(strs, 1)?;
            // SAFETY: `array` is a fresh `git__calloc` array of `slots` slots, at least one,
            // so never NULL: the first `slots - 1` of them `git__strdup` copies and the last
            // NULL, which the list owns and `git_strlist_free_with_null` releases.
            Some(unsafe { NullStrList::adopt(array, slots - 1) })
        }
    }

    /// A NULL-terminated list of strings in libgit2's allocator, as
    /// `git_strlist_copy_with_null` makes one and `git_strlist_free_with_null` releases it:
    /// the `argv` or `envp` C hands to `execve`.
    ///
    /// Dereferences to its elements before the terminator, `&[AllocStr]`, and mutably to
    /// `&mut [AllocStr]`: an element replaced through it is released at once, and the list
    /// releases what its elements hold when it drops. The terminator is never in view, so
    /// it stays where it is. Every list has an array: [`NullStrListFree::copy`] builds one,
    /// and `Clone` copies one as it does.
    pub struct NullStrList(CVec<Slot, NullStrListFree>);

    impl NullStrList {
        /// The owner of a copied or adopted NULL-terminated array.
        ///
        /// # Safety
        ///
        /// `array` is a live, non-NULL array of libgit2's allocator whose first `len`
        /// elements are non-NULL strings of the same allocator and whose element `len` is
        /// NULL, all of which the caller gives up.
        unsafe fn adopt(array: *mut *mut c_char, len: usize) -> Self {
            // SAFETY: per this function's contract; the run covers the `len` strings, and
            // `git_strlist_free_with_null`, which takes no size, releases them and the array
            // through the terminator after them.
            let run = unsafe { CVec::from_raw_parts_with(array.cast(), len, NullStrListFree(())) };
            Self(run.expect("a copied or adopted list's array is never NULL"))
        }

        /// Takes ownership of the NULL-terminated array `strings`, as
        /// `git_strlist_free_with_null` would release it.
        ///
        /// Returns `None` for a NULL `strings`, which `git_strlist_free_with_null` would
        /// ignore but a list never is.
        ///
        /// # Safety
        ///
        /// A non-NULL `strings` must be an array allocated by libgit2's current allocator,
        /// aligned for a pointer, holding NUL-terminated strings allocated by the same
        /// allocator followed by a NULL element, no two of the strings the same, since the
        /// list releases each one once. The caller transfers unique ownership of the array
        /// and of those strings, and does not use them again unless `None` is returned.
        pub unsafe fn from_raw(strings: *mut *mut c_char) -> Option<Self> {
            if strings.is_null() {
                return None;
            }
            let mut len = 0;
            loop {
                // SAFETY: per this function's contract, the elements up to and including the
                // NULL one are within the array, and the loop stops at that one.
                let slot = unsafe { strings.add(len) };
                // SAFETY: as above, the element is initialised.
                if unsafe { slot.read() }.is_null() {
                    break;
                }
                len += 1;
            }
            // SAFETY: a non-NULL array of libgit2's allocator whose first `len` elements are
            // non-NULL strings of the same allocator, followed by a NULL one, which the
            // caller gives up.
            Some(unsafe { Self::adopt(strings, len) })
        }

        /// Gives up the list without releasing it: the NULL-terminated array, for
        /// `git_strlist_free_with_null`.
        #[must_use = "the returned pointer owns the strings and must be freed"]
        pub fn into_raw(self) -> *mut *mut c_char {
            self.0.into_raw_parts().0.cast()
        }

        /// The NULL-terminated array, for C to read as a `char *const *`, such as
        /// `execve`'s `argv`. Never NULL; ownership is retained.
        #[must_use]
        pub fn as_ptr(&self) -> *const *const c_char {
            self.0.as_ptr().cast_const().cast()
        }

        /// A copy of every string, as [`NullStrListFree::copy`] makes one; `None` when an
        /// allocation is refused, as for [`NullStrListFree::copy`].
        ///
        /// Replaces `fn::git_strlist_copy_with_null`
        #[doc(alias = "git_strlist_copy_with_null")]
        #[must_use]
        pub fn try_clone(&self) -> Option<Self> {
            NullStrListFree::copy(&self[..])
        }
    }

    impl Deref for NullStrList {
        type Target = [AllocStr];

        fn deref(&self) -> &[AllocStr] {
            // SAFETY: the list owns its array's `len` live slots before the terminator; the
            // view is bound by `&self`, so nothing writes them while it lives.
            unsafe { slots(self.0.as_ptr(), self.0.len()) }
        }
    }

    impl DerefMut for NullStrList {
        fn deref_mut(&mut self) -> &mut [AllocStr] {
            // SAFETY: as in `deref`, with `&mut self` making the view exclusive; a slot
            // written through it holds another non-NULL string of libgit2's allocator, and
            // the terminator is outside the view.
            unsafe { slots_mut(self.0.as_ptr(), self.0.len()) }
        }
    }

    impl Clone for NullStrList {
        /// Copies every string; aborts when an allocation is refused, as an
        /// [`AllocStr`]'s `Clone` does. [`try_clone`](Self::try_clone) reports the failure
        /// instead.
        fn clone(&self) -> Self {
            self.try_clone().unwrap_or_else(|| std::process::abort())
        }
    }

    impl fmt::Debug for NullStrList {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_list()
                .entries(self.iter().map(AllocStr::as_c_str))
                .finish()
        }
    }

    // --- Searching ---------------------------------------------------------------------

    /// Whether any of `strings` begins with `prefix`.
    ///
    /// C compares each string with `strncmp(string, prefix, prefix.len())`, which stops at
    /// a NUL: a NUL in `prefix` ends it, and a string then matches only when it equals the
    /// bytes before that NUL. An empty `prefix` matches every string, so it is found in any
    /// non-empty run, and nothing is found in an empty one.
    ///
    /// ```
    /// use libgit2::libgit2::src::util::strlist::contains_prefix;
    ///
    /// let env = [c"HOME=/root", c"PATH=/bin"];
    /// assert!(contains_prefix(&env, b"PATH="));
    /// assert!(!contains_prefix(&env, b"USER="));
    /// // A NUL ends the prefix, and then asks for the whole string.
    /// assert!(!contains_prefix(&env, b"PATH\0"));
    /// assert!(contains_prefix(&env, b"PATH=/bin\0trailing"));
    /// ```
    ///
    /// Wraps `fn::git_strlist_contains_prefix`
    #[doc(alias = "git_strlist_contains_prefix")]
    #[must_use]
    pub fn contains_prefix<S: AsRef<CStr>>(strings: &[S], prefix: &[u8]) -> bool {
        let ptrs = thin_ptrs(strings);
        // `strncmp` reads at most `prefix.len()` bytes through the pointer: an empty
        // prefix's is a live empty string, never dangling.
        let prefix_ptr = if prefix.is_empty() {
            c"".as_ptr()
        } else {
            prefix.as_ptr().cast()
        };
        // SAFETY: `ptrs` holds `ptrs.len()` NUL-terminated strings borrowed from `strings`
        // (NULL, unread, when empty), each read up to its terminator; C reads at most
        // `prefix.len()` bytes of `prefix_ptr`, all of them in `prefix`. Nothing is kept
        // past the call, and nothing reaches libgit2's state.
        unsafe {
            ffi::git_strlist_contains_prefix(array_ptr(&ptrs), ptrs.len(), prefix_ptr, prefix.len())
        }
    }

    #[cfg(test)]
    mod unit_tests {
        use core::hash::{Hash, Hasher};
        use core::ptr::NonNull;
        use std::alloc::{Layout, alloc, dealloc, realloc};
        use std::cell::RefCell;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering::Relaxed;

        use super::*;
        use crate::libgit2::src::libgit2::settings::set_allocator;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator, StrFree};
        use crate::libgit2::src::util::errors::{ErrorClass, clear, last, runtime_pinned};
        use crate::libgit2::src::util::strlist::contains_key;

        /// One request that reached [`Counted`]: an allocation of so many bytes, through
        /// `malloc` or `realloc(NULL, ..)`, or a release.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Call {
            Alloc(usize),
            Free,
        }

        thread_local! {
            static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
        }

        /// Records `call` on the calling thread, unless the thread is exiting and its record
        /// is gone: libgit2 releases a thread's error state from a TLS destructor.
        fn record(call: Call) {
            let _gone = CALLS.try_with(|c| c.borrow_mut().push(call));
        }

        fn take_calls() -> Vec<Call> {
            CALLS.with(RefCell::take)
        }

        /// The one request size [`Counted`] refuses; [`NONE`] refuses none.
        static REFUSE: AtomicUsize = AtomicUsize::new(NONE);

        /// No request size: a zero-byte request is one libgit2 makes.
        const NONE: usize = usize::MAX;

        /// A Rust-heap allocator that records each request: each block carries its size in
        /// a 16-byte header.
        struct Counted;

        const HEADER: usize = 16;

        fn layout(len: usize) -> Layout {
            Layout::from_size_align(HEADER + len, HEADER).unwrap()
        }

        // SAFETY: blocks come from the global allocator, 16-aligned past their header, fresh
        // per call; `realloc` keeps the prefix and `free` releases with the recorded layout.
        // Thread-safe as the global allocator is.
        unsafe impl Alloc for Counted {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                record(Call::Alloc(len));
                if len == REFUSE.load(Relaxed) {
                    return None;
                }
                // SAFETY: the layout is never zero-sized.
                let base = NonNull::new(unsafe { alloc(layout(len)) })?;
                // SAFETY: the header is the block's first 16 bytes, aligned for a `usize`.
                unsafe { base.cast::<usize>().write(len) };
                // SAFETY: the block spans `HEADER + len` bytes.
                Some(unsafe { base.add(HEADER) })
            }

            unsafe fn realloc(
                ptr: Option<NonNull<u8>>,
                len: usize,
                file: Option<&CStr>,
                line: i32,
            ) -> Option<NonNull<u8>> {
                let Some(ptr) = ptr else {
                    return Self::malloc(len, file, line);
                };
                // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
                let base = unsafe { ptr.sub(HEADER) };
                // SAFETY: as above; the header holds the block's length.
                let old = unsafe { base.cast::<usize>().read() };
                // SAFETY: `base` was allocated with `layout(old)`; the new size is non-zero.
                let base =
                    NonNull::new(unsafe { realloc(base.as_ptr(), layout(old), HEADER + len) })?;
                // SAFETY: as in `malloc`.
                unsafe { base.cast::<usize>().write(len) };
                // SAFETY: as in `malloc`.
                Some(unsafe { base.add(HEADER) })
            }

            unsafe fn free(ptr: NonNull<u8>) {
                record(Call::Free);
                // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
                let base = unsafe { ptr.sub(HEADER) };
                // SAFETY: as above; `base` was allocated with `layout(len)`.
                let len = unsafe { base.cast::<usize>().read() };
                // SAFETY: as above.
                unsafe { dealloc(base.as_ptr(), layout(len)) }
            }
        }

        const CHILD: &str = "CRUSTIFY_STRLIST_UNIT_CHILD";

        /// Runs `body` alone in a child process of this binary, with [`Counted`] installed as
        /// libgit2's allocator before anything initialises libgit2. `leaks` lets the child
        /// leak, as C's copies do on a refusal, without failing `LeakSanitizer`'s exit check.
        fn with_counted(path: &str, leaks: bool, body: fn()) {
            let name = path.split_once("::").map_or(path, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                static COUNTED: Allocator = Allocator::new::<Counted>();
                // SAFETY: `Counted` meets `Alloc`'s contract; it is installed before libgit2
                // holds any allocation, in a process of its own, before any other thread runs.
                unsafe { set_allocator(Some(COUNTED.as_ref())) };
                assert!(runtime_pinned());
                // The thread's error state is allocated once, before the steps.
                clear();
                take_calls();
                body();
                return;
            }
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name);
            if leaks {
                child.env("ASAN_OPTIONS", "detect_leaks=0:abort_on_error=1");
            }
            let out = child.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        fn assert_out_of_memory() {
            let e = last();
            assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{e:?}");
            clear();
        }

        const PTR: usize = size_of::<*mut c_char>();

        /// The empty list without an array is built, read, given up and adopted without C,
        /// so Miri runs this.
        #[test]
        fn empty_list_without_array_never_reaches_c() {
            let mut list = StrList::new();
            assert!(list.is_empty() && list.as_ptr().is_null());
            assert!(list.iter_mut().next().is_none());
            assert_eq!(format!("{list:?}"), "[]");
            assert!(StrList::default().is_empty());
            let (array, len) = list.into_raw_parts();
            assert!(array.is_null() && len == 0);
            // SAFETY: NULL with 0 is the empty list; a NULL array with a count, or a NULL
            // NULL-terminated array, is refused, and adopts nothing.
            let back = unsafe { StrList::from_raw_parts(ptr::null_mut(), 0) }.unwrap();
            assert!(back.is_empty() && back.as_ptr().is_null());
            // SAFETY: as above.
            assert!(unsafe { StrList::from_raw_parts(ptr::null_mut(), 2) }.is_none());
            // SAFETY: as above.
            assert!(unsafe { NullStrList::from_raw(ptr::null_mut()) }.is_none());
        }

        /// Each list asks libgit2's allocator for what `strlist.c` asks it for, the array
        /// through `git__calloc` and each string through `git__strdup`, and its releaser
        /// releases each once: strings and array alike.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn lists_request_and_release_what_strlist_c_does() {
            with_counted(
                concat!(
                    module_path!(),
                    "::lists_request_and_release_what_strlist_c_does"
                ),
                false,
                || {
                    let list = StrListFree::copy(&[c"ab", c"c"]).unwrap();
                    let copied = [Call::Alloc(2 * PTR), Call::Alloc(3), Call::Alloc(2)];
                    assert_eq!(take_calls(), copied);
                    let mut cloned = list.clone();
                    assert_eq!(take_calls(), copied);
                    drop(list);
                    assert_eq!(take_calls(), [Call::Free; 3]);
                    // A replaced element is released at once, its replacement with the list.
                    cloned[1] = StrFree::dup(c"zzz").unwrap();
                    assert_eq!(take_calls(), [Call::Alloc(4), Call::Free]);
                    assert_eq!(format!("{cloned:?}"), r#"["ab", "zzz"]"#);
                    drop(cloned);
                    assert_eq!(take_calls(), [Call::Free; 3]);

                    // One slot more for the terminator.
                    let argv = NullStrListFree::copy(&[c"x"]).unwrap();
                    assert_eq!(take_calls(), [Call::Alloc(2 * PTR), Call::Alloc(2)]);
                    let again = argv.try_clone().unwrap();
                    assert_eq!(take_calls(), [Call::Alloc(2 * PTR), Call::Alloc(2)]);
                    assert_eq!(format!("{again:?}"), r#"["x"]"#);
                    drop((argv, again));
                    assert_eq!(take_calls(), [Call::Free; 4]);

                    // An empty copy still allocates its array; the list without one does
                    // not.
                    let empty = StrListFree::copy::<&CStr>(&[]).unwrap();
                    let terminator = NullStrListFree::copy::<&CStr>(&[]).unwrap();
                    assert!(empty.is_empty() && !empty.as_ptr().is_null());
                    assert!(terminator.is_empty());
                    assert_eq!(take_calls(), [Call::Alloc(0), Call::Alloc(PTR)]);
                    drop((empty, terminator, StrList::new()));
                    assert_eq!(take_calls(), [Call::Free; 2]);
                },
            );
        }

        /// A refused request fails the copy with libgit2's out-of-memory error, and leaves
        /// nothing behind: a refused string releases the array and the copies before it,
        /// which `git_strlist_copy` and `git_strlist_copy_with_null` leak
        /// (`logic/refused-copies-leak`).
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn refused_copies_report_out_of_memory() {
            with_counted(
                concat!(module_path!(), "::refused_copies_report_out_of_memory"),
                false,
                || {
                    let strs = [c"one", c"four", c"x"];
                    REFUSE.store(3 * PTR, Relaxed);
                    assert!(StrListFree::copy(&strs).is_none());
                    assert_out_of_memory();
                    assert_eq!(take_calls(), [Call::Alloc(3 * PTR)]);
                    REFUSE.store(4 * PTR, Relaxed);
                    assert!(NullStrListFree::copy(&strs).is_none());
                    assert_out_of_memory();
                    assert_eq!(take_calls(), [Call::Alloc(4 * PTR)]);

                    // The copy of "one" and the array are released, in that order.
                    REFUSE.store(5, Relaxed);
                    assert!(StrListFree::copy(&strs).is_none());
                    assert_out_of_memory();
                    let refused = |array| {
                        [
                            Call::Alloc(array),
                            Call::Alloc(4),
                            Call::Alloc(5),
                            Call::Free,
                            Call::Free,
                        ]
                    };
                    assert_eq!(take_calls(), refused(3 * PTR));
                    assert!(NullStrListFree::copy(&strs).is_none());
                    assert_out_of_memory();
                    assert_eq!(take_calls(), refused(4 * PTR));

                    // A refused first string releases the array alone.
                    REFUSE.store(4, Relaxed);
                    assert!(NullStrListFree::copy(&strs).is_none());
                    assert_out_of_memory();
                    assert_eq!(
                        take_calls(),
                        [Call::Alloc(4 * PTR), Call::Alloc(4), Call::Free]
                    );

                    // A refused clone leaves its source live.
                    REFUSE.store(NONE, Relaxed);
                    let list = StrListFree::copy(&strs[..1]).unwrap();
                    let argv = NullStrListFree::copy(&strs[..1]).unwrap();
                    take_calls();
                    REFUSE.store(PTR, Relaxed);
                    assert!(list.try_clone().is_none());
                    assert_out_of_memory();
                    REFUSE.store(2 * PTR, Relaxed);
                    assert!(argv.try_clone().is_none());
                    assert_out_of_memory();
                    REFUSE.store(NONE, Relaxed);
                    assert_eq!(take_calls(), [Call::Alloc(PTR), Call::Alloc(2 * PTR)]);
                    assert_eq!((list[0].as_c_str(), argv[0].as_c_str()), (c"one", c"one"));
                    drop((list, argv));
                    assert_eq!(take_calls(), [Call::Free; 4]);
                },
            );
        }

        /// A NULL-terminated list's array holds its strings, then a NULL pointer, which is
        /// never in view; a counted list's holds its strings alone.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn arrays_read_as_c_lays_them_out() {
            let mut argv = NullStrListFree::copy(&[c"/bin/sh", c"-c"]).unwrap();
            argv[1] = StrFree::dup(c"-x").unwrap();
            let array = argv.as_ptr();
            let read = |i: usize| {
                // SAFETY: `array` is the list's live array of three slots, `i` below three.
                let slot = unsafe { array.add(i) };
                // SAFETY: as above, the slot is initialised.
                unsafe { slot.read() }
            };
            // SAFETY: the first two slots are live NUL-terminated strings of the list.
            let first = unsafe { CStr::from_ptr(read(0)) };
            // SAFETY: as above.
            let second = unsafe { CStr::from_ptr(read(1)) };
            assert_eq!((first, second), (c"/bin/sh", c"-x"));
            assert!(read(2).is_null());
            assert_eq!(argv.len(), 2);

            let list = StrListFree::copy(&[c"k=v"]).unwrap();
            // SAFETY: the list's array holds one slot.
            let string = unsafe { list.as_ptr().read() };
            // SAFETY: the slot holds a live string of the list.
            let only = unsafe { CStr::from_ptr(string) };
            assert_eq!(only, c"k=v");
            assert_eq!(list[0].as_ptr(), only.as_ptr());
        }

        /// The copies make C's requests in C's order, and differ from C's only on a
        /// refusal: C's `git_strlist_copy` and `git_strlist_copy_with_null` return without
        /// releasing the array and the copies before the refused one, which the wrappers
        /// release (`logic/refused-copies-leak`).
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn c_copies_leak_what_the_wrappers_release() {
            with_counted(
                concat!(module_path!(), "::c_copies_leak_what_the_wrappers_release"),
                true,
                || {
                    let strs = [c"one", c"four"];
                    let ptrs = strs.map(CStr::as_ptr);
                    for null_terminated in [false, true] {
                        let c_copy = if null_terminated {
                            ffi::git_strlist_copy_with_null
                        } else {
                            ffi::git_strlist_copy
                        };
                        let array = (2 + usize::from(null_terminated)) * PTR;
                        let requests = [Call::Alloc(array), Call::Alloc(4), Call::Alloc(5)];
                        let mut out: *mut *mut c_char = ptr::null_mut();
                        // A refused array is where both stop, with nothing to release.
                        REFUSE.store(array, Relaxed);
                        // SAFETY: as below; C reads nothing once its array is refused.
                        let rc = unsafe { c_copy(&raw mut out, ptrs.as_ptr().cast_mut(), 2) };
                        assert!(rc < 0 && out.is_null());
                        assert_out_of_memory();
                        assert_eq!(take_calls(), [Call::Alloc(array)]);
                        REFUSE.store(5, Relaxed);
                        // SAFETY: two live strings borrowed for the call; `out` is a writable
                        // slot, which a refused copy leaves untouched.
                        let rc = unsafe { c_copy(&raw mut out, ptrs.as_ptr().cast_mut(), 2) };
                        assert!(rc < 0 && out.is_null());
                        assert_out_of_memory();
                        assert_eq!(take_calls(), requests, "C leaks both");
                        let rust = if null_terminated {
                            NullStrListFree::copy(&strs).is_some()
                        } else {
                            StrListFree::copy(&strs).is_some()
                        };
                        assert!(!rust);
                        assert_out_of_memory();
                        let released = [&requests[..], &[Call::Free; 2]].concat();
                        assert_eq!(take_calls(), released, "the wrapper releases both");
                        REFUSE.store(NONE, Relaxed);
                    }
                    // `git_strlist_copy_with_null` refuses a count whose terminator
                    // overflows before it reads `in` or allocates; no slice reaches it.
                    let mut out: *mut *mut c_char = ptr::null_mut();
                    // SAFETY: C checks `len + 1` first and returns, reading nothing.
                    let rc = unsafe {
                        ffi::git_strlist_copy_with_null(&raw mut out, ptr::null_mut(), usize::MAX)
                    };
                    assert!(rc < 0 && out.is_null());
                    assert_out_of_memory();
                    assert_eq!(take_calls(), []);
                },
            );
        }

        /// A key's name is found with its delimiter: a longer name that begins with it is
        /// not the key's, where `git_strlist_contains_key` finds it
        /// (`logic/contains-key-matches-longer-names`).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn contains_key_compares_the_delimiter() {
            let env = [c"FOOBAR=2", c"PATH=/bin:/usr/bin", c"NOEQ", c"EMPTY="];
            for (key, delimiter, found) in [
                (c"FOO=1", b'=', false),
                (c"FOOBAR=1", b'=', true),
                (c"EMPTY=x", b'=', true),
                (c"EMPTY", b'=', false),
                (c"=leading", b'=', false),
                (c"PATH:x", b':', false),
                (c"PATH=/bin:", b':', true),
                (c"NOEQ=1", b'=', false),
                (c"FOOBAR=2", 0, false),
                (c"", b'=', false),
            ] {
                assert_eq!(contains_key(&env, key, delimiter), found, "{key:?}");
            }
            let empty: [&CStr; 0] = [];
            assert!(!contains_key(&empty, c"FOO=1", b'='));
            assert!(contains_key(&[c"=x"], c"=leading", b'='));
        }

        /// The lists compare, hash and iterate as their strings do, whatever their arrays.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn lists_compare_and_iterate_as_their_strings() {
            use std::collections::hash_map::DefaultHasher;

            fn hash<T: Hash + ?Sized>(t: &T) -> u64 {
                let mut h = DefaultHasher::new();
                t.hash(&mut h);
                h.finish()
            }

            let mut a = StrListFree::copy(&[c"x", c"y"]).unwrap();
            let b = a.clone();
            assert!(a == b && hash(&a) == hash(&b) && a.as_ptr() != b.as_ptr());
            assert_eq!(hash(&a), hash(&a[..]));
            for s in &mut a {
                *s = StrFree::dup(c"z").unwrap();
            }
            let zs = vec![StrFree::dup(c"z").unwrap(), StrFree::dup(c"z").unwrap()];
            assert!(a != b && a == *zs.as_slice());
            let seen: Vec<&CStr> = (&b).into_iter().map(AllocStr::as_c_str).collect();
            assert_eq!(seen, [c"x", c"y"]);
            assert_eq!(b.as_ref().len(), 2);
            a.as_mut().swap(0, 1);

            let empty = StrListFree::copy::<&CStr>(&[]).unwrap();
            assert!(empty == StrList::new() && hash(&empty) == hash(&StrList::new()));

            let argv = NullStrListFree::copy(&[c"x", c"y"]).unwrap();
            assert!(argv == argv.clone() && argv[..] == b[..] && hash(&argv) == hash(&b));
        }
    }
}
