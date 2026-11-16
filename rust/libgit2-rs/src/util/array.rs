//! A Rust equivalent of `src/util/array.h`: `git_array_t(T)`, the
//! growable array of plain values that C embeds in structs.
//!
//! [`GitArray<T>`] has the layout of `struct { T *ptr; size_t size, asize;
//! }`, allocates with `git__allocator`, and grows as `git_array_alloc`
//! (8, then ×1.5). On allocation failure, as in C, the array is freed and
//! emptied, and the out-of-memory error is set.
//!
//! Elements are `Copy` (plain data, as C stores). Dropping the array frees
//! it; note that C's `git_array_dispose` frees without resetting the
//! fields, so an array that C disposes must not be dropped by Rust (use
//! [`GitArray::dispose`] from Rust, which resets them).
//!
//! ```
//! use libgit2_rs::util::array::GitArray;
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut a: GitArray<u32> = GitArray::new();
//! *a.alloc()? = 7;
//! a.push(9)?;
//! assert_eq!(a.as_slice(), [7, 9]);
//! assert_eq!(a.capacity(), 8);
//! # Ok::<(), libgit2_rs::util::errors::ErrorCode>(())
//! ```

use core::ffi::c_int;

use super::alloc;
use super::errors::{self, ErrorCode, Result};

/// `git_array_t(T)`.
#[repr(C)]
pub struct GitArray<T: Copy> {
    ptr: *mut T,
    size: usize,
    asize: usize,
}

// SAFETY: the array owns its buffer of plain values; `T: Send` makes
// moving them to another thread sound, and `git__allocator` frees from any
// thread.
unsafe impl<T: Copy + Send> Send for GitArray<T> {}
// SAFETY: shared access only reads the values.
unsafe impl<T: Copy + Sync> Sync for GitArray<T> {}

impl<T: Copy> Default for GitArray<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> Drop for GitArray<T> {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl<T: Copy + core::fmt::Debug> core::fmt::Debug for GitArray<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T: Copy> GitArray<T> {
    const ELEM: () = assert!(
        size_of::<T>() > 0 && align_of::<T>() <= 8,
        "git_array of a ZST or over-aligned type"
    );

    /// An empty array (`GIT_ARRAY_INIT`).
    pub const fn new() -> GitArray<T> {
        GitArray {
            ptr: core::ptr::null_mut(),
            size: 0,
            asize: 0,
        }
    }

    /// `git_array_init_to_size`: an empty array with zeroed room for
    /// `desired` elements.
    pub fn with_capacity(desired: usize) -> Result<GitArray<T>> {
        let () = Self::ELEM;
        let p = alloc::calloc(desired, size_of::<T>()).cast::<T>();
        if p.is_null() && desired > 0 {
            return Err(ErrorCode::GENERIC);
        }
        Ok(GitArray {
            ptr: p,
            size: 0,
            asize: if p.is_null() { 0 } else { desired },
        })
    }

    /// `git_array_size`.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Whether it has no elements.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `asize`.
    pub fn capacity(&self) -> usize {
        self.asize
    }

    /// The elements.
    pub fn as_slice(&self) -> &[T] {
        if self.ptr.is_null() || self.size == 0 {
            return &[];
        }
        // SAFETY: the array's own buffer holds `size <= asize` initialized
        // elements (every slot below `size` was written by `push`/`alloc`).
        unsafe { core::slice::from_raw_parts(self.ptr, self.size) }
    }

    /// The elements, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if self.ptr.is_null() || self.size == 0 {
            return &mut [];
        }
        // SAFETY: as in `as_slice`; `&mut self` makes it the only access.
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    /// `git_array_get`.
    pub fn get(&self, i: usize) -> Option<&T> {
        self.as_slice().get(i)
    }

    /// `git_array_last`.
    pub fn last(&self) -> Option<&T> {
        self.as_slice().last()
    }

    /// `git_array_pop`: remove the last element and return it.
    pub fn pop(&mut self) -> Option<T> {
        let v = *self.as_slice().last()?;
        self.size -= 1;
        Some(v)
    }

    /// The elements, in order.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// Grow as `git_array__alloc` when full; on failure free the array.
    fn grow(&mut self) -> Result<()> {
        let () = Self::ELEM;
        if self.size < self.asize {
            return Ok(());
        }
        let new_size = if self.size < 8 {
            8
        } else {
            match self.asize.checked_mul(3) {
                Some(n) => n / 2,
                None => {
                    errors::set_oom();
                    self.dispose();
                    return Err(ErrorCode::GENERIC);
                }
            }
        };
        // SAFETY: `ptr` is NULL or the array's own buffer from
        // `git__allocator`.
        let p =
            unsafe { alloc::reallocarray(self.ptr.cast(), new_size, size_of::<T>()) }.cast::<T>();
        if p.is_null() {
            self.dispose();
            return Err(ErrorCode::GENERIC);
        }
        self.ptr = p;
        self.asize = new_size;
        Ok(())
    }

    /// `git_array_alloc`: a new last element, set to `T::default()` (C
    /// leaves it uninitialized).
    pub fn alloc(&mut self) -> Result<&mut T>
    where
        T: Default,
    {
        self.push(T::default())
    }

    /// Append `value`; a reference to it.
    pub fn push(&mut self, value: T) -> Result<&mut T> {
        self.grow()?;
        let i = self.size;
        // SAFETY: after `grow`, `i < asize`, a slot of the array's buffer.
        unsafe { self.ptr.wrapping_add(i).write(value) };
        self.size = i + 1;
        Ok(&mut self.as_mut_slice()[i])
    }

    /// `git_array_clear`: free and empty.
    pub fn clear(&mut self) {
        self.dispose();
    }

    /// `git_array_dispose`, resetting the fields (unlike C's).
    pub fn dispose(&mut self) {
        // SAFETY: the array's own buffer (or NULL), freed once.
        unsafe { alloc::free(self.ptr.cast()) };
        self.ptr = core::ptr::null_mut();
        self.size = 0;
        self.asize = 0;
    }

    /// The buffer (`ptr`; NULL when none was allocated).
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    /// Remove the element at `i`, moving the rest down; `None` if out of
    /// range.
    pub fn remove(&mut self, i: usize) -> Option<T> {
        let v = *self.as_slice().get(i)?;
        let n = self.size;
        self.as_mut_slice().copy_within(i + 1..n, i);
        self.size = n - 1;
        Some(v)
    }

    /// Give up the buffer: `(ptr, size, asize)`, which the caller frees
    /// with `git__free`.
    pub fn into_raw_parts(self) -> (*mut T, usize, usize) {
        let me = core::mem::ManuallyDrop::new(self);
        (me.ptr, me.size, me.asize)
    }

    /// An array over a buffer.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL (with `size` and `asize` 0) or an allocation of
    /// `git__allocator` with room for `asize` elements, of which the first
    /// `size` are initialized, owned by the new array.
    pub unsafe fn from_raw_parts(ptr: *mut T, size: usize, asize: usize) -> GitArray<T> {
        GitArray { ptr, size, asize }
    }

    /// `git_array_search`: binary search with `cmp(element)` giving the
    /// key's order against it; `Ok(index)` or `Err(insertion index)`
    /// (`GIT_ENOTFOUND` in C).
    pub fn search(&self, cmp: impl FnMut(&T) -> c_int) -> core::result::Result<usize, usize> {
        super::util::bsearch(self.as_slice(), cmp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_like_c() {
        let _g = crate::util::test_support::init();
        let mut a: GitArray<u64> = GitArray::new();
        let mut caps = vec![];
        for i in 0..100u64 {
            a.push(i).unwrap();
            caps.push(a.capacity());
        }
        caps.dedup();
        assert_eq!(caps, [8, 12, 18, 27, 40, 60, 90, 135]);
        assert_eq!(a.iter().sum::<u64>(), 4950);
        assert_eq!(a.pop(), Some(99));
        assert_eq!(a.search(|e| 50i32 - *e as i32), Ok(50));
        let b: GitArray<u32> = GitArray::with_capacity(4).unwrap();
        assert_eq!(b.capacity(), 4);
    }
}

crate::c_layout_test! {
    headers: ["array.h"],
    types: [
        GitArray<u64> => "git_array_t(uint64_t)" { ptr, size, asize },
    ],
}
