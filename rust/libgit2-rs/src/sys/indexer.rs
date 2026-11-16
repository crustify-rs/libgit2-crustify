//! `src/libgit2/indexer.c` (C that stays C): the globals the Rust reads and
//! writes (`GIT_OPT_SET_MAX_OBJECT_SIZE` and friends in `settings.rs`).

use core::sync::atomic::AtomicUsize;
#[cfg(not(miri))]
use core::sync::atomic::Ordering;

/// `UINT31_MAX`, `git_indexer__max_object_size`'s initial value.
pub const DEFAULT_MAX_OBJECT_SIZE: usize = 0x7fff_ffff;

#[cfg_attr(miri, allow(dead_code))]
unsafe extern "C" {
    /// `size_t git_indexer__max_objects`. Declared as an `AtomicUsize`, which
    /// has `size_t`'s layout and lets C write it while Rust reads it.
    pub static git_indexer__max_objects: AtomicUsize;
    /// `size_t git_indexer__max_object_size`, likewise.
    pub static git_indexer__max_object_size: AtomicUsize;
}

/// The current `git_indexer__max_object_size`: the largest object the
/// indexer and delta application accept. Under Miri (no C), the C's
/// initial value.
#[inline]
pub fn max_object_size() -> usize {
    #[cfg(miri)]
    {
        DEFAULT_MAX_OBJECT_SIZE
    }
    #[cfg(not(miri))]
    {
        // SAFETY: the C global is a `size_t`, always initialized, with the
        // layout of `AtomicUsize`; C's plain accesses and Rust's atomic ones
        // never race in a way C's own accesses would not.
        unsafe { git_indexer__max_object_size.load(Ordering::Relaxed) }
    }
}
