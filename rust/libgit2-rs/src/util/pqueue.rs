//! Port of `src/util/pqueue.c`: `git_pqueue`, a binary min-heap of pointers
//! over a `git_vector`.
//!
//! [`GitPqueue`] is the mirrored `git_pqueue`, which C declares as
//! `typedef git_vector git_pqueue` (kind 3: embedded by value, e.g. in
//! `git_revwalk`, and handled with the vector's macros `git_pqueue_free`,
//! `git_pqueue_size`, `git_pqueue_get`, …). The heap is C's: same sift up
//! and down, same element positions after each operation, same handling of
//! a full fixed-size queue (`GIT_PQUEUE_FIXED_SIZE`) and of a queue without
//! comparison function (a stack).
//!
//! A NULL element ends a sift down as the end of the array does, as in C
//! (`git_vector_get` returns NULL for both).
//!
//! ```
//! use core::ffi::{c_int, c_void};
//! use libgit2_rs::util::pqueue::GitPqueue;
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut vals = [5usize, 1, 4, 2, 3];
//! let mut pq = GitPqueue::new(0, 0, None)?;
//! // SAFETY (for the example): the elements point to `vals`.
//! let mut cmp = |a: *mut c_void, b: *mut c_void| unsafe { *a.cast::<usize>() as c_int - *b.cast::<usize>() as c_int };
//! for v in vals.iter_mut() {
//!     pq.insert_by((v as *mut usize).cast(), &mut cmp)?;
//! }
//! let first = pq.pop_by(&mut cmp);
//! assert_eq!(first, (&raw mut vals[1]).cast());
//! # Ok::<(), libgit2_rs::util::errors::ErrorCode>(())
//! ```

use core::ffi::{c_int, c_void};
use core::ptr;

use super::errors::Result;
use super::vector::{GitVector, VectorCmp};

/// `GIT_VECTOR_FLAG_MAX`.
const GIT_VECTOR_FLAG_MAX: u32 = 1 << 1;

/// `GIT_PQUEUE_FIXED_SIZE`: do not grow; keep the highest values only.
pub const GIT_PQUEUE_FIXED_SIZE: u32 = GIT_VECTOR_FLAG_MAX << 1;

/// A comparison of two elements, as a Rust closure.
pub type PqueueCmp<'a> = &'a mut dyn FnMut(*mut c_void, *mut c_void) -> c_int;

/// The comparison type of the calls without one.
type NoCmp = fn(*mut c_void, *mut c_void) -> c_int;

/// `git_pqueue`: a `git_vector` used as a heap.
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct GitPqueue(GitVector);

#[inline(always)]
const fn parent_of(i: usize) -> usize {
    i.wrapping_sub(1) >> 1
}

#[inline(always)]
const fn lchild_of(i: usize) -> usize {
    (i << 1) + 1
}

impl GitPqueue {
    /// `git_pqueue_init`: an empty queue with room for `init_size` elements
    /// (exactly `init_size`, when `GIT_PQUEUE_FIXED_SIZE` and it is > 0),
    /// ordered by `cmp`.
    pub fn new(flags: u32, init_size: usize, cmp: Option<VectorCmp>) -> Result<GitPqueue> {
        let mut v = GitVector::with_capacity(init_size, cmp)?;
        v.add_flags(flags);
        if flags & GIT_PQUEUE_FIXED_SIZE != 0 && init_size > 0 {
            v.limit_capacity(init_size);
        }
        Ok(GitPqueue(v))
    }

    /// The vector underneath (for `git_pqueue_get` & co.).
    #[inline]
    pub fn as_vector(&self) -> &GitVector {
        &self.0
    }

    /// The vector underneath, mutable (`git_pqueue_clear`,
    /// `git_pqueue_reverse`, `git_pqueue_free`).
    #[inline]
    pub fn as_vector_mut(&mut self) -> &mut GitVector {
        &mut self.0
    }

    /// `git_pqueue_size`.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `git_pqueue_get`: the element at heap position `i` (0 is the top).
    #[inline]
    pub fn get(&self, i: usize) -> Option<*mut c_void> {
        self.0.get(i)
    }

    /// `git_pqueue_clear`.
    #[inline]
    pub fn clear(&mut self) {
        self.0.clear();
    }

    #[inline(always)]
    fn at(&self, i: usize) -> *mut c_void {
        self.0.get(i).unwrap_or(ptr::null_mut())
    }

    fn up<F: FnMut(*mut c_void, *mut c_void) -> c_int + ?Sized>(
        &mut self,
        mut el: usize,
        cmp: &mut F,
    ) {
        let c = self.0.as_mut_slice();
        let Some(&kid) = c.get(el) else { return };
        while el > 0 {
            let parent_el = parent_of(el);
            let parent = c[parent_el];
            if cmp(parent, kid) <= 0 {
                break;
            }
            c[el] = parent;
            el = parent_el;
        }
        c[el] = kid;
    }

    /// Sift down from `el`. A NULL element ends it as the end does (C's
    /// `git_vector_get` returns NULL for both).
    fn down<F: FnMut(*mut c_void, *mut c_void) -> c_int + ?Sized>(
        &mut self,
        mut el: usize,
        cmp: &mut F,
    ) {
        let c = self.0.as_mut_slice();
        let n = c.len();
        let Some(&parent) = c.get(el) else { return };
        loop {
            let mut kid_el = lchild_of(el);
            if kid_el >= n || c[kid_el].is_null() {
                break;
            }
            let mut kid = c[kid_el];
            if kid_el + 1 < n {
                let rkid = c[kid_el + 1];
                if !rkid.is_null() && cmp(kid, rkid) > 0 {
                    kid = rkid;
                    kid_el += 1;
                }
            }
            if cmp(parent, kid) <= 0 {
                break;
            }
            c[el] = kid;
            el = kid_el;
        }
        c[el] = parent;
    }

    fn insert_impl<F: FnMut(*mut c_void, *mut c_void) -> c_int + ?Sized>(
        &mut self,
        item: *mut c_void,
        mut cmp: Option<&mut F>,
    ) -> Result<()> {
        if self.0.flags() & GIT_PQUEUE_FIXED_SIZE != 0 && self.0.len() >= self.0.capacity() {
            // skip the item if below the heap's minimum, or if there is
            // no comparison
            match cmp.as_mut() {
                None => return Ok(()),
                Some(c) => {
                    if c(item, self.at(0)) <= 0 {
                        return Ok(());
                    }
                }
            }
            let _ = self.pop_impl(cmp.as_deref_mut());
        }
        self.0.insert(item)?;
        if let Some(c) = cmp {
            let n = self.0.len();
            self.up(n - 1, c);
        }
        Ok(())
    }

    fn pop_impl<F: FnMut(*mut c_void, *mut c_void) -> c_int + ?Sized>(
        &mut self,
        cmp: Option<&mut F>,
    ) -> *mut c_void {
        let rval = match cmp {
            None => self.0.last(),
            Some(_) => self.0.get(0),
        }
        .unwrap_or(ptr::null_mut());
        match cmp {
            Some(c) if self.0.len() > 1 => {
                let last = self.at(self.0.len() - 1);
                self.0.as_mut_slice()[0] = last;
                self.0.pop();
                self.down(0, c);
            }
            _ => self.0.pop(),
        }
        rval
    }

    /// `git_pqueue_insert` with the queue's C comparison function.
    ///
    /// # Safety
    ///
    /// The comparison function (if any) is sound to call on `item` and on
    /// the queue's elements.
    pub unsafe fn insert(&mut self, item: *mut c_void) -> Result<()> {
        match self.0.cmp_fn() {
            None => self.insert_impl::<NoCmp>(item, None),
            Some(f) => {
                // SAFETY: the caller's contract.
                let mut c = |a: *mut c_void, b: *mut c_void| unsafe { f(a, b) };
                self.insert_impl(item, Some(&mut c))
            }
        }
    }

    /// `git_pqueue_pop` with the queue's C comparison function: the top
    /// element (the last one without a comparison), NULL if empty.
    ///
    /// # Safety
    ///
    /// The comparison function (if any) is sound to call on the queue's
    /// elements.
    pub unsafe fn pop(&mut self) -> *mut c_void {
        match self.0.cmp_fn() {
            None => self.pop_impl::<NoCmp>(None),
            Some(f) => {
                // SAFETY: the caller's contract.
                let mut c = |a: *mut c_void, b: *mut c_void| unsafe { f(a, b) };
                self.pop_impl(Some(&mut c))
            }
        }
    }

    /// `git_pqueue_insert` ordered by the Rust `cmp` (which takes the
    /// place of the queue's comparison function).
    pub fn insert_by(&mut self, item: *mut c_void, cmp: PqueueCmp<'_>) -> Result<()> {
        self.insert_impl(item, Some(cmp))
    }

    /// `git_pqueue_pop` ordered by the Rust `cmp`.
    pub fn pop_by(&mut self, cmp: PqueueCmp<'_>) -> *mut c_void {
        self.pop_impl(Some(cmp))
    }

    /// `git_pqueue_insert` without comparison: push (as a queue with no
    /// comparison function).
    pub fn push_unordered(&mut self, item: *mut c_void) -> Result<()> {
        self.insert_impl::<NoCmp>(item, None)
    }

    /// `git_pqueue_pop` without comparison: the last element.
    pub fn pop_unordered(&mut self) -> *mut c_void {
        self.pop_impl::<NoCmp>(None)
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `pqueue.c`.
pub mod ffi {
    use core::ffi::{c_int, c_void};

    use super::*;
    use crate::util::export::{barrier, status};

    /// `git_pqueue_init`: overwrite `*pq` with an empty queue.
    ///
    /// # Safety
    ///
    /// `pq` is NULL (an error, as C) or writable (it need not be
    /// initialized).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pqueue_init(
        pq: *mut GitPqueue,
        flags: u32,
        init_size: usize,
        cmp: Option<VectorCmp>,
    ) -> c_int {
        barrier(|| {
            // `git_vector_init`'s check.
            crate::git_assert_arg!(!pq.is_null(), "v", -1);
            // SAFETY: the caller's contract, `pq` non-null.
            unsafe { pqueue_init(pq, flags, init_size, cmp) }
        })
    }

    /// The body of [`git_pqueue_init`], `pq` non-null.
    ///
    /// # Safety
    ///
    /// As [`git_pqueue_init`], `pq` writable.
    unsafe fn pqueue_init(
        pq: *mut GitPqueue,
        flags: u32,
        init_size: usize,
        cmp: Option<VectorCmp>,
    ) -> c_int {
        match GitPqueue::new(flags, init_size, cmp) {
            Ok(q) => {
                // SAFETY: writable; the old contents are not read or
                // dropped (as C's `git_vector_init`).
                unsafe { pq.write(q) };
                0
            }
            Err(e) => {
                // C's `git_vector_init` leaves the vector empty, with its
                // comparison and sorted, on failure.
                let mut v = GitVector::new();
                v.set_cmp(cmp);
                v.set_sorted(true);
                // SAFETY: writable.
                unsafe { pq.write(GitPqueue(v)) };
                status(Err(e))
            }
        }
    }

    /// `git_pqueue_insert`.
    ///
    /// # Safety
    ///
    /// `pq` is a valid queue, not used concurrently; its comparison
    /// function is sound to call on `item` and its elements.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pqueue_insert(pq: *mut GitPqueue, item: *mut c_void) -> c_int {
        barrier(|| {
            // SAFETY: a valid queue, exclusively ours (the caller's
            // contract).
            let pq = unsafe { &mut *pq };
            // SAFETY: the comparison is sound on the elements and does not
            // reach the queue (C's contract for comparison functions).
            status(unsafe { pq.insert(item) })
        })
    }

    /// `git_pqueue_pop`.
    ///
    /// # Safety
    ///
    /// As [`git_pqueue_insert`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pqueue_pop(pq: *mut GitPqueue) -> *mut c_void {
        barrier(|| {
            // SAFETY: a valid queue, exclusively ours.
            let pq = unsafe { &mut *pq };
            // SAFETY: as in `git_pqueue_insert`.
            unsafe { pq.pop() }
        })
    }
}

crate::c_layout_test! {
    headers: ["pqueue.h"],
    types: [
        GitPqueue => "git_pqueue" {},
    ],
    asserts: ["GIT_PQUEUE_FIXED_SIZE == 4"],
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block)]
mod tests {
    use super::*;

    unsafe extern "C" fn cmp_usize(a: *const c_void, b: *const c_void) -> c_int {
        // SAFETY: the tests' elements point to `usize`s.
        let (a, b) = unsafe { (*a.cast::<usize>(), *b.cast::<usize>()) };
        a.cmp(&b) as c_int
    }

    fn val(p: *mut c_void) -> usize {
        // SAFETY: the tests' elements point to live `usize`s.
        unsafe { *p.cast::<usize>() }
    }

    #[test]
    fn heap_order() {
        let _g = crate::util::test_support::init();
        let mut vals: Vec<usize> = (0..200).map(|i| (i * 7919) % 211).collect();
        let mut pq = GitPqueue::new(0, 0, Some(cmp_usize)).unwrap();
        for v in vals.iter_mut() {
            // SAFETY: the comparison reads the `usize`s, which outlive `pq`.
            unsafe { pq.insert((v as *mut usize).cast()).unwrap() };
        }
        let mut out = vec![];
        loop {
            // SAFETY: as above.
            let p = unsafe { pq.pop() };
            if p.is_null() {
                break;
            }
            out.push(val(p));
        }
        let mut sorted: Vec<usize> = (0..200).map(|i| (i * 7919) % 211).collect();
        sorted.sort();
        assert_eq!(out, sorted);
    }

    #[test]
    fn fixed_size_keeps_highest() {
        let _g = crate::util::test_support::init();
        let mut vals: Vec<usize> = vec![5, 1, 9, 3, 7, 2, 8];
        let mut pq = GitPqueue::new(GIT_PQUEUE_FIXED_SIZE, 3, Some(cmp_usize)).unwrap();
        assert_eq!(pq.as_vector().capacity(), 3);
        for v in vals.iter_mut() {
            // SAFETY: as in `heap_order`.
            unsafe { pq.insert((v as *mut usize).cast()).unwrap() };
        }
        assert_eq!(pq.len(), 3);
        // SAFETY: as above.
        let got: Vec<usize> = (0..3).map(|_| val(unsafe { pq.pop() })).collect();
        assert_eq!(got, [7, 8, 9]);
    }

    #[test]
    fn without_cmp_is_a_stack() {
        let _g = crate::util::test_support::init();
        let mut vals = [1usize, 2, 3];
        let mut pq = GitPqueue::new(0, 0, None).unwrap();
        for v in vals.iter_mut() {
            pq.push_unordered((v as *mut usize).cast()).unwrap();
        }
        assert_eq!(val(pq.pop_unordered()), 3);
        assert_eq!(val(pq.pop_unordered()), 2);
        assert_eq!(val(pq.pop_unordered()), 1);
        assert!(pq.pop_unordered().is_null());
        // a fixed-size queue without comparison drops items once full
        let mut pq = GitPqueue::new(GIT_PQUEUE_FIXED_SIZE, 2, None).unwrap();
        for v in vals.iter_mut() {
            pq.push_unordered((v as *mut usize).cast()).unwrap();
        }
        assert_eq!(pq.len(), 2);
    }

    #[test]
    fn closure_order() {
        let _g = crate::util::test_support::init();
        let mut vals: Vec<usize> = (0..50).rev().collect();
        let mut pq = GitPqueue::default();
        let mut cmp = |a: *mut c_void, b: *mut c_void| (val(b) as c_int) - (val(a) as c_int);
        for v in vals.iter_mut() {
            pq.insert_by((v as *mut usize).cast(), &mut cmp).unwrap();
        }
        assert_eq!(val(pq.pop_by(&mut cmp)), 49);
        assert_eq!(val(pq.pop_by(&mut cmp)), 48);
    }
}
