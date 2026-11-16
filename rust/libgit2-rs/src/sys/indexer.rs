//! `src/libgit2/indexer.c` (C that stays C): the globals the Rust reads.

use core::sync::atomic::AtomicUsize;
#[cfg(not(miri))]
use core::sync::atomic::Ordering;

/// `UINT31_MAX`, `git_indexer__max_object_size`'s initial value.
pub const DEFAULT_MAX_OBJECT_SIZE: usize = 0x7fff_ffff;

#[cfg_attr(miri, allow(dead_code))]
unsafe extern "C" {
    /// `size_t git_indexer__max_object_size` (`GIT_OPT_SET_MAX_OBJECT_SIZE`
    /// in `settings.c` writes it). Declared as an `AtomicUsize`, which has
    /// `size_t`'s layout and lets C write it while Rust reads it (C's own
    /// writes and reads are plain, as before).
    static git_indexer__max_object_size: AtomicUsize;
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
        // layout of `AtomicUsize`; reading it atomically races with no
        // Rust write (only C writes it, through `git_libgit2_opts`).
        unsafe { git_indexer__max_object_size.load(Ordering::Relaxed) }
    }
}
