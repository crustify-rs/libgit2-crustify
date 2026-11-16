//! Port of `src/util/vector.c`: `git_vector`, libgit2's growable array of
//! pointers.
//!
//! [`GitVector`] is the mirrored `git_vector` (kind 3: C embeds it by
//! value, reads `contents` and `length` directly, and uses its inline
//! functions and `git_vector_foreach`). Its array comes from
//! `git__allocator`, grows as the C's (8, then ×1.5), and is sorted with
//! the port of `tsort.c`.
//!
//! The elements are raw pointers, which the vector neither owns nor
//! dereferences. The comparison function is a C function pointer; the
//! methods that call it are `unsafe` (their contract: it is sound to call
//! on the elements). Rust comparators go through the `_by` methods.
//!
//! ```
//! use core::ffi::c_void;
//! use libgit2_rs::util::vector::GitVector;
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut v = GitVector::with_capacity(0, None)?;
//! let (mut a, mut b) = (1u32, 2u32);
//! v.insert((&raw mut b).cast())?;
//! v.insert((&raw mut a).cast())?;
//! // SAFETY (for the example): the elements point to live `u32`s.
//! v.sort_by(|x, y| unsafe { *x.cast::<u32>() as i32 - *y.cast::<u32>() as i32 });
//! assert_eq!(v.get(0), Some((&raw mut a).cast::<c_void>()));
//! # Ok::<(), libgit2_rs::util::errors::ErrorCode>(())
//! ```

use core::ffi::{c_int, c_void};
use core::ptr;

use super::alloc;
use super::errors::{ErrorCode, Result};
use super::tsort::tsort;
use super::util::bsearch;
use crate::{git_assert, git_assert_arg};

/// A Rust duplicate handler for [`GitVector::insert_sorted_by`]: given the
/// equal element's slot and the new element; a negative result cancels.
pub type OnDup<'a> = &'a mut dyn FnMut(&mut *mut c_void, *mut c_void) -> c_int;

/// `git_vector_cmp`.
pub type VectorCmp = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

/// `GIT_VECTOR_SORTED`.
pub const GIT_VECTOR_SORTED: u32 = 1 << 0;

/// In elements, not bytes.
const MIN_ALLOCSIZE: usize = 8;

/// `git_vector`, mirrored.
///
/// Invariant: `contents` is NULL, or an allocation of `git__allocator`
/// holding `_alloc_size` pointers that this vector owns, of which the
/// first `length` are initialized; `length <= _alloc_size` (both are 0 when
/// `contents` is NULL). Dropping it frees the array (not the elements), as
/// `git_vector_dispose`, which leaves it empty, so a drop after a C dispose
/// frees nothing.
#[repr(C)]
pub struct GitVector {
    alloc_size: usize,
    cmp: Option<VectorCmp>,
    contents: *mut *mut c_void,
    length: usize,
    flags: u32,
}

// SAFETY: the vector owns its array, which `git__allocator` frees from any
// thread; it never dereferences its elements (the methods that call the C
// comparison are `unsafe`, and their contract covers the elements).
unsafe impl Send for GitVector {}
// SAFETY: shared access only reads the array of pointers; every method that
// writes takes `&mut self`.
unsafe impl Sync for GitVector {}

impl Drop for GitVector {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Default for GitVector {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for GitVector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitVector")
            .field("length", &self.length)
            .field("alloc_size", &self.alloc_size)
            .field("sorted", &self.is_sorted())
            .field("contents", &self.as_slice())
            .finish()
    }
}

fn same_cmp(a: Option<VectorCmp>, b: Option<VectorCmp>) -> bool {
    a.map(|f| f as usize) == b.map(|f| f as usize)
}

/// `strict_comparison`: pointer identity.
unsafe extern "C" fn strict_comparison(a: *const c_void, b: *const c_void) -> c_int {
    if a == b { 0 } else { -1 }
}

impl GitVector {
    /// An empty vector with no array and no comparison (`GIT_VECTOR_INIT`,
    /// all zero; not marked sorted).
    pub const fn new() -> GitVector {
        GitVector {
            alloc_size: 0,
            cmp: None,
            contents: ptr::null_mut(),
            length: 0,
            flags: 0,
        }
    }

    /// `git_vector_init`: an empty, sorted vector with room for
    /// `max(initial_size, 8)` elements.
    pub fn with_capacity(initial_size: usize, cmp: Option<VectorCmp>) -> Result<GitVector> {
        let mut v = GitVector {
            alloc_size: 0,
            cmp,
            contents: ptr::null_mut(),
            length: 0,
            flags: GIT_VECTOR_SORTED,
        };
        v.resize_vector(initial_size.max(MIN_ALLOCSIZE))?;
        Ok(v)
    }

    // ----- accessors -----

    /// `git_vector_length`.
    #[inline]
    pub fn len(&self) -> usize {
        self.length
    }

    /// Whether the vector has no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// `_alloc_size`.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.alloc_size
    }

    /// The elements.
    #[inline]
    pub fn as_slice(&self) -> &[*mut c_void] {
        if self.contents.is_null() || self.length == 0 {
            return &[];
        }
        // SAFETY: the invariant: `length` initialized pointers in the
        // vector's own array; `&self` excludes writers.
        unsafe { core::slice::from_raw_parts(self.contents, self.length) }
    }

    /// The elements, mutable.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [*mut c_void] {
        if self.contents.is_null() || self.length == 0 {
            return &mut [];
        }
        // SAFETY: as in `as_slice`, with `&mut self` making this the only
        // access.
        unsafe { core::slice::from_raw_parts_mut(self.contents, self.length) }
    }

    /// The whole array (`_alloc_size` slots), for writing.
    #[inline]
    fn slots(&mut self) -> &mut [core::mem::MaybeUninit<*mut c_void>] {
        if self.contents.is_null() {
            return &mut [];
        }
        // SAFETY: the invariant: an array of `alloc_size` pointers that this
        // vector owns; `MaybeUninit` admits the unwritten ones.
        unsafe { core::slice::from_raw_parts_mut(self.contents.cast(), self.alloc_size) }
    }

    /// `git_vector_get`: the element at `position`, if any.
    #[inline]
    pub fn get(&self, position: usize) -> Option<*mut c_void> {
        self.as_slice().get(position).copied()
    }

    /// `git_vector_last`.
    #[inline]
    pub fn last(&self) -> Option<*mut c_void> {
        self.as_slice().last().copied()
    }

    /// The elements, in order.
    pub fn iter(&self) -> impl Iterator<Item = *mut c_void> + '_ {
        self.as_slice().iter().copied()
    }

    /// `git_vector_is_sorted`.
    #[inline]
    pub fn is_sorted(&self) -> bool {
        self.flags & GIT_VECTOR_SORTED != 0
    }

    /// `git_vector_set_sorted`.
    #[inline]
    pub fn set_sorted(&mut self, sorted: bool) {
        if sorted {
            self.flags |= GIT_VECTOR_SORTED;
        } else {
            self.flags &= !GIT_VECTOR_SORTED;
        }
    }

    /// The comparison function.
    #[inline]
    pub fn cmp_fn(&self) -> Option<VectorCmp> {
        self.cmp
    }

    /// `git_vector_set_cmp`: a different function unsets the sorted flag.
    pub fn set_cmp(&mut self, cmp: Option<VectorCmp>) {
        if !same_cmp(cmp, self.cmp) {
            self.cmp = cmp;
            self.set_sorted(false);
        }
    }

    /// The array, for C (`contents`).
    #[inline]
    pub fn as_ptr(&self) -> *const *mut c_void {
        self.contents
    }

    // ----- allocation -----

    fn compute_new_size(&self) -> usize {
        let n = self.alloc_size;
        if n < MIN_ALLOCSIZE {
            MIN_ALLOCSIZE
        } else if n <= (usize::MAX / 3) * 2 {
            n + n / 2
        } else {
            usize::MAX
        }
    }

    fn resize_vector(&mut self, new_size: usize) -> Result<()> {
        if new_size == 0 {
            return Ok(());
        }
        // SAFETY: `contents` is NULL or the vector's own array from
        // `git__allocator`; on failure it is left as it was.
        let p = unsafe {
            alloc::reallocarray(self.contents.cast(), new_size, size_of::<*mut c_void>())
        };
        if p.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        self.alloc_size = new_size;
        self.contents = p.cast();
        Ok(())
    }

    /// `git_vector_size_hint`: room for `size_hint` elements.
    pub fn size_hint(&mut self, size_hint: usize) -> Result<()> {
        if self.alloc_size >= size_hint {
            return Ok(());
        }
        self.resize_vector(size_hint)
    }

    /// `git_vector_dup`: a copy of `self`'s elements, with `cmp` (or
    /// `self`'s), sorted only if `self` is and the function is the same.
    pub fn dup(&self, cmp: Option<VectorCmp>) -> Result<GitVector> {
        let mut v = GitVector {
            alloc_size: 0,
            cmp: if cmp.is_some() { cmp } else { self.cmp },
            contents: ptr::null_mut(),
            length: 0,
            flags: self.flags,
        };
        if !same_cmp(cmp, self.cmp) {
            v.set_sorted(false);
        }
        if self.length > 0 {
            let bytes = super::integer::alloc_multiply(self.length, size_of::<*mut c_void>())?;
            let p = alloc::malloc(bytes).cast::<*mut c_void>();
            if p.is_null() {
                return Err(ErrorCode::GENERIC);
            }
            // SAFETY: `p` is a fresh array of `length` pointers, disjoint
            // from `self`'s, which holds `length` initialized ones.
            unsafe { ptr::copy_nonoverlapping(self.contents, p, self.length) };
            v.contents = p;
            v.alloc_size = self.length;
            v.length = self.length;
        }
        Ok(v)
    }

    /// `git_vector_dispose`: free the array (not the elements).
    pub fn dispose(&mut self) {
        // SAFETY: the vector's own array (or NULL), freed once.
        unsafe { alloc::free(self.contents.cast()) };
        self.contents = ptr::null_mut();
        self.length = 0;
        self.alloc_size = 0;
    }

    /// `git_vector_dispose_deep`: free each element with `git__free`, then
    /// the array.
    ///
    /// # Safety
    ///
    /// Each element is NULL or an allocation of `git__allocator` that the
    /// vector owns, and they are distinct.
    pub unsafe fn dispose_deep(&mut self) {
        for e in self.as_mut_slice() {
            // SAFETY: the caller's contract: owned, distinct allocations.
            unsafe { alloc::free(*e) };
            *e = ptr::null_mut();
        }
        self.dispose();
    }

    /// `git_vector_detach`: take the array, with its length and size,
    /// leaving the vector empty.
    pub fn detach(&mut self) -> (*mut *mut c_void, usize, usize) {
        let r = (self.contents, self.length, self.alloc_size);
        self.alloc_size = 0;
        self.length = 0;
        self.contents = ptr::null_mut();
        r
    }

    // ----- insertion and removal -----

    /// `git_vector_insert`: append `element`.
    #[inline]
    pub fn insert(&mut self, element: *mut c_void) -> Result<()> {
        if self.length >= self.alloc_size {
            self.resize_vector(self.compute_new_size())?;
        }
        let n = self.length;
        self.slots()[n].write(element);
        self.length = n + 1;
        self.set_sorted(self.length <= 1);
        Ok(())
    }

    /// `git_vector_insert_sorted` with a Rust comparator and duplicate
    /// handler: insert `element` in order (sorting first if needed); when
    /// an equal element exists and `on_dup` returns a negative value, that
    /// value is returned and nothing is inserted.
    pub fn insert_sorted_by(
        &mut self,
        element: *mut c_void,
        mut cmp: impl FnMut(*mut c_void, *mut c_void) -> c_int,
        on_dup: Option<OnDup<'_>>,
    ) -> Result<()> {
        if !self.is_sorted() {
            tsort(self.as_mut_slice(), &mut cmp);
            self.set_sorted(true);
        }
        if self.length >= self.alloc_size {
            self.resize_vector(self.compute_new_size())?;
        }
        let found = bsearch(self.as_slice(), |e| cmp(element, *e));
        let pos = match found {
            Ok(p) => {
                if let Some(on_dup) = on_dup {
                    let r = on_dup(&mut self.as_mut_slice()[p], element);
                    if r < 0 {
                        return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
                    }
                }
                p
            }
            Err(p) => p,
        };
        let n = self.length;
        let slots = self.slots();
        slots.copy_within(pos..n, pos + 1);
        slots[pos].write(element);
        self.length = n + 1;
        Ok(())
    }

    /// `git_vector_insert_sorted`, with the vector's comparison function
    /// (which must be set: `unrecoverable internal error: 'v->_cmp'`).
    ///
    /// # Safety
    ///
    /// The comparison function and `on_dup` are sound to call with the
    /// vector's elements and `element` (and a pointer to a slot).
    pub unsafe fn insert_sorted(
        &mut self,
        element: *mut c_void,
        on_dup: Option<unsafe extern "C" fn(*mut *mut c_void, *mut c_void) -> c_int>,
    ) -> Result<()> {
        let Some(cmp) = self.cmp else {
            git_assert!(false, "v->_cmp");
            unreachable!()
        };
        // SAFETY: the caller vouches for `cmp` and `on_dup`.
        let cmp = |a: *mut c_void, b: *mut c_void| unsafe { cmp(a, b) };
        match on_dup {
            Some(f) => {
                // SAFETY: as above.
                let mut d = |old: &mut *mut c_void, new: *mut c_void| unsafe { f(old, new) };
                self.insert_sorted_by(element, cmp, Some(&mut d))
            }
            None => self.insert_sorted_by(element, cmp, None),
        }
    }

    /// `git_vector_remove`: remove the element at `idx`
    /// (`GIT_ENOTFOUND` past the end).
    pub fn remove(&mut self, idx: usize) -> Result<()> {
        if idx >= self.length {
            return Err(ErrorCode::NOT_FOUND);
        }
        let n = self.length;
        self.as_mut_slice().copy_within(idx + 1..n, idx);
        self.length = n - 1;
        Ok(())
    }

    /// `git_vector_pop`.
    pub fn pop(&mut self) {
        if self.length > 0 {
            self.length -= 1;
        }
    }

    /// `git_vector_clear`: no elements, sorted (the array is kept).
    pub fn clear(&mut self) {
        self.length = 0;
        self.set_sorted(true);
    }

    /// `git_vector_swap`.
    pub fn swap(&mut self, other: &mut GitVector) {
        core::mem::swap(self, other);
    }

    /// `git_vector_resize_to`: set the length, new elements NULL.
    pub fn resize_to(&mut self, new_length: usize) -> Result<()> {
        if new_length > self.alloc_size {
            self.resize_vector(new_length)?;
        }
        let n = self.length;
        if new_length > n {
            for s in &mut self.slots()[n..new_length] {
                s.write(ptr::null_mut());
            }
        }
        self.length = new_length;
        Ok(())
    }

    /// `git_vector_insert_null`: insert `insert_len` NULLs at `idx`.
    pub fn insert_null(&mut self, idx: usize, insert_len: usize) -> Result<()> {
        git_assert_arg!(insert_len > 0, "insert_len > 0");
        git_assert_arg!(idx <= self.length, "idx <= v->length");
        let new_length = super::integer::alloc_add(self.length, insert_len)?;
        if new_length > self.alloc_size {
            self.resize_vector(new_length)?;
        }
        let n = self.length;
        let slots = self.slots();
        slots.copy_within(idx..n, idx + insert_len);
        for s in &mut slots[idx..idx + insert_len] {
            s.write(ptr::null_mut());
        }
        self.length = new_length;
        Ok(())
    }

    /// `git_vector_remove_range`: remove `remove_len` elements at `idx`.
    pub fn remove_range(&mut self, idx: usize, remove_len: usize) -> Result<()> {
        git_assert_arg!(remove_len > 0, "remove_len > 0");
        let Some(end_idx) = idx.checked_add(remove_len) else {
            git_assert!(false, "0");
            unreachable!()
        };
        git_assert!(end_idx <= self.length, "end_idx <= v->length");
        let n = self.length;
        let new_length = n - remove_len;
        let slots = self.slots();
        slots.copy_within(end_idx..n, idx);
        for s in &mut slots[new_length..n] {
            s.write(ptr::null_mut());
        }
        self.length = new_length;
        Ok(())
    }

    /// `git_vector_set`: set the element at `position` (growing with NULLs
    /// to reach it), returning the old one.
    pub fn set(&mut self, position: usize, value: *mut c_void) -> Result<*mut c_void> {
        if position >= self.length {
            let len = super::integer::alloc_add(position, 1)?;
            self.resize_to(len)?;
        }
        let slot = &mut self.as_mut_slice()[position];
        Ok(core::mem::replace(slot, value))
    }

    /// `git_vector_reverse`.
    pub fn reverse(&mut self) {
        self.as_mut_slice().reverse();
    }

    /// `git_vector_remove_matching` with a Rust predicate: remove the
    /// elements for which `matches(self, idx)` is true, called as C calls
    /// its callback (after the element is moved to `idx`).
    pub fn remove_matching(&mut self, mut matches: impl FnMut(&GitVector, usize) -> bool) {
        let mut i = 0;
        let n = self.length;
        for j in 0..n {
            let s = self.as_mut_slice();
            s[i] = s[j];
            if !matches(self, i) {
                i += 1;
            }
        }
        self.length = i;
    }

    // ----- sorting and searching -----

    /// Sort with a Rust comparator (the vector's flags are unchanged).
    pub fn sort_by(&mut self, cmp: impl FnMut(*mut c_void, *mut c_void) -> c_int) {
        tsort(self.as_mut_slice(), cmp);
    }

    /// `git_vector_sort`: sort with the vector's comparison function, unless
    /// sorted already or there is none.
    ///
    /// # Safety
    ///
    /// The comparison function is sound to call with any two elements.
    pub unsafe fn sort(&mut self) {
        let Some(cmp) = self.cmp else { return };
        if self.is_sorted() {
            return;
        }
        if self.length > 1 {
            // SAFETY: the caller vouches for `cmp` on the elements.
            tsort(self.as_mut_slice(), |a, b| unsafe { cmp(a, b) });
        }
        self.set_sorted(true);
    }

    /// Binary search with a Rust comparator (`cmp(element)` is the key's
    /// order against `element`), without sorting first: `Ok(index)` or
    /// `Err(insertion index)`.
    pub fn bsearch_by(
        &self,
        cmp: impl FnMut(*mut c_void) -> c_int,
    ) -> core::result::Result<usize, usize> {
        let mut cmp = cmp;
        bsearch(self.as_slice(), |e| cmp(*e))
    }

    /// `git_vector_bsearch2`: sort (with the vector's function), then
    /// binary-search with `key_lookup`. Without a comparison function, -1
    /// with no error set, as C. `Err(GIT_ENOTFOUND)` carries the insertion
    /// position in `pos`.
    ///
    /// # Safety
    ///
    /// The vector's comparison function is sound to call with any two
    /// elements, and `key_lookup` with `key` and any element.
    pub unsafe fn bsearch2(
        &mut self,
        pos: Option<&mut usize>,
        key_lookup: VectorCmp,
        key: *const c_void,
    ) -> Result<()> {
        git_assert_arg!(!key.is_null(), "key");
        if self.cmp.is_none() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: forwarded.
        unsafe { self.sort() };
        // SAFETY: the caller vouches for `key_lookup`.
        let r = bsearch(self.as_slice(), |e| unsafe { key_lookup(key, *e) });
        let (p, ret) = match r {
            Ok(p) => (p, Ok(())),
            Err(p) => (p, Err(ErrorCode::NOT_FOUND)),
        };
        if let Some(out) = pos {
            *out = p;
        }
        ret
    }

    /// `git_vector_bsearch`: [`bsearch2`](Self::bsearch2) with the vector's
    /// own comparison function.
    ///
    /// # Safety
    ///
    /// As [`bsearch2`](Self::bsearch2).
    pub unsafe fn bsearch(&mut self, pos: Option<&mut usize>, key: *const c_void) -> Result<()> {
        let Some(cmp) = self.cmp else {
            git_assert!(false, "key_lookup");
            unreachable!()
        };
        // SAFETY: forwarded.
        unsafe { self.bsearch2(pos, cmp, key) }
    }

    /// `git_vector_search2`: the first index where `key_lookup(key,
    /// element)` is 0 (`GIT_ENOTFOUND` otherwise).
    ///
    /// # Safety
    ///
    /// `key_lookup` is sound to call with `key` and any element.
    pub unsafe fn search2(&self, key_lookup: VectorCmp, key: *const c_void) -> Result<usize> {
        git_assert_arg!(!key.is_null(), "key");
        // SAFETY: the caller vouches for `key_lookup`.
        self.iter()
            .position(|e| unsafe { key_lookup(key, e) } == 0)
            .ok_or(ErrorCode::NOT_FOUND)
    }

    /// `git_vector_search`: [`search2`](Self::search2) with the vector's
    /// function, or pointer identity without one.
    ///
    /// # Safety
    ///
    /// The vector's comparison function is sound to call with `entry` and
    /// any element.
    pub unsafe fn search(&self, entry: *const c_void) -> Result<usize> {
        let f = self.cmp.unwrap_or(strict_comparison);
        // SAFETY: forwarded (`strict_comparison` reads no memory).
        unsafe { self.search2(f, entry) }
    }

    /// `git_vector_uniq`: sort, then drop adjacent duplicates (by the
    /// vector's function, or identity), passing each dropped one to
    /// `free_cb`.
    ///
    /// # Safety
    ///
    /// The comparison function is sound to call with any two elements, and
    /// `free_cb` with each dropped one.
    pub unsafe fn uniq(&mut self, free_cb: Option<unsafe extern "C" fn(*mut c_void)>) {
        if self.length <= 1 {
            return;
        }
        // SAFETY: forwarded.
        unsafe { self.sort() };
        let cmp = self.cmp.unwrap_or(strict_comparison);
        let s = self.as_mut_slice();
        let mut i = 0;
        let mut j = 1;
        while j < s.len() {
            // SAFETY: the caller vouches for `cmp` and `free_cb`.
            if unsafe { cmp(s[i], s[j]) } == 0 {
                if let Some(f) = free_cb {
                    // SAFETY: as above.
                    unsafe { f(s[i]) };
                }
                s[i] = s[j];
            } else {
                i += 1;
                s[i] = s[j];
            }
            j += 1;
        }
        self.length -= j - i - 1;
    }

    /// `git_vector_verify_sorted`: whether the sorted flag is set and the
    /// elements are in order (0 or -1). Without a comparison function, 0
    /// (C calls NULL).
    ///
    /// # Safety
    ///
    /// The comparison function is sound to call with any two elements.
    pub unsafe fn verify_sorted(&self) -> c_int {
        if !self.is_sorted() {
            return -1;
        }
        let Some(cmp) = self.cmp else { return 0 };
        let s = self.as_slice();
        // SAFETY: the caller vouches for `cmp`.
        if s.windows(2).any(|w| unsafe { cmp(w[0], w[1]) } > 0) {
            -1
        } else {
            0
        }
    }
}

/// The exports of `vector.c`.
///
/// Every `git_vector *` argument is a valid `git_vector` that no other
/// thread accesses during the call, and its comparison function (and every
/// callback passed) is sound to call with its elements and the keys passed.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, status, write_opt};

    /// # Safety
    ///
    /// `v` is a valid `git_vector`, not otherwise accessed for `'a`.
    #[inline(always)]
    unsafe fn v<'a>(v: *mut GitVector) -> &'a mut GitVector {
        // SAFETY: the caller's contract.
        unsafe { &mut *v }
    }

    /// `git_vector_init`.
    ///
    /// # Safety
    ///
    /// `v` is NULL or writable (its old contents are overwritten).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_init(
        vec: *mut GitVector,
        initial_size: usize,
        cmp: Option<VectorCmp>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            let mut nv = GitVector {
                alloc_size: 0,
                cmp,
                contents: ptr::null_mut(),
                length: 0,
                flags: GIT_VECTOR_SORTED,
            };
            let r = nv.resize_vector(initial_size.max(MIN_ALLOCSIZE));
            // SAFETY: a writable `git_vector`; C does not free the old one.
            unsafe { vec.write(nv) };
            status(r)
        })
    }

    /// `git_vector_dispose` (NULL allowed).
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_dispose(vec: *mut GitVector) {
        barrier(|| {
            if !vec.is_null() {
                // SAFETY: the module's contract.
                unsafe { v(vec) }.dispose()
            }
        })
    }

    /// `git_vector_dispose_deep` (NULL allowed).
    ///
    /// # Safety
    ///
    /// See the [module](self); the elements are distinct `git__malloc`
    /// allocations (or NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_dispose_deep(vec: *mut GitVector) {
        barrier(|| {
            if !vec.is_null() {
                // SAFETY: the module's contract: a valid, unaliased vector.
                let vv = unsafe { v(vec) };
                // SAFETY: the elements are distinct `git__malloc`
                // allocations or NULL (this export's contract).
                unsafe { vv.dispose_deep() }
            }
        })
    }

    /// `git_vector_clear`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_clear(vec: *mut GitVector) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { v(vec) }.clear())
    }

    /// `git_vector_dup`.
    ///
    /// # Safety
    ///
    /// `vec` is NULL or writable; `src` is NULL or a valid `git_vector`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_dup(
        vec: *mut GitVector,
        src: *const GitVector,
        cmp: Option<VectorCmp>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            git_assert_arg!(!src.is_null(), "src", -1);
            // SAFETY: a valid source, only read.
            let s = unsafe { &*src };
            match s.dup(cmp) {
                Ok(d) => {
                    // SAFETY: a writable target.
                    unsafe { vec.write(d) };
                    0
                }
                Err(e) => {
                    // C leaves `v` with the source's length, flags and
                    // function and no array; do the same.
                    let mut d = GitVector::new();
                    d.cmp = if cmp.is_some() { cmp } else { s.cmp };
                    d.flags = s.flags;
                    if !same_cmp(cmp, s.cmp) {
                        d.set_sorted(false);
                    }
                    // SAFETY: a writable target.
                    unsafe { vec.write(d) };
                    e.raw()
                }
            }
        })
    }

    /// `git_vector_swap`.
    ///
    /// # Safety
    ///
    /// Both are valid (they may be the same).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_swap(a: *mut GitVector, b: *mut GitVector) {
        // SAFETY: valid vectors; `ptr::swap` allows them to be equal.
        barrier(|| unsafe { ptr::swap(a, b) })
    }

    /// `git_vector_size_hint`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_size_hint(vec: *mut GitVector, size_hint: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { v(vec) }.size_hint(size_hint)))
    }

    /// `git_vector_detach`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `size` and `asize` are NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_detach(
        size: *mut usize,
        asize: *mut usize,
        vec: *mut GitVector,
    ) -> *mut *mut c_void {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased vector.
            let (p, n, a) = unsafe { v(vec) }.detach();
            // SAFETY: `size` is NULL or writable.
            unsafe { write_opt(size, n) };
            // SAFETY: `asize` is NULL or writable.
            unsafe { write_opt(asize, a) };
            p
        })
    }

    /// `git_vector_sort`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_sort(vec: *mut GitVector) {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased vector.
            let vv = unsafe { v(vec) };
            // SAFETY: the module's contract covers the comparison function.
            unsafe { vv.sort() }
        })
    }

    /// `git_vector_search`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `at_pos` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_search(
        at_pos: *mut usize,
        vec: *const GitVector,
        entry: *const c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            // SAFETY: the module's contract: a valid vector, only read.
            let vv = unsafe { &*vec };
            // SAFETY: the module's contract covers the comparison function.
            match unsafe { vv.search(entry) } {
                Ok(i) => {
                    // SAFETY: optional out-parameter.
                    unsafe { write_opt(at_pos, i) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_vector_search2`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `at_pos` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_search2(
        at_pos: *mut usize,
        vec: *const GitVector,
        key_lookup: Option<VectorCmp>,
        key: *const c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            git_assert_arg!(!key.is_null(), "key", -1);
            let Some(f) = key_lookup else {
                git_assert!(false, "key_lookup", -1);
                unreachable!()
            };
            // SAFETY: the module's contract: a valid vector, only read.
            let vv = unsafe { &*vec };
            // SAFETY: the module's contract covers `key_lookup`.
            match unsafe { vv.search2(f, key) } {
                Ok(i) => {
                    // SAFETY: optional out-parameter.
                    unsafe { write_opt(at_pos, i) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_vector_bsearch2`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `at_pos` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_bsearch2(
        at_pos: *mut usize,
        vec: *mut GitVector,
        key_lookup: Option<VectorCmp>,
        key: *const c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            git_assert_arg!(!key.is_null(), "key", -1);
            let Some(f) = key_lookup else {
                git_assert!(false, "key_lookup", -1);
                unreachable!()
            };
            // SAFETY: `at_pos` is NULL or writable, and distinct from the vector.
            let pos = unsafe { at_pos.as_mut() };
            // SAFETY: the module's contract: a valid, unaliased vector.
            let vv = unsafe { v(vec) };
            // SAFETY: the module's contract covers both functions.
            status(unsafe { vv.bsearch2(pos, f, key) })
        })
    }

    /// `git_vector_insert`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_insert(vec: *mut GitVector, element: *mut c_void) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            // SAFETY: the module's contract.
            status(unsafe { v(vec) }.insert(element))
        })
    }

    /// `git_vector_insert_sorted`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_insert_sorted(
        vec: *mut GitVector,
        element: *mut c_void,
        on_dup: Option<unsafe extern "C" fn(*mut *mut c_void, *mut c_void) -> c_int>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            // SAFETY: the module's contract: a valid, unaliased vector.
            let vv = unsafe { v(vec) };
            // SAFETY: the module's contract covers the comparison and `on_dup`.
            status(unsafe { vv.insert_sorted(element, on_dup) })
        })
    }

    /// `git_vector_remove`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_remove(vec: *mut GitVector, idx: usize) -> c_int {
        barrier(|| {
            git_assert_arg!(!vec.is_null(), "v", -1);
            // SAFETY: the module's contract.
            status(unsafe { v(vec) }.remove(idx))
        })
    }

    /// `git_vector_pop`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_pop(vec: *mut GitVector) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { v(vec) }.pop())
    }

    /// `git_vector_uniq`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_uniq(
        vec: *mut GitVector,
        git_free_cb: Option<unsafe extern "C" fn(*mut c_void)>,
    ) {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased vector.
            let vv = unsafe { v(vec) };
            // SAFETY: the module's contract covers the comparison and `git_free_cb`.
            unsafe { vv.uniq(git_free_cb) }
        })
    }

    /// `git_vector_remove_matching`. The callback receives the vector, so
    /// it is only reached through raw pointers here.
    ///
    /// # Safety
    ///
    /// See the [module](self); `match_` is sound to call with the vector,
    /// an index and `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_remove_matching(
        vec: *mut GitVector,
        match_: unsafe extern "C" fn(*const GitVector, usize, *mut c_void) -> c_int,
        payload: *mut c_void,
    ) {
        barrier(|| {
            let mut i = 0usize;
            let mut j = 0usize;
            // No Rust reference to the vector is held across the callback,
            // which may read it; `length` and `contents` are read afresh.
            loop {
                // SAFETY: `vec` is a valid vector (the module's contract).
                let len = unsafe { (*vec).length };
                // SAFETY: as above.
                let c = unsafe { (*vec).contents };
                if j >= len {
                    break;
                }
                // SAFETY: `i <= j < length <= _alloc_size`: both slots are
                // initialized elements of the vector's array.
                let e = unsafe { c.wrapping_add(j).read() };
                // SAFETY: as above.
                unsafe { c.wrapping_add(i).write(e) };
                // SAFETY: the module's contract covers `match_`.
                if unsafe { match_(vec, i, payload) } == 0 {
                    i += 1;
                }
                j += 1;
            }
            // SAFETY: `vec` is valid; `i` is at most its length.
            unsafe { (*vec).length = i };
        })
    }

    /// `git_vector_resize_to`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_resize_to(vec: *mut GitVector, new_length: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { v(vec) }.resize_to(new_length)))
    }

    /// `git_vector_insert_null`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_insert_null(
        vec: *mut GitVector,
        idx: usize,
        insert_len: usize,
    ) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { v(vec) }.insert_null(idx, insert_len)))
    }

    /// `git_vector_remove_range`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_remove_range(
        vec: *mut GitVector,
        idx: usize,
        remove_len: usize,
    ) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { v(vec) }.remove_range(idx, remove_len)))
    }

    /// `git_vector_set`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `old` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_set(
        old: *mut *mut c_void,
        vec: *mut GitVector,
        position: usize,
        value: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            match unsafe { v(vec) }.set(position, value) {
                Ok(prev) => {
                    // SAFETY: optional out-parameter.
                    unsafe { write_opt(old, prev) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_vector_verify_sorted`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_verify_sorted(vec: *const GitVector) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract: a valid vector, only read.
            let vv = unsafe { &*vec };
            // SAFETY: the module's contract covers the comparison function.
            unsafe { vv.verify_sorted() }
        })
    }

    /// `git_vector_reverse`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_vector_reverse(vec: *mut GitVector) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { v(vec) }.reverse())
    }
}

crate::c_layout_test! {
    headers: ["vector.h"],
    types: [
        GitVector => "git_vector" { alloc_size as "_alloc_size", cmp as "_cmp", contents, length, flags },
    ],
    asserts: ["GIT_VECTOR_SORTED == 1"],
}
