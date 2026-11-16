//! `src/libgit2/indexer.c` (C that stays C): the limits the options set.

use core::sync::atomic::AtomicUsize;

unsafe extern "C" {
    /// `git_indexer__max_objects`.
    pub static git_indexer__max_objects: AtomicUsize;
    /// `git_indexer__max_object_size`.
    pub static git_indexer__max_object_size: AtomicUsize;
}
