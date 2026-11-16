//! Port of `src/util/tsort.c`: libgit2's timsort, which `git_vector_sort`
//! uses. Stable. The algorithm, its comparisons and its merge buffer
//! (allocated with `git__allocator`; on allocation failure a merge is
//! skipped and the array left unsorted, with the out-of-memory error set,
//! as in C) are the C's. The comparator must not access the array being
//! sorted (the Rust holds a reference to it across the calls; C would
//! allow reads).
//!
//! ```
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut v = [5, 3, 9, 1];
//! libgit2_rs::util::tsort::tsort(&mut v, |a, b| a - b);
//! assert_eq!(v, [1, 3, 5, 9]);
//! ```

use core::ffi::{c_int, c_void};

use super::alloc::{GIT_ALLOC, GitAlloc};
use super::errors;

/// Binary search for where `x` goes among the sorted `dst[..size]` (C's
/// `binsearch`; `size >= 1`).
fn binsearch<T: Copy>(dst: &[T], x: T, size: usize, cmp: &mut impl FnMut(T, T) -> c_int) -> usize {
    let mut l: usize = 0;
    let mut r: usize = size - 1;
    let mut c: usize = r >> 1;
    let lx = dst[l];

    if cmp(x, lx) < 0 {
        return 0;
    } else if cmp(x, lx) == 0 {
        let mut i = 1;
        // Bounded by `size` (the C reads on; see rust/DEFECTS.md).
        while i < size && cmp(x, dst[i]) == 0 {
            i += 1;
        }
        return i;
    }

    let mut cx = dst[c];
    loop {
        let val = cmp(x, cx);
        if val < 0 {
            if c - l <= 1 {
                return c;
            }
            r = c;
        } else if val > 0 {
            if r - c <= 1 {
                return c + 1;
            }
            l = c;
        } else {
            loop {
                c += 1;
                if c >= size {
                    return size;
                }
                cx = dst[c];
                if cmp(x, cx) != 0 {
                    return c;
                }
            }
        }
        c = l + ((r - l) >> 1);
        cx = dst[c];
    }
}

/// Binary insertion sort of `dst`, whose first `start` entries are sorted.
fn bisort<T: Copy>(dst: &mut [T], start: usize, cmp: &mut impl FnMut(T, T) -> c_int) {
    for i in start.max(1)..dst.len() {
        if cmp(dst[i - 1], dst[i]) <= 0 {
            continue;
        }
        let x = dst[i];
        let location = binsearch(dst, x, i, cmp);
        dst.copy_within(location..i, location + 1);
        dst[location] = x;
    }
}

#[derive(Clone, Copy, Default)]
struct Run {
    start: usize,
    length: usize,
}

struct Store<T> {
    storage: Vec<T, GitAlloc>,
}

impl<T: Copy> Store<T> {
    /// Make room for `new_size` elements; `false` (with the out-of-memory
    /// error set) on failure.
    fn resize(&mut self, new_size: usize) -> bool {
        if self.storage.capacity() < new_size {
            self.storage.clear();
            if self.storage.try_reserve_exact(new_size).is_err() {
                errors::set_oom();
                return false;
            }
        }
        true
    }
}

fn count_run<T: Copy>(dst: &mut [T], start: usize, cmp: &mut impl FnMut(T, T) -> c_int) -> usize {
    let size = dst.len();
    let mut curr = start + 2;
    if size - start == 1 {
        return 1;
    }
    if start >= size - 2 {
        if cmp(dst[size - 2], dst[size - 1]) > 0 {
            dst.swap(size - 2, size - 1);
        }
        return 2;
    }
    if cmp(dst[start], dst[start + 1]) <= 0 {
        while curr < size - 1 && cmp(dst[curr - 1], dst[curr]) <= 0 {
            curr += 1;
        }
        curr - start
    } else {
        while curr < size - 1 && cmp(dst[curr - 1], dst[curr]) > 0 {
            curr += 1;
        }
        dst[start..curr].reverse();
        curr - start
    }
}

fn compute_minrun(mut n: usize) -> usize {
    let mut r = 0;
    while n >= 64 {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

fn check_invariant(stack: &[Run], stack_curr: usize) -> bool {
    if stack_curr < 2 {
        true
    } else if stack_curr == 2 {
        stack[0].length > stack[1].length
    } else {
        let a = stack[stack_curr - 3].length;
        let b = stack[stack_curr - 2].length;
        let c = stack[stack_curr - 1].length;
        !(a <= b + c || b <= c)
    }
}

/// Merge the runs `stack[stack_curr - 2]` and `stack[stack_curr - 1]`.
fn merge<T: Copy>(
    dst: &mut [T],
    stack: &[Run],
    stack_curr: usize,
    store: &mut Store<T>,
    cmp: &mut impl FnMut(T, T) -> c_int,
) {
    let a = stack[stack_curr - 2].length;
    let b = stack[stack_curr - 1].length;
    let curr = stack[stack_curr - 2].start;

    if !store.resize(a.min(b)) {
        return;
    }
    let storage = &mut store.storage;
    storage.clear();

    if a < b {
        // Left merge.
        storage.extend_from_slice(&dst[curr..curr + a]);
        let mut i = 0;
        let mut j = curr + a;
        for k in curr..curr + a + b {
            if i < a && j < curr + a + b {
                if cmp(storage[i], dst[j]) <= 0 {
                    dst[k] = storage[i];
                    i += 1;
                } else {
                    dst[k] = dst[j];
                    j += 1;
                }
            } else if i < a {
                dst[k] = storage[i];
                i += 1;
            } else {
                dst[k] = dst[j];
                j += 1;
            }
        }
    } else {
        // Right merge; `i` and `j` count down past their starts, as the
        // C's signed indices do: `i == 0` / `j == curr` mean exhausted.
        storage.extend_from_slice(&dst[curr + a..curr + a + b]);
        let mut i = b; // one past the next element of `storage`
        let mut j = curr + a; // one past the next element of the left run
        let mut k = curr + a + b;
        while k > curr {
            k -= 1;
            if i > 0 && j > curr {
                if cmp(dst[j - 1], storage[i - 1]) > 0 {
                    dst[k] = dst[j - 1];
                    j -= 1;
                } else {
                    dst[k] = storage[i - 1];
                    i -= 1;
                }
            } else if i > 0 {
                dst[k] = storage[i - 1];
                i -= 1;
            } else {
                dst[k] = dst[j - 1];
                j -= 1;
            }
        }
    }
}

fn collapse<T: Copy>(
    dst: &mut [T],
    stack: &mut [Run],
    mut stack_curr: usize,
    store: &mut Store<T>,
    size: usize,
    cmp: &mut impl FnMut(T, T) -> c_int,
) -> usize {
    loop {
        if stack_curr <= 1 {
            break;
        }
        // The last merge, or a stack of two that breaks the invariant (two
        // branches with the same body in C).
        if stack_curr == 2
            && (stack[0].length + stack[1].length == size || stack[0].length <= stack[1].length)
        {
            merge(dst, stack, stack_curr, store, cmp);
            stack[0].length += stack[1].length;
            stack_curr -= 1;
            break;
        } else if stack_curr == 2 {
            break;
        }

        let a = stack[stack_curr - 3].length;
        let b = stack[stack_curr - 2].length;
        let c = stack[stack_curr - 1].length;

        if a <= b + c {
            if a < c {
                merge(dst, stack, stack_curr - 1, store, cmp);
                stack[stack_curr - 3].length += stack[stack_curr - 2].length;
                stack[stack_curr - 2] = stack[stack_curr - 1];
                stack_curr -= 1;
            } else {
                merge(dst, stack, stack_curr, store, cmp);
                stack[stack_curr - 2].length += stack[stack_curr - 1].length;
                stack_curr -= 1;
            }
        } else if b <= c {
            merge(dst, stack, stack_curr, store, cmp);
            stack[stack_curr - 2].length += stack[stack_curr - 1].length;
            stack_curr -= 1;
        } else {
            break;
        }
    }
    stack_curr
}

/// The state of one sort: C's locals of `git__tsort_r`.
struct Sort<'d, T, F> {
    dst: &'d mut [T],
    cmp: F,
    store: Store<T>,
    run_stack: [Run; 128],
    stack_curr: usize,
    curr: usize,
    minrun: usize,
}

impl<T: Copy, F: FnMut(T, T) -> c_int> Sort<'_, T, F> {
    /// C's `PUSH_NEXT`: push the next run; `true` once the array is sorted.
    fn push_next(&mut self) -> bool {
        let size = self.dst.len();
        let curr = self.curr;
        let mut len = count_run(self.dst, curr, &mut self.cmp);
        let run = self.minrun.min(size - curr);
        if run > len {
            bisort(&mut self.dst[curr..curr + run], len, &mut self.cmp);
            len = run;
        }
        self.run_stack[self.stack_curr] = Run {
            start: curr,
            length: len,
        };
        self.stack_curr += 1;
        self.curr += len;
        if self.curr == size {
            while self.stack_curr > 1 {
                merge(
                    self.dst,
                    &self.run_stack,
                    self.stack_curr,
                    &mut self.store,
                    &mut self.cmp,
                );
                self.run_stack[self.stack_curr - 2].length +=
                    self.run_stack[self.stack_curr - 1].length;
                self.stack_curr -= 1;
            }
            return true;
        }
        false
    }
}

/// Sort `dst` with `cmp` (negative, 0, positive for less, equal, greater),
/// stably: `git__tsort_r`'s algorithm, for any `Copy` element.
pub fn tsort<T: Copy>(dst: &mut [T], mut cmp: impl FnMut(T, T) -> c_int) {
    let size = dst.len();
    if size < 64 {
        bisort(dst, 1, &mut cmp);
        return;
    }
    let mut s = Sort {
        dst,
        cmp,
        store: Store {
            storage: Vec::new_in(GIT_ALLOC),
        },
        run_stack: [Run::default(); 128],
        stack_curr: 0,
        curr: 0,
        minrun: compute_minrun(size),
    };
    for _ in 0..3 {
        if s.push_next() {
            return;
        }
    }
    loop {
        if !check_invariant(&s.run_stack, s.stack_curr) {
            s.stack_curr = collapse(
                s.dst,
                &mut s.run_stack,
                s.stack_curr,
                &mut s.store,
                size,
                &mut s.cmp,
            );
            continue;
        }
        if s.push_next() {
            return;
        }
    }
}

/// `git__tsort_r`'s comparator type.
pub type SortRCmp = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void) -> c_int;
/// `git__tsort`'s comparator type.
pub type SortCmp = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

/// The exports of `tsort.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// # Safety
    ///
    /// `dst` holds `size` pointers (or `size` is 0) that nothing else
    /// accesses during the call.
    unsafe fn slice<'a>(dst: *mut *mut c_void, size: usize) -> &'a mut [*mut c_void] {
        if size == 0 || dst.is_null() {
            return &mut [];
        }
        // SAFETY: the caller passes `size` pointers, not accessed elsewhere.
        unsafe { core::slice::from_raw_parts_mut(dst, size) }
    }

    /// `git__tsort_r`.
    ///
    /// # Safety
    ///
    /// `dst` holds `size` pointers that nothing else (the comparator
    /// included) accesses during the call; `cmp` is sound to call with any
    /// two of them and `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__tsort_r(
        dst: *mut *mut c_void,
        size: usize,
        cmp: SortRCmp,
        payload: *mut c_void,
    ) {
        barrier(|| {
            // SAFETY: forwarded to the caller's contract.
            let v = unsafe { slice(dst, size) };
            // SAFETY: the caller vouches for `cmp` on the elements.
            tsort(v, |a, b| unsafe { cmp(a, b, payload) })
        })
    }

    /// `git__tsort`.
    ///
    /// # Safety
    ///
    /// As `git__tsort_r`, without the payload.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__tsort(dst: *mut *mut c_void, size: usize, cmp: SortCmp) {
        barrier(|| {
            // SAFETY: forwarded to the caller's contract.
            let v = unsafe { slice(dst, size) };
            // SAFETY: the caller vouches for `cmp` on the elements.
            tsort(v, |a, b| unsafe { cmp(a, b) })
        })
    }
}
