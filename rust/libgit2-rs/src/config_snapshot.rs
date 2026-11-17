//! Port of `src/libgit2/config_snapshot.c`: a read-only copy of another
//! backend's entries (`git_config_backend_snapshot`), taken when the
//! snapshot is opened.
//!
//! The backend is a `CTable` over [`SnapshotBackend`]; the source is any
//! backend (C's or Rust's), read through its `iterator` slot.

use core::ffi::{c_char, c_int};
use core::ptr::NonNull;
use std::sync::{Arc, Mutex};

use ffibox::vtable::CTable;

use crate::config::{
    Backend, BackendCalls, BackendPtr, ConfigBackendCell, GIT_CONFIG_BACKEND_VERSION,
    git_config_backend, git_config_backend_entry, git_config_iterator, slot_busy, slot_panicked,
};
use crate::config_list::ConfigList;
use crate::config_mem::{arg, error_readonly};
use crate::repository::GitRepository as git_repository;
use crate::util::cabi::Out;
use crate::util::errors::{ErrorCode, Result};

/// `config_snapshot_backend`.
#[derive(Debug)]
pub struct SnapshotBackend {
    /// The backend copied at `open` (not owned: C's contract keeps it live
    /// until then).
    source: NonNull<git_config_backend>,
    list: Mutex<Option<Arc<ConfigList>>>,
}

// SAFETY: the fields are `list` (a `Mutex` of a `Send + Sync` list) and
// `source`, a pointer to another backend that is dereferenced only by
// `open`, when `backend_snapshot`'s contract keeps it live; a backend may
// be used from any thread and from several at once (sys/config.h; see
// `ConfigBackendCell`), so `open` may run on any thread.
unsafe impl Send for SnapshotBackend {}
// SAFETY: as for `Send`: through `&SnapshotBackend` only `open` reaches
// `source`, which C calls on any thread, and `list` is behind its lock.
unsafe impl Sync for SnapshotBackend {}

impl SnapshotBackend {
    fn list(&self) -> Option<Arc<ConfigList>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `config_snapshot_open`: copy the source's entries.
    fn open(&self) -> Result<()> {
        let mut list = ConfigList::new();
        // SAFETY: `source` is a live table until the snapshot is opened
        // (`backend_snapshot`'s contract, the only constructor), and `src`
        // is used only during this `open`.
        let src = unsafe { BackendPtr::new(self.source) };
        let mut it = src.iterator()?;
        loop {
            match it.next_entry() {
                Ok(be) => {
                    // SAFETY: `be` is the entry the source's iterator just
                    // returned: a live `git_config_entry` (its first field)
                    // whose strings are NULL or NUL-terminated, unwritten
                    // until the iterator's next call, after `push_c` copies
                    // them.
                    unsafe { list.push_c(be.as_ptr().cast()) }?;
                }
                Err(e) if e == ErrorCode::ITER_OVER => break,
                Err(e) => return Err(e),
            }
        }
        drop(it);
        *self.list.lock().unwrap_or_else(|e| e.into_inner()) = Some(list.freeze());
        Ok(())
    }

    /// `config_snapshot_get`.
    fn get(&self, key: &[u8], mut out: Out<'_, *mut git_config_backend_entry>) -> c_int {
        let g = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let Some(list) = g.as_ref() else {
            return ErrorCode::NOT_FOUND.raw();
        };
        match list.get(key) {
            None => ErrorCode::NOT_FOUND.raw(),
            Some(e) => {
                out.write(ConfigList::lend(list, e));
                0
            }
        }
    }

    /// `config_snapshot_iterator`.
    fn iterator(&self, mut out: Out<'_, *mut git_config_iterator>) -> c_int {
        let list = self.list().unwrap_or_else(|| ConfigList::new().freeze());
        out.write(crate::config_list::iterator_new(list));
        0
    }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot its backend in the marked position,
    // and calls only the slots set here.
    unsafe impl CVtable<SnapshotBackend> for ConfigBackendCell {}
    base = git_config_backend {
        readonly: 1,
        snapshot: Some(ffi::git_config_backend_snapshot),
        ..git_config_backend::default()
    };
    /// `config_snapshot_open`.
    fn open(@cfg, level: c_int, repo: *const git_repository) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &SnapshotBackend| {
            let _ = (level, repo);
            match b.open() {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        },
    }
    /// `config_snapshot_get`.
    fn get(@cfg, key: *const c_char, out: *mut *mut git_config_backend_entry) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `key` is a NUL-terminated string and `out` aligned and
        // writable for the call (the slot's contract, sys/config.h);
        // nothing references `*out`.
        |b: &SnapshotBackend| b.get(unsafe { arg(key) }, unsafe { Out::from_ptr(out) }),
    }
    fn set(@cfg, name: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| {
            let _ = (name, value);
            error_readonly()
        },
    }
    fn set_multivar(@cfg, name: *const c_char, regexp: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| {
            let _ = (name, regexp, value);
            error_readonly()
        },
    }
    fn del(@cfg, name: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| {
            let _ = name;
            error_readonly()
        },
    }
    fn del_multivar(@cfg, name: *const c_char, regexp: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| {
            let _ = (name, regexp);
            error_readonly()
        },
    }
    /// `config_snapshot_iterator`.
    fn iterator(out: *mut *mut git_config_iterator, @cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `out` is aligned and writable for the call (the slot's
        // contract), and nothing references `*out`.
        |b: &SnapshotBackend| b.iterator(unsafe { Out::from_ptr(out) }),
    }
    fn lock(@cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| error_readonly(),
    }
    fn unlock(@cfg, success: c_int) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &SnapshotBackend| {
            let _ = success;
            error_readonly()
        },
    }
    release free;
}

/// `git_config_backend_snapshot`: a snapshot of `source`, filled when it
/// is opened.
///
/// Safe code cannot hand it an arbitrary pointer:
///
/// ```compile_fail
/// let _ = libgit2_rs::__test::config_snapshot::backend_snapshot(core::ptr::NonNull::dangling());
/// ```
///
/// # Safety
///
/// `source` is a live backend table (sys/config.h) until the snapshot is
/// opened (its `open` slot reads `source` through its `iterator` slot), or
/// the snapshot is never opened.
pub unsafe fn backend_snapshot(source: NonNull<git_config_backend>) -> Backend {
    let state = SnapshotBackend {
        source,
        list: Mutex::new(None),
    };
    let p = CTable::<ConfigBackendCell, SnapshotBackend>::new(state).into_c();
    // SAFETY: `p` is the table `into_c` surrendered just now, owned by
    // nothing else, and released through its `free` slot
    // (`CTable::release`), which `Backend`'s policy calls.
    let b = unsafe { Backend::from_c(p.as_ptr().cast()) }.unwrap_or_else(|| unreachable!());
    debug_assert_eq!(
        // SAFETY: `b` owns the live, aligned table just made; the field is
        // copied through the raw pointer.
        unsafe { (*b.raw()).version },
        GIT_CONFIG_BACKEND_VERSION
    );
    b
}

/// The exports of `config_snapshot.c`.
pub mod ffi {
    use super::*;

    use crate::util::export::barrier;

    /// `git_config_backend_snapshot` (also the `snapshot` slot of every
    /// backend of this crate).
    ///
    /// # Safety
    ///
    /// `out` is writable; `source` is a live backend until the snapshot is
    /// opened.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_snapshot(
        out: *mut *mut git_config_backend,
        source: *mut git_config_backend,
    ) -> c_int {
        barrier(|| {
            let Some(source) = NonNull::new(source) else {
                return crate::util::errors::assert_arg_failed("source").raw();
            };
            // SAFETY: `source` is a live backend until the snapshot is
            // opened (this export's contract).
            let b = unsafe { backend_snapshot(source) };
            // SAFETY: `out` is aligned and writable for the call, and no
            // reference covers it (this export's contract; as a `snapshot`
            // slot its caller passes a local). It stays a raw pointer: the
            // function is also the C slot, whose type is C's.
            unsafe { out.write(b.into_c().cast()) };
            0
        })
    }
}
