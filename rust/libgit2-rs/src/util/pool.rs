//! Port of `src/util/pool.c`: `git_pool`, the chunked allocator.
//!
//! [`GitPool`] is the mirrored `git_pool` (kind 3: C embeds it by value, as
//! in `git_index`'s `tree_pool`, and reads `item_size` through
//! `git_pool_is_initialized`). The configuration is without
//! `GIT_DEBUG_POOL`. Pages come from `git__allocator`, with C's layout (a
//! 24-byte header, then the data, 8-aligned) and paging: a page of the
//! system page size less malloc's overhead (4096 - 40 = 4056 bytes on
//! x86-64), or of the request when it is larger, put at the front; blocks
//! come from the front page's unused tail, sizes rounded to 8 bytes.
//!
//! Memory from a pool stays valid until the pool is cleared or dropped;
//! the methods return raw pointers, as C's.
//!
//! ```
//! use libgit2_rs::util::pool::GitPool;
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut pool = GitPool::with_item_size(1)?;
//! let s = pool.strndup(b"hello")?;
//! // SAFETY (for the example): the pool keeps the string until it is dropped.
//! assert_eq!(unsafe { core::ffi::CStr::from_ptr(s.as_ptr()) }, c"hello");
//! assert_eq!(pool.open_pages(), 1);
//! # Ok::<(), libgit2_rs::util::errors::ErrorCode>(())
//! ```

use core::ffi::{CStr, c_char, c_void};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};

use super::alloc;
use super::errors::{self, ErrorCode, Result};
use crate::git_assert_arg;

/// `struct git_pool_page`'s header; the data follows it.
#[repr(C)]
struct PoolPage {
    next: *mut PoolPage,
    size: usize,
    avail: usize,
}

/// `sizeof(git_pool_page)`: the data's offset.
const PAGE_HEADER: usize = size_of::<PoolPage>();

/// `system_page_size`: set by `git_pool_global_init` (0 before).
static SYSTEM_PAGE_SIZE: AtomicUsize = AtomicUsize::new(0);

/// The OS page size (`git__page_size`; 4096 if it cannot be determined).
fn os_page_size() -> usize {
    #[cfg(miri)]
    {
        4096
    }
    #[cfg(not(miri))]
    {
        // SAFETY: `sysconf` has no preconditions.
        let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if n < 0 {
            let _ =
                crate::git_error_set!(errors::ErrorClass::OS, "can't determine system page size");
            4096
        } else {
            n as usize
        }
    }
}

/// `git_pool_global_init`: the page size of new pools, the system page
/// size less malloc's overhead and a page header.
pub fn global_init() -> Result<()> {
    let n = os_page_size() - (2 * size_of::<*const c_void>() + PAGE_HEADER);
    SYSTEM_PAGE_SIZE.store(n, Ordering::Relaxed);
    Ok(())
}

/// The page size of new pools; initialized on first use if
/// [`global_init`] has not run.
fn system_page_size() -> usize {
    let n = SYSTEM_PAGE_SIZE.load(Ordering::Relaxed);
    if n != 0 {
        return n;
    }
    let _ = global_init();
    SYSTEM_PAGE_SIZE.load(Ordering::Relaxed)
}

/// `git_pool`, mirrored.
///
/// Invariant: `pages` is NULL or the first of a list of pages this pool
/// owns, each from `git__allocator`, of `PAGE_HEADER + size` bytes with
/// `avail <= size`.
#[repr(C)]
pub struct GitPool {
    pages: *mut PoolPage,
    item_size: usize,
    page_size: usize,
}

// SAFETY: the pool owns its pages, which `git__allocator` frees from any
// thread; it never reads the blocks it hands out.
unsafe impl Send for GitPool {}
// SAFETY: shared access only reads the page list.
unsafe impl Sync for GitPool {}

impl Default for GitPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GitPool {
    fn drop(&mut self) {
        self.clear();
    }
}

impl core::fmt::Debug for GitPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitPool")
            .field("item_size", &self.item_size)
            .field("page_size", &self.page_size)
            .field("open_pages", &self.open_pages())
            .finish()
    }
}

/// A pool page's data.
#[inline(always)]
fn page_data(page: *mut PoolPage) -> *mut u8 {
    page.cast::<u8>().wrapping_add(PAGE_HEADER)
}

impl GitPool {
    /// An uninitialized pool (`GIT_POOL_INIT`): allocating from it fails
    /// until [`init`](GitPool::init).
    pub const fn new() -> GitPool {
        GitPool {
            pages: ptr::null_mut(),
            item_size: 0,
            page_size: 0,
        }
    }

    /// `git_pool_init` of a new pool.
    pub fn with_item_size(item_size: usize) -> Result<GitPool> {
        let mut p = GitPool::new();
        p.init(item_size)?;
        Ok(p)
    }

    /// `git_pool_init`: an empty pool of `item_size`-byte items (1 for
    /// strings). The pool's pages are freed first (C's leaks them).
    pub fn init(&mut self, item_size: usize) -> Result<()> {
        git_assert_arg!(item_size >= 1, "item_size >= 1");
        self.clear();
        self.item_size = item_size;
        self.page_size = system_page_size();
        Ok(())
    }

    /// `git_pool_is_initialized`.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.item_size > 0
    }

    /// The item size.
    #[inline]
    pub fn item_size(&self) -> usize {
        self.item_size
    }

    /// The page size.
    #[inline]
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// `git_pool_clear`: free every page (the memory handed out).
    pub fn clear(&mut self) {
        let mut scan = self.pages;
        while !scan.is_null() {
            // SAFETY: a page of this pool (the invariant), freed once below.
            let next = unsafe { (*scan).next };
            // SAFETY: a page from `git__allocator`, not used again.
            unsafe { alloc::free(scan.cast()) };
            scan = next;
        }
        self.pages = ptr::null_mut();
    }

    /// `git_pool_swap`.
    #[inline]
    pub fn swap(&mut self, other: &mut GitPool) {
        core::mem::swap(self, other);
    }

    /// `alloc_size`: the bytes for `count` items, each rounded up to the
    /// pointer size (for byte items, the total is rounded). `None` on
    /// overflow (C wraps).
    #[inline]
    fn alloc_size(&self, count: usize) -> Option<usize> {
        const ALIGN: usize = size_of::<*const c_void>() - 1;
        if self.item_size > 1 {
            let item_size = self.item_size.checked_add(ALIGN)? & !ALIGN;
            item_size.checked_mul(count)
        } else {
            Some(count.checked_add(ALIGN)? & !ALIGN)
        }
    }

    /// `pool_alloc_page`.
    fn alloc_page(&mut self, size: usize) -> Result<NonNull<u8>> {
        let new_page_size = if size <= self.page_size {
            self.page_size
        } else {
            size
        };
        let Some(alloc_size) = new_page_size.checked_add(PAGE_HEADER) else {
            return Err(errors::oom());
        };
        let page = alloc::malloc(alloc_size).cast::<PoolPage>();
        if page.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: a fresh allocation of at least the header's size, aligned
        // to 8 (`git__allocator`'s guarantee).
        unsafe {
            page.write(PoolPage {
                next: self.pages,
                size: new_page_size,
                avail: new_page_size - size,
            })
        };
        self.pages = page;
        // SAFETY: the data of a non-null allocation.
        Ok(unsafe { NonNull::new_unchecked(page_data(page)) })
    }

    /// `pool_alloc`.
    #[inline]
    fn alloc(&mut self, size: usize) -> Result<NonNull<u8>> {
        let page = self.pages;
        // SAFETY: NULL or the pool's front page.
        let fits = unsafe { page.as_ref() }.is_some_and(|p| p.avail >= size);
        if !fits {
            return self.alloc_page(size);
        }
        // SAFETY: the front page, exclusively ours through `&mut self`.
        let p = unsafe { &mut *page };
        // `size - avail` is within its data (`avail <= size`).
        let ptr = page_data(page).wrapping_add(p.size - p.avail);
        p.avail -= size;
        // SAFETY: within a non-null allocation.
        Ok(unsafe { NonNull::new_unchecked(ptr) })
    }

    /// `git_pool_malloc`: room for `items` items, uninitialized. Fails
    /// (out of memory) on allocation failure or a size overflow.
    #[inline]
    pub fn malloc(&mut self, items: usize) -> Result<NonNull<u8>> {
        let Some(size) = self.alloc_size(items) else {
            return Err(errors::oom());
        };
        self.alloc(size)
    }

    /// `git_pool_mallocz`: as [`malloc`](GitPool::malloc), zeroed (the whole
    /// rounded size).
    pub fn mallocz(&mut self, items: usize) -> Result<NonNull<u8>> {
        let Some(size) = self.alloc_size(items) else {
            return Err(errors::oom());
        };
        let p = self.alloc(size)?;
        // SAFETY: `size` bytes of the pool, just handed out.
        unsafe { ptr::write_bytes(p.as_ptr(), 0, size) };
        Ok(p)
    }

    /// `git_pool_strndup` (of all of `s`): a NUL-terminated copy. Only for
    /// pools of 1-byte items.
    pub fn strndup(&mut self, s: &[u8]) -> Result<NonNull<c_char>> {
        git_assert_arg!(self.item_size == 1, "pool->item_size == sizeof(char)");
        self.strndup_bytes(s)
    }

    /// [`strndup`](GitPool::strndup), the item size already checked.
    #[inline]
    fn strndup_bytes(&mut self, s: &[u8]) -> Result<NonNull<c_char>> {
        let Some(n1) = s.len().checked_add(1) else {
            return Err(errors::oom());
        };
        let p = self.malloc(n1)?;
        let d = p.as_ptr();
        // SAFETY: `n1` bytes of the pool, just handed out (uninitialized,
        // so written through the pointer only), disjoint from `s`.
        unsafe { ptr::copy_nonoverlapping(s.as_ptr(), d, s.len()) };
        // SAFETY: the last of those bytes.
        unsafe { d.wrapping_add(s.len()).write(0) };
        Ok(p.cast())
    }

    /// `git_pool_strdup`.
    pub fn strdup(&mut self, s: &CStr) -> Result<NonNull<c_char>> {
        self.strndup(s.to_bytes())
    }

    /// `git_pool_strcat`: the NUL-terminated concatenation of `a` and `b`.
    pub fn strcat(&mut self, a: &[u8], b: &[u8]) -> Result<NonNull<c_char>> {
        git_assert_arg!(self.item_size == 1, "pool->item_size == sizeof(char)");
        let Some(total) = a.len().checked_add(b.len()).and_then(|t| t.checked_add(1)) else {
            return Err(errors::oom());
        };
        let p = self.malloc(total)?;
        let d = p.as_ptr();
        // SAFETY: `total` bytes of the pool, just handed out (uninitialized,
        // so written through the pointer only), disjoint from `a` and `b`.
        unsafe { ptr::copy_nonoverlapping(a.as_ptr(), d, a.len()) };
        let d = d.wrapping_add(a.len());
        // SAFETY: as above, after `a`'s bytes.
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), d, b.len()) };
        // SAFETY: the last of the `total` bytes.
        unsafe { d.wrapping_add(b.len()).write(0) };
        Ok(p.cast())
    }

    /// `git_pool__open_pages`.
    pub fn open_pages(&self) -> u32 {
        let mut ct = 0u32;
        let mut scan = self.pages;
        while !scan.is_null() {
            ct = ct.wrapping_add(1);
            // SAFETY: a page of this pool.
            scan = unsafe { (*scan).next };
        }
        ct
    }

    /// `git_pool__ptr_in_pool`: whether `p` is within a page's data.
    pub fn ptr_in_pool(&self, p: *const c_void) -> bool {
        let p = p as usize;
        let mut scan = self.pages;
        while !scan.is_null() {
            // SAFETY: a page of this pool.
            let page = unsafe { &*scan };
            let start = page_data(scan) as usize;
            if start <= p && p < start + page.size {
                return true;
            }
            scan = page.next;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `pool.c`.
pub mod ffi {
    use core::ffi::{c_char, c_int, c_void};
    use core::ptr;

    use super::*;
    use crate::util::export::{barrier, cstr_bytes, status};

    /// `git_pool_global_init`.
    ///
    /// # Safety
    ///
    /// None (called by `git_libgit2_init`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_global_init() -> c_int {
        barrier(|| status(global_init()))
    }

    /// `git_pool_init`: overwrite `*pool` with an empty pool (whatever it
    /// held, as C's `memset`).
    ///
    /// # Safety
    ///
    /// `pool` is NULL or writable (it need not be initialized).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_init(pool: *mut GitPool, item_size: usize) -> c_int {
        barrier(|| {
            git_assert_arg!(!pool.is_null(), "pool", -1);
            git_assert_arg!(item_size >= 1, "item_size >= 1", -1);
            let p = GitPool {
                pages: ptr::null_mut(),
                item_size,
                page_size: system_page_size(),
            };
            // SAFETY: writable; the old contents are not read or dropped.
            unsafe { pool.write(p) };
            0
        })
    }

    /// `git_pool_clear`.
    ///
    /// # Safety
    ///
    /// `pool` is a valid pool, not used concurrently.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_clear(pool: *mut GitPool) {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { (*pool).clear() })
    }

    /// `git_pool_swap`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are valid pools (possibly the same), not used
    /// concurrently.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_swap(a: *mut GitPool, b: *mut GitPool) {
        if a != b {
            // SAFETY: two distinct valid pools.
            unsafe { ptr::swap_nonoverlapping(a, b, 1) };
        }
    }

    /// `git_pool_malloc`.
    ///
    /// # Safety
    ///
    /// `pool` is a valid pool, not used concurrently.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_malloc(pool: *mut GitPool, items: usize) -> *mut c_void {
        barrier(|| {
            // SAFETY: the caller's contract.
            let pool = unsafe { &mut *pool };
            pool.malloc(items)
                .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
        })
    }

    /// `git_pool_mallocz`.
    ///
    /// # Safety
    ///
    /// As [`git_pool_malloc`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_mallocz(pool: *mut GitPool, items: usize) -> *mut c_void {
        barrier(|| {
            // SAFETY: the caller's contract.
            let pool = unsafe { &mut *pool };
            pool.mallocz(items)
                .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
        })
    }

    /// `git_pool_strndup`: copy `n` bytes of `str` (C copies `n` bytes
    /// whatever they are; so does this).
    ///
    /// # Safety
    ///
    /// `pool` is NULL or a valid pool, not used concurrently; `str` is NULL
    /// or readable for `n` bytes, and not within the pool's unused memory.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_strndup(
        pool: *mut GitPool,
        str: *const c_char,
        n: usize,
    ) -> *mut c_char {
        barrier(|| {
            git_assert_arg!(!pool.is_null(), "pool", ptr::null_mut());
            git_assert_arg!(!str.is_null(), "str", ptr::null_mut());
            // SAFETY: a valid pool.
            let pool = unsafe { &mut *pool };
            git_assert_arg!(
                pool.item_size == 1,
                "pool->item_size == sizeof(char)",
                ptr::null_mut()
            );
            if n == usize::MAX {
                return ptr::null_mut();
            }
            if n > isize::MAX as usize {
                // C's `git_pool_malloc(n + 1)` fails before reading `str`.
                let _ = errors::oom();
                return ptr::null_mut();
            }
            // SAFETY: `n` readable bytes, which the allocation below does
            // not write (it hands out memory that is not `str`'s).
            let s = unsafe { core::slice::from_raw_parts(str.cast::<u8>(), n) };
            pool.strndup_bytes(s)
                .map_or(ptr::null_mut(), NonNull::as_ptr)
        })
    }

    /// `git_pool_strdup`.
    ///
    /// # Safety
    ///
    /// `pool` as [`git_pool_strndup`]; `str` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_strdup(
        pool: *mut GitPool,
        str: *const c_char,
    ) -> *mut c_char {
        barrier(|| {
            git_assert_arg!(!pool.is_null(), "pool", ptr::null_mut());
            git_assert_arg!(!str.is_null(), "str", ptr::null_mut());
            // SAFETY: a valid pool.
            let pool = unsafe { &mut *pool };
            git_assert_arg!(
                pool.item_size == 1,
                "pool->item_size == sizeof(char)",
                ptr::null_mut()
            );
            // SAFETY: a NUL-terminated string.
            let s = unsafe { cstr_bytes(str) };
            pool.strndup_bytes(s)
                .map_or(ptr::null_mut(), NonNull::as_ptr)
        })
    }

    /// `git_pool_strdup_safe`: NULL for a NULL string.
    ///
    /// # Safety
    ///
    /// As [`git_pool_strdup`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_strdup_safe(
        pool: *mut GitPool,
        str: *const c_char,
    ) -> *mut c_char {
        if str.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: forwarded.
        unsafe { git_pool_strdup(pool, str) }
    }

    /// `git_pool_strcat`: a NULL string is empty.
    ///
    /// # Safety
    ///
    /// `pool` as [`git_pool_strndup`]; `a` and `b` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool_strcat(
        pool: *mut GitPool,
        a: *const c_char,
        b: *const c_char,
    ) -> *mut c_char {
        barrier(|| {
            git_assert_arg!(!pool.is_null(), "pool", ptr::null_mut());
            // SAFETY: a valid pool.
            let pool = unsafe { &mut *pool };
            git_assert_arg!(
                pool.item_size == 1,
                "pool->item_size == sizeof(char)",
                ptr::null_mut()
            );
            // SAFETY: NULL or a NUL-terminated string.
            let a = unsafe { cstr_bytes(a) };
            // SAFETY: as above.
            let b = unsafe { cstr_bytes(b) };
            pool.strcat(a, b).map_or(ptr::null_mut(), NonNull::as_ptr)
        })
    }

    /// `git_pool__open_pages`.
    ///
    /// # Safety
    ///
    /// `pool` is a valid pool.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool__open_pages(pool: *mut GitPool) -> u32 {
        // SAFETY: a valid pool.
        unsafe { (*pool).open_pages() }
    }

    /// `git_pool__ptr_in_pool`.
    ///
    /// # Safety
    ///
    /// `pool` is a valid pool.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pool__ptr_in_pool(pool: *mut GitPool, ptr: *mut c_void) -> bool {
        // SAFETY: a valid pool.
        unsafe { (*pool).ptr_in_pool(ptr) }
    }
}

crate::c_layout_test! {
    headers: ["pool.h"],
    types: [
        GitPool => "git_pool" { pages, item_size, page_size },
    ],
    asserts: ["sizeof(struct { git_pool_page *next; size_t size; size_t avail; GIT_ALIGN(char data[GIT_FLEX_ARRAY], 8); }) == 24"],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_is_c_s() {
        let _g = crate::util::test_support::init();
        let mut p = GitPool::with_item_size(1).unwrap();
        assert_eq!(p.page_size(), 4096 - 40);
        let a = p.malloc(1).unwrap();
        let b = p.malloc(9).unwrap();
        assert_eq!(b.as_ptr() as usize - a.as_ptr() as usize, 8);
        assert_eq!(a.as_ptr() as usize % 8, 0);
        assert_eq!(p.open_pages(), 1);
        assert!(p.ptr_in_pool(b.as_ptr().cast()));
        assert!(!p.ptr_in_pool(core::ptr::null()));
        // a big request gets its own page at the front
        let big = p.malloc(10_000).unwrap();
        assert_eq!(p.open_pages(), 2);
        assert!(p.ptr_in_pool(big.as_ptr().wrapping_add(9_999).cast()));
        // the big page is full: the next block opens a new one
        p.malloc(8).unwrap();
        assert_eq!(p.open_pages(), 3);
        p.clear();
        assert_eq!(p.open_pages(), 0);
    }

    #[test]
    fn items_and_strings() {
        let _g = crate::util::test_support::init();
        let mut p = GitPool::with_item_size(12).unwrap();
        let a = p.mallocz(3).unwrap();
        let b = p.malloc(1).unwrap();
        assert_eq!(b.as_ptr() as usize - a.as_ptr() as usize, 48);
        assert!(p.strndup(b"x").is_err());
        assert!(p.malloc(usize::MAX / 2).is_err());
        let mut s = GitPool::with_item_size(1).unwrap();
        let c = s.strcat(b"ab", b"cd").unwrap();
        // SAFETY: the pool keeps the string.
        assert_eq!(unsafe { CStr::from_ptr(c.as_ptr()) }, c"abcd");
        let d = s.strdup(c"x").unwrap();
        // SAFETY: as above.
        assert_eq!(unsafe { CStr::from_ptr(d.as_ptr()) }, c"x");
        assert!(s.malloc(usize::MAX).is_err());
        p.swap(&mut s);
        assert_eq!(p.item_size(), 1);
        assert!(GitPool::with_item_size(0).is_err());
        assert!(!GitPool::new().is_initialized());
    }

    #[test]
    fn fills_pages() {
        let _g = crate::util::test_support::init();
        let mut p = GitPool::with_item_size(1).unwrap();
        for _ in 0..(4056 / 8) {
            p.malloc(8).unwrap();
        }
        assert_eq!(p.open_pages(), 1);
        p.malloc(1).unwrap();
        assert_eq!(p.open_pages(), 2);
    }
}
