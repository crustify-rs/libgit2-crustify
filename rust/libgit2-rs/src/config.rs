//! Port of `src/libgit2/config.c`: the configuration object
//! (`git_config`), its backends by level, lookups, iteration, writes, and
//! the value parsers (`git_config_parse_*`).
//!
//! # Types (PORTING.md §4.1)
//!
//! - [`GitConfig`] (`git_config`): **shared prefix**. C that stays
//!   (`repository.c`) takes and drops references (`GIT_REFCOUNT_INC`,
//!   `GIT_REFCOUNT_OWN` on `rc`) but never allocates or sizes it, so only
//!   `rc` stays in `config.h`; the backends follow in Rust-native state.
//! - [`git_config_entry`], [`git_config_backend_entry`],
//!   [`git_config_iterator`], [`git_config_backend`], [`git_configmap`],
//!   [`git_config_backend_memory_options`]: **mirrored** (public headers).
//! - `backend_instance`, `backend_entry`, the iterators: opaque (Rust
//!   types; C sees the iterators as `git_config_iterator` tables).
//!
//! # Backends
//!
//! A backend is a C table of function pointers (`git_config_backend`),
//! whoever implements it. The config holds every backend the same way, as
//! a [`Backend`] (a `CBox` whose release calls the table's `free` slot) and
//! calls its slots. The file, memory and snapshot backends of this crate
//! are `CTable`s (ffibox) whose slots C calls in turn; so are the
//! iterators. Rust consumers implement [`ConfigBackend`] instead.
//!
//! # Concurrency
//!
//! A repository's config is read from several threads at once
//! (`tests/libgit2/threads`), so [`GitConfig`] is `Sync`: its list of
//! backends is an `Arc` behind a mutex, which readers clone (and release)
//! before they call any backend, and writers replace. No lock is held
//! while a backend runs, so a backend may re-enter the config.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;
use std::sync::Arc;

use ffibox::vtable::CTable;
use ffibox::{CBox, CRefSeam};

use crate::buf::GitBuf;
use crate::repository::GitRepository as git_repository;
use crate::util::cabi::Out;
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::regexp::Regexp;
use crate::util::str::GitStr;
use crate::util::thread::GitRefcount;
use crate::{git_assert, git_assert_arg, git_error_set};
use ffibox::CShared;

// ---------------------------------------------------------------------------
// The public C types
// ---------------------------------------------------------------------------

/// `GIT_CONFIG_LEVEL_SYSTEM`.
pub const GIT_CONFIG_LEVEL_SYSTEM: c_int = 2;
/// `GIT_CONFIG_LEVEL_XDG`.
pub const GIT_CONFIG_LEVEL_XDG: c_int = 3;
/// `GIT_CONFIG_LEVEL_GLOBAL`.
pub const GIT_CONFIG_LEVEL_GLOBAL: c_int = 4;
/// `GIT_CONFIG_LEVEL_LOCAL`.
pub const GIT_CONFIG_LEVEL_LOCAL: c_int = 5;
/// `GIT_CONFIG_LEVEL_WORKTREE`.
pub const GIT_CONFIG_LEVEL_WORKTREE: c_int = 6;
/// `GIT_CONFIG_LEVEL_APP`.
pub const GIT_CONFIG_LEVEL_APP: c_int = 7;
/// `GIT_CONFIG_HIGHEST_LEVEL`.
pub const GIT_CONFIG_HIGHEST_LEVEL: c_int = -1;

/// `GIT_CONFIG_BACKEND_VERSION`.
pub const GIT_CONFIG_BACKEND_VERSION: c_uint = 1;
/// `GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION`.
pub const GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION: c_uint = 1;

/// `GIT_CONFIGMAP_FALSE`.
pub const GIT_CONFIGMAP_FALSE: c_int = 0;
/// `GIT_CONFIGMAP_TRUE`.
pub const GIT_CONFIGMAP_TRUE: c_int = 1;
/// `GIT_CONFIGMAP_INT32`.
pub const GIT_CONFIGMAP_INT32: c_int = 2;
/// `GIT_CONFIGMAP_STRING`.
pub const GIT_CONFIGMAP_STRING: c_int = 3;

/// `GIT_CONFIG_FILENAME_SYSTEM`.
pub const GIT_CONFIG_FILENAME_SYSTEM: &CStr = c"gitconfig";
/// `GIT_CONFIG_FILENAME_GLOBAL`.
pub const GIT_CONFIG_FILENAME_GLOBAL: &CStr = c".gitconfig";
/// `GIT_CONFIG_FILENAME_XDG`.
pub const GIT_CONFIG_FILENAME_XDG: &CStr = c"config";
/// `GIT_CONFIG_FILE_MODE`.
pub const GIT_CONFIG_FILE_MODE: libc::mode_t = 0o666;

/// `git_config_entry`, mirrored.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct git_config_entry {
    /// `name`.
    pub name: *const c_char,
    /// `value`.
    pub value: *const c_char,
    /// `backend_type`.
    pub backend_type: *const c_char,
    /// `origin_path`.
    pub origin_path: *const c_char,
    /// `include_depth`.
    pub include_depth: c_uint,
    /// `level`.
    pub level: c_int,
}

/// `git_config_backend_entry`, mirrored: an entry and how to free it.
#[repr(C)]
#[derive(Debug)]
pub struct git_config_backend_entry {
    /// `entry`.
    pub entry: git_config_entry,
    /// `free`.
    pub free: Option<unsafe extern "C" fn(*mut git_config_backend_entry)>,
}

/// `git_config_iterator`, mirrored: a C table.
#[repr(C)]
#[derive(Debug)]
pub struct git_config_iterator {
    /// `backend`.
    pub backend: *mut git_config_backend,
    /// `flags`.
    pub flags: c_uint,
    /// `next`.
    pub next: Option<
        unsafe extern "C" fn(*mut *mut git_config_backend_entry, *mut git_config_iterator) -> c_int,
    >,
    /// `free`.
    pub free: Option<unsafe extern "C" fn(*mut git_config_iterator)>,
}

impl Default for git_config_iterator {
    fn default() -> Self {
        git_config_iterator {
            backend: core::ptr::null_mut(),
            flags: 0,
            next: None,
            free: None,
        }
    }
}

/// `git_config_backend`, mirrored: a C table.
#[repr(C)]
#[derive(Debug)]
#[allow(clippy::type_complexity)]
pub struct git_config_backend {
    /// `version`.
    pub version: c_uint,
    /// `readonly`.
    pub readonly: c_int,
    /// `cfg`.
    pub cfg: *mut GitConfig,
    /// `open`.
    pub open: Option<
        unsafe extern "C" fn(*mut git_config_backend, c_int, *const git_repository) -> c_int,
    >,
    /// `get`.
    pub get: Option<
        unsafe extern "C" fn(
            *mut git_config_backend,
            *const c_char,
            *mut *mut git_config_backend_entry,
        ) -> c_int,
    >,
    /// `set`.
    pub set: Option<
        unsafe extern "C" fn(*mut git_config_backend, *const c_char, *const c_char) -> c_int,
    >,
    /// `set_multivar`.
    pub set_multivar: Option<
        unsafe extern "C" fn(
            *mut git_config_backend,
            *const c_char,
            *const c_char,
            *const c_char,
        ) -> c_int,
    >,
    /// `del`.
    pub del: Option<unsafe extern "C" fn(*mut git_config_backend, *const c_char) -> c_int>,
    /// `del_multivar`.
    pub del_multivar: Option<
        unsafe extern "C" fn(*mut git_config_backend, *const c_char, *const c_char) -> c_int,
    >,
    /// `iterator`.
    pub iterator: Option<
        unsafe extern "C" fn(*mut *mut git_config_iterator, *mut git_config_backend) -> c_int,
    >,
    /// `snapshot`.
    pub snapshot: Option<
        unsafe extern "C" fn(*mut *mut git_config_backend, *mut git_config_backend) -> c_int,
    >,
    /// `lock`.
    pub lock: Option<unsafe extern "C" fn(*mut git_config_backend) -> c_int>,
    /// `unlock`.
    pub unlock: Option<unsafe extern "C" fn(*mut git_config_backend, c_int) -> c_int>,
    /// `free`.
    pub free: Option<unsafe extern "C" fn(*mut git_config_backend)>,
}

impl Default for git_config_backend {
    /// `GIT_CONFIG_BACKEND_INIT`.
    fn default() -> Self {
        git_config_backend {
            version: GIT_CONFIG_BACKEND_VERSION,
            readonly: 0,
            cfg: core::ptr::null_mut(),
            open: None,
            get: None,
            set: None,
            set_multivar: None,
            del: None,
            del_multivar: None,
            iterator: None,
            snapshot: None,
            lock: None,
            unlock: None,
            free: None,
        }
    }
}

/// `git_config_backend_memory_options`, mirrored.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct git_config_backend_memory_options {
    /// `version`.
    pub version: c_uint,
    /// `backend_type`.
    pub backend_type: *const c_char,
    /// `origin_path`.
    pub origin_path: *const c_char,
}

/// `git_configmap`, mirrored.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct git_configmap {
    /// `type` (`git_configmap_t`).
    pub type_: c_int,
    /// `str_match`.
    pub str_match: *const c_char,
    /// `map_value`.
    pub map_value: c_int,
}

// The layout types of the two tables, and their handles.
ffibox::define_ctype!(
    ConfigBackendCell,
    ConfigBackendRef,
    ConfigBackendMut,
    git_config_backend
);
ffibox::define_ctype!(
    ConfigIterator,
    ConfigIteratorRef,
    ConfigIteratorMut,
    git_config_iterator
);

// SAFETY: the table's fields are `version` and `readonly` (plain data),
// `cfg` (a pointer the backend only stores and hands back) and the slots
// (function pointers). libgit2's contract for config backends
// (sys/config.h) is that a backend may be used from any thread, and from
// several at once, as a repository's config is (`tests/libgit2/threads`):
// the slots synchronise the state they share. The handles only read the
// fields and call the slots; the one Rust write of a field (`cfg` in
// `add_backend`, `readonly` in `rust_backend::new`) happens before the
// table is shared.
unsafe impl Send for ConfigBackendCell {}
// SAFETY: as for `Send`: every slot may run concurrently with the others.
unsafe impl Sync for ConfigBackendCell {}
// SAFETY: the table's fields are `backend` (a pointer the iterator only
// stores), `flags` and the `next` / `free` slots. An iterator is used by
// one thread at a time (libgit2's contract for non-shared objects) and may
// move between threads: its slots keep no thread-local state.
unsafe impl Send for ConfigIterator {}
// SAFETY: a shared borrow reaches no slot: the slots take `&mut` (the
// `impl_cvtable!` tables are exclusive) and `IterBox` calls them only
// through `&mut self`.
unsafe impl Sync for ConfigIterator {}

/// The `on panic:` result of this crate's table slots: C's generic
/// failure, with the barrier's internal error.
#[cold]
pub(crate) fn slot_panicked() -> c_int {
    errors::set_panic_error();
    -1
}

/// The `on busy:` result of this crate's table slots: a slot refused
/// because the table is being released.
#[cold]
pub(crate) fn iter_busy() -> c_int {
    git_error_set!(
        ErrorClass::CONFIG,
        "the config iterator is already in use or being released"
    )
    .raw()
}

/// The `on busy:` result of the backend tables' slots (all shared): a slot
/// refused because the table is being released.
#[cold]
pub(crate) fn slot_busy() -> c_int {
    git_error_set!(ErrorClass::CONFIG, "the config backend is being released").raw()
}

/// A Rust error raised into C's thread state: its code, with its class and
/// message set.
pub(crate) fn raise(e: &Error) -> ErrorCode {
    e.set_last()
}

// ---------------------------------------------------------------------------
// Backends, entries and iterators held from Rust
// ---------------------------------------------------------------------------

/// Calls a table's `free` slot (the release of [`Backend`]).
///
/// # Safety
///
/// `b` is a live backend table that nothing uses any more, released once.
unsafe extern "C" fn backend_free(b: *mut git_config_backend) {
    // SAFETY: `b` is a live, aligned table (this function's contract); the
    // field is copied out through the raw pointer, no reference formed.
    if let Some(free) = unsafe { (*b).free } {
        // SAFETY: the table's own `free`, called once with its table, which
        // nothing uses afterwards (this function's contract): the slot's
        // contract (sys/config.h).
        unsafe { free(b) };
    }
}

/// The release policy of [`Backend`]: the table's `free` slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackendFree;
ffibox::impl_cdrop!(BackendFree, ConfigBackendCell, backend_free);

/// A config backend, whoever implements it: an owner of the table, which
/// it releases through the table's `free` slot.
pub type Backend = CBox<ConfigBackendCell, BackendFree>;

/// Read slot `$f` of the table of `$h`, a [`BackendCalls`] handle (raw, as
/// C may write the table). Only a handle's table is read: the sealed trait's
/// `raw` is live while the handle is borrowed.
macro_rules! slot {
    ($h:expr, $f:ident) => {{
        let p: *mut git_config_backend = BackendCalls::raw($h);
        // SAFETY: `p` is the live table of a `BackendCalls` handle (the
        // trait's invariant); the field is read through a raw pointer.
        unsafe { (&raw const (*p).$f).read() }
    }};
}

/// The error for a slot a backend left NULL (C would call NULL).
#[cold]
fn no_slot(name: &str) -> ErrorCode {
    git_error_set!(
        ErrorClass::CONFIG,
        "the config backend does not implement '{name}'"
    )
}

/// A C string argument, or NULL.
fn opt_ptr(s: Option<&CStr>) -> *const c_char {
    s.map_or(core::ptr::null(), CStr::as_ptr)
}

mod sealed {
    /// Only this crate's handles of a live table implement
    /// [`BackendCalls`](super::BackendCalls): [`Backend`](super::Backend)
    /// (an owner) and [`BackendPtr`](super::BackendPtr) (whose `unsafe`
    /// constructor carries the liveness).
    pub trait Sealed {}
}

/// The slots of a backend, called as `config.c` calls them.
///
/// Every method keeps the slot's contract (sys/config.h): the table is
/// live (the handle borrows its owner), and each argument is what the slot
/// expects. The trait is sealed: `raw` of every implementation returns a
/// live table for as long as `&self` lives.
///
/// ```compile_fail
/// use libgit2_rs::__test::config::{BackendCalls, git_config_backend};
/// struct Bogus;
/// impl BackendCalls for Bogus {
///     fn raw(&self) -> *mut git_config_backend { core::ptr::dangling_mut() }
/// }
/// ```
pub trait BackendCalls: sealed::Sealed {
    /// The table.
    fn raw(&self) -> *mut git_config_backend;

    /// `readonly`, as C may have set it.
    fn is_readonly(&self) -> bool {
        let p = self.raw();
        // SAFETY: `p` is the live, aligned table of this handle (the sealed
        // trait's invariant); the field is copied through the raw pointer.
        unsafe { (*p).readonly != 0 }
    }

    /// `open`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository that outlives the backend (the
    /// file backend keeps it, for conditional includes).
    unsafe fn open(&self, level: c_int, repo: *const git_repository) -> Result<()> {
        let p = self.raw();
        let f = slot!(self, open).ok_or_else(|| no_slot("open"))?;
        // SAFETY: `f` is the `open` slot of `p`, called with its live table
        // (the trait's invariant); `repo` is NULL or a repository that
        // outlives the backend (this method's contract).
        ErrorCode::check(unsafe { f(p, level, repo) }).map(drop)
    }

    /// `get`: `Ok(None)` for `GIT_ENOTFOUND` (the backend's message kept).
    fn get(&self, key: &CStr) -> Result<Option<EntryBox>> {
        let p = self.raw();
        let f = slot!(self, get).ok_or_else(|| no_slot("get"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `f` is the `get` slot of `p`, called with its live table
        // (the trait's invariant), a NUL-terminated key borrowed for the
        // call, and a local out-parameter.
        let r = unsafe { f(p, key.as_ptr(), &raw mut out) };
        if r == ErrorCode::NOT_FOUND.raw() {
            return Ok(None);
        }
        if r != 0 {
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        // SAFETY: on success the slot handed over an entry, ours to free.
        Ok(unsafe { EntryBox::from_raw(out) })
    }

    /// `set`.
    fn set(&self, name: &CStr, value: &CStr) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, set).ok_or_else(|| no_slot("set"))?;
        // SAFETY: `f` is the `set` slot of `p`, called with its live table
        // (the trait's invariant) and two NUL-terminated strings borrowed
        // for the call (the backend copies what it keeps).
        ErrorCode::check(unsafe { f(p, name.as_ptr(), value.as_ptr()) })
    }

    /// `set_multivar`.
    fn set_multivar(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
        value: Option<&CStr>,
    ) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, set_multivar).ok_or_else(|| no_slot("set_multivar"))?;
        // SAFETY: as in `set`; `regexp` and `value` may be NULL, which the
        // slot checks (the file backend rejects a NULL `regexp`).
        ErrorCode::check(unsafe { f(p, name.as_ptr(), opt_ptr(regexp), opt_ptr(value)) })
    }

    /// `del`.
    fn del(&self, name: &CStr) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, del).ok_or_else(|| no_slot("del"))?;
        // SAFETY: `f` is the `del` slot of `p`, called with its live table
        // (the trait's invariant) and a NUL-terminated name borrowed for
        // the call.
        ErrorCode::check(unsafe { f(p, name.as_ptr()) })
    }

    /// `del_multivar`.
    fn del_multivar(&self, name: &CStr, regexp: Option<&CStr>) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, del_multivar).ok_or_else(|| no_slot("del_multivar"))?;
        // SAFETY: as in `del`; `regexp` may be NULL, which the slot checks.
        ErrorCode::check(unsafe { f(p, name.as_ptr(), opt_ptr(regexp)) })
    }

    /// `iterator`.
    fn iterator(&self) -> Result<IterBox> {
        let p = self.raw();
        let f = slot!(self, iterator).ok_or_else(|| no_slot("iterator"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `f` is the `iterator` slot of `p`, called with a local
        // out-parameter and its live table (the trait's invariant).
        let r = unsafe { f(&raw mut out, p) };
        if r != 0 {
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        // SAFETY: on success the slot handed over an iterator, ours to free.
        unsafe { IterBox::from_raw(out) }.ok_or_else(|| no_slot("iterator"))
    }

    /// `snapshot`: a new backend, ours.
    fn snapshot(&self) -> Result<Backend> {
        let p = self.raw();
        let f = slot!(self, snapshot).ok_or_else(|| no_slot("snapshot"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `f` is the `snapshot` slot of `p`, called with a local
        // out-parameter and its live table (the trait's invariant).
        let r = unsafe { f(&raw mut out, p) };
        if r != 0 {
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        // SAFETY: on success the slot handed over a backend, ours to free.
        unsafe { Backend::from_c(out.cast()) }.ok_or_else(|| no_slot("snapshot"))
    }

    /// `lock`.
    fn lock(&self) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, lock).ok_or_else(|| no_slot("lock"))?;
        // SAFETY: `f` is the `lock` slot of `p`, called with its live table
        // (the trait's invariant).
        ErrorCode::check(unsafe { f(p) })
    }

    /// `unlock`.
    fn unlock(&self, success: c_int) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(self, unlock).ok_or_else(|| no_slot("unlock"))?;
        // SAFETY: `f` is the `unlock` slot of `p`, called with its live
        // table (the trait's invariant).
        ErrorCode::check(unsafe { f(p, success) })
    }
}

impl sealed::Sealed for Backend {}

impl BackendCalls for Backend {
    fn raw(&self) -> *mut git_config_backend {
        self.as_ref().as_ptr().cast_mut().cast()
    }
}

/// A backend table C lent for the extent of a call.
#[derive(Clone, Copy, Debug)]
pub struct BackendPtr(NonNull<git_config_backend>);

impl BackendPtr {
    /// A borrowed table.
    ///
    /// # Safety
    ///
    /// `p` is a live backend for as long as the value is used.
    pub unsafe fn new(p: NonNull<git_config_backend>) -> BackendPtr {
        BackendPtr(p)
    }
}

impl sealed::Sealed for BackendPtr {}

impl BackendCalls for BackendPtr {
    fn raw(&self) -> *mut git_config_backend {
        self.0.as_ptr()
    }
}

/// An entry a backend handed out, released through its `free`.
#[derive(Debug)]
pub struct EntryBox(NonNull<git_config_backend_entry>);

// SAFETY: the one field is the pointer to an entry this box owns alone
// (`from_raw`'s contract). An entry is plain data (strings it points at
// and its `free` slot), and libgit2 hands entries to any thread and frees
// them there (C's contract: they keep their backend's data alive by
// atomic reference counts, as `config_list`'s do). `EntryBox` is not
// `Sync`.
unsafe impl Send for EntryBox {}

impl EntryBox {
    /// Adopt an entry.
    ///
    /// # Safety
    ///
    /// `p` is NULL or an entry a backend handed over, released by nothing
    /// else.
    pub unsafe fn from_raw(p: *mut git_config_backend_entry) -> Option<EntryBox> {
        NonNull::new(p).map(EntryBox)
    }

    /// Give it to C, which frees it with `git_config_entry_free`.
    pub fn into_raw(self) -> *mut git_config_backend_entry {
        let p = self.0.as_ptr();
        core::mem::forget(self);
        p
    }

    /// The public entry.
    pub fn as_entry(&self) -> *mut git_config_entry {
        self.0.as_ptr().cast()
    }

    /// The entry's value (`None` for a bare name).
    pub fn value(&self) -> Option<&[u8]> {
        // SAFETY: a live, aligned entry that this box owns (`from_raw`'s
        // contract); the field is copied through the raw pointer.
        let v = unsafe { (*self.0.as_ptr()).entry.value };
        // SAFETY: an entry's value is NULL or a NUL-terminated string that
        // lives as long as the entry (until `drop`, after the `&self`
        // borrow) and that nothing writes.
        (!v.is_null()).then(|| unsafe { CStr::from_ptr(v) }.to_bytes())
    }

    /// The entry's fields, read once.
    pub fn view(&self) -> EntryView<'_> {
        // SAFETY: a live, aligned entry that this box owns (`from_raw`'s
        // contract), whose `git_config_entry` is its first field; it is
        // copied by value.
        let e = unsafe { self.0.as_ptr().cast::<git_config_entry>().read() };
        // SAFETY: the entry's strings are NULL or NUL-terminated, live as
        // long as the entry (until `drop`, after the borrow of `self` the
        // view carries), and written by nothing.
        unsafe { EntryView::new(&e) }
    }
}

impl Drop for EntryBox {
    fn drop(&mut self) {
        // SAFETY: the entry this box owns (`from_raw`'s contract), released
        // once, here; nothing borrows it any more (`drop` has `&mut`).
        unsafe { entry_free(self.0.as_ptr().cast()) }
    }
}

/// `git_config_entry_free` of a non-NULL entry.
///
/// # Safety
///
/// `entry` is the `entry` of a live `git_config_backend_entry`, released
/// once.
unsafe fn entry_free(entry: *mut git_config_entry) {
    let be = entry.cast::<git_config_backend_entry>();
    // SAFETY: `entry` is the first field of a live, aligned backend entry
    // (this function's contract), so `be` is that entry; the slot is copied
    // through the raw pointer.
    if let Some(free) = unsafe { (*be).free } {
        // SAFETY: the entry's own `free`, called once with it (this
        // function's contract): the slot's contract (sys/config.h).
        unsafe { free(be) };
    }
}

/// The fields of an entry, borrowed (from a live entry).
#[derive(Clone, Copy, Debug)]
pub struct EntryView<'a> {
    /// `name` (empty for a NULL one).
    pub name: &'a [u8],
    /// `value`.
    pub value: Option<&'a [u8]>,
    /// `backend_type`.
    pub backend_type: Option<&'a [u8]>,
    /// `origin_path`.
    pub origin_path: Option<&'a [u8]>,
    /// `include_depth`.
    pub include_depth: c_uint,
    /// `level`.
    pub level: c_int,
}

impl<'a> EntryView<'a> {
    /// The fields of `e`.
    ///
    /// # Safety
    ///
    /// `e`'s strings are NULL or NUL-terminated, and not written or freed
    /// for `'a`.
    pub unsafe fn new(e: &git_config_entry) -> EntryView<'a> {
        let cs = |p: *const c_char| -> Option<&'a [u8]> {
            // SAFETY: each string of `e` is NULL (excluded here) or
            // NUL-terminated, and neither written nor freed for `'a` (this
            // function's contract).
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes())
        };
        EntryView {
            name: cs(e.name).unwrap_or(b""),
            value: cs(e.value),
            backend_type: cs(e.backend_type),
            origin_path: cs(e.origin_path),
            include_depth: e.include_depth,
            level: e.level,
        }
    }

    /// The fields of the entry at `be`.
    ///
    /// # Safety
    ///
    /// `be` is a live, aligned `git_config_backend_entry` whose strings are
    /// NULL or NUL-terminated, and not written or freed, for `'a`.
    pub unsafe fn of(be: *const git_config_backend_entry) -> EntryView<'a> {
        // SAFETY: `be` is a live, aligned entry (this function's contract),
        // whose `git_config_entry` is its first field; copied by value.
        let e = unsafe { be.cast::<git_config_entry>().read() };
        // SAFETY: its strings are as `new` requires (this function's
        // contract).
        unsafe { EntryView::new(&e) }
    }
}

/// A C iterator, released through its `free`.
#[derive(Debug)]
pub struct IterBox(NonNull<git_config_iterator>);

// SAFETY: the one field is the pointer to an iterator this box owns alone
// (`from_raw`'s contract); as for `ConfigIterator`, an iterator may move
// between threads (used by one at a time).
unsafe impl Send for IterBox {}
// SAFETY: `IterBox` has no method taking `&self` that reaches the
// iterator (`next_entry` takes `&mut self`, `into_raw` takes `self`).
unsafe impl Sync for IterBox {}

impl IterBox {
    /// Adopt an iterator.
    ///
    /// # Safety
    ///
    /// `p` is NULL or an iterator handed over, released by nothing else.
    pub unsafe fn from_raw(p: *mut git_config_iterator) -> Option<IterBox> {
        NonNull::new(p).map(IterBox)
    }

    /// Give it to C.
    pub fn into_raw(self) -> *mut git_config_iterator {
        let p = self.0.as_ptr();
        core::mem::forget(self);
        p
    }

    /// `next`: the next entry (owned by the iterator, valid until the next
    /// call or its release); `Err(GIT_ITEROVER)` at the end.
    pub fn next_entry(&mut self) -> Result<NonNull<git_config_backend_entry>> {
        let p = self.0.as_ptr();
        // SAFETY: a live, aligned iterator this box owns (`from_raw`'s
        // contract); the slot is copied through the raw pointer.
        let Some(f) = (unsafe { (*p).next }) else {
            return Err(no_slot("next"));
        };
        let mut out = core::ptr::null_mut();
        // SAFETY: `f` is the iterator's `next`, called with a local
        // out-parameter and the iterator, which `&mut self` makes ours for
        // the call (the slot's contract, sys/config.h).
        let r = unsafe { f(&raw mut out, p) };
        if r != 0 {
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        NonNull::new(out).ok_or_else(|| no_slot("next"))
    }
}

impl Drop for IterBox {
    fn drop(&mut self) {
        let p = self.0.as_ptr();
        // SAFETY: a live, aligned iterator this box owns (`from_raw`'s
        // contract); the slot is copied through the raw pointer.
        if let Some(free) = unsafe { (*p).free } {
            // SAFETY: the iterator's own `free`, called once, here, with
            // the iterator nothing uses afterwards.
            unsafe { free(p) };
        }
    }
}

// ---------------------------------------------------------------------------
// git_config
// ---------------------------------------------------------------------------

/// `backend_instance`: a backend shared by a config, its snapshots and its
/// levels (C's `git_refcount` is the `Arc`'s).
#[derive(Debug)]
pub struct Instance {
    backend: Backend,
}

impl Instance {
    /// The backend.
    pub fn backend(&self) -> &Backend {
        &self.backend
    }
}

/// `backend_entry`: an instance at a level, with its write order.
#[derive(Clone, Debug)]
struct BackendEntry {
    instance: Arc<Instance>,
    level: c_int,
    write_order: c_int,
}

/// The backends of a config: `readers` by decreasing level, `writers` by
/// decreasing write order (as inserted: see [`GitConfig::set_writeorder`]).
#[derive(Clone, Debug, Default)]
pub struct Backends {
    readers: Vec<BackendEntry>,
    writers: Vec<BackendEntry>,
}

impl Backends {
    /// `get_writer_instance`.
    fn writer(&self) -> Option<&Arc<Instance>> {
        self.writers
            .iter()
            .find(|e| !e.instance.backend.is_readonly() && e.write_order >= 0)
            .map(|e| &e.instance)
    }

    /// The backends to read, by decreasing level.
    pub fn readers(&self) -> impl DoubleEndedIterator<Item = (&Arc<Instance>, c_int)> + '_ {
        self.readers.iter().map(|e| (&e.instance, e.level))
    }
}

/// `git_config`: a reference count (the C-visible prefix) and the
/// backends.
#[repr(C)]
#[derive(Debug)]
pub struct GitConfig {
    /// `rc`: C takes and drops references (`GIT_REFCOUNT_INC`,
    /// `git_config_free`) and sets the owner (`GIT_REFCOUNT_OWN`).
    pub rc: GitRefcount,
    backends: std::sync::RwLock<Arc<Backends>>,
}

crate::impl_ccounted!(GitConfig, std::alloc::Global);

// SAFETY: C writes only `rc` (GIT_REFCOUNT_*, atomics); the backends are
// behind an `RwLock`.
unsafe impl ffibox::CShareable for GitConfig {}

/// `git_vector_insert_sorted`'s position for `key` (with `git__bsearch` and
/// comparison `cmp(key, element)`), or the position of an equal element.
fn sorted_position<T>(v: &[T], cmp: impl FnMut(&T) -> c_int) -> core::result::Result<usize, usize> {
    crate::util::util::bsearch(v, cmp)
}

/// `GIT_ERROR_CHECK_VERSION`.
pub(crate) fn check_version(actual: c_uint, expected_max: c_uint, name: &str) -> Result<()> {
    if actual > 0 && actual <= expected_max {
        return Ok(());
    }
    Err(git_error_set!(
        ErrorClass::INVALID,
        "invalid version {} on {name}",
        actual as c_int
    ))
}

/// `config_error_notfound`.
fn error_notfound(name: &[u8]) -> ErrorCode {
    let _ = git_error_set!(
        ErrorClass::CONFIG,
        "config value '{}' was not found",
        Bytes(name)
    );
    ErrorCode::NOT_FOUND
}

/// How much of a lookup's failure `get_entry` reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    /// `GET_ALL_ERRORS`.
    AllErrors,
    /// `GET_NO_MISSING`: a missing entry is `None`.
    NoMissing,
    /// `GET_NO_ERRORS`: any failure is `None` (the error cleared).
    NoErrors,
}

/// A NUL-terminated copy of `s`.
pub(crate) fn cstring(s: &[u8]) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(s.len() + 1)
        .map_err(|_| errors::oom())?;
    v.extend_from_slice(s);
    v.push(0);
    Ok(v)
}

/// The `CStr` of a [`cstring`] (up to its first NUL).
pub(crate) fn as_cstr(v: &[u8]) -> &CStr {
    CStr::from_bytes_until_nul(v).unwrap_or(c"")
}

impl GitConfig {
    /// `git_config_new`: a config with one reference.
    pub fn new() -> Box<GitConfig> {
        Box::new(GitConfig {
            rc: GitRefcount::one(),
            backends: std::sync::RwLock::new(Arc::new(Backends::default())),
        })
    }

    /// The current backends (shared: later changes replace them).
    pub fn backends(&self) -> Arc<Backends> {
        // Readers share the lock; it is released before any backend runs.
        let g = self.backends.read().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&g)
    }

    /// Change the backends: `f` edits a copy, which replaces them if it
    /// succeeds. What `f` drops is dropped after the lock is released.
    fn update<R>(&self, f: impl FnOnce(&mut Backends) -> Result<R>) -> Result<R> {
        let old;
        let r;
        {
            let mut g = self.backends.write().unwrap_or_else(|e| e.into_inner());
            let mut copy = Backends::clone(&g);
            r = f(&mut copy)?;
            old = core::mem::replace(&mut *g, Arc::new(copy));
        }
        drop(old);
        Ok(r)
    }

    /// The repository that owns the config (`GIT_REFCOUNT_OWNER`).
    pub fn owner(&self) -> *mut git_repository {
        self.rc.owner().cast()
    }

    /// `git_config__add_instance`.
    fn add_instance(&self, instance: &Arc<Instance>, level: c_int, force: bool) -> Result<()> {
        self.update(|b| {
            if force {
                b.remove_level(level);
            }
            let entry = BackendEntry {
                instance: Arc::clone(instance),
                level,
                write_order: level,
            };
            match sorted_position(&b.readers, |p| p.level - entry.level) {
                Ok(pos) => {
                    let _ = git_error_set!(
                        ErrorClass::CONFIG,
                        "configuration at level {} already exists",
                        b.readers[pos].level
                    );
                    return Err(ErrorCode::EXISTS);
                }
                Err(pos) => {
                    b.readers.try_reserve(1).map_err(|_| errors::oom())?;
                    b.readers.insert(pos, entry.clone());
                }
            }
            let pos = match sorted_position(&b.writers, |p| p.write_order - entry.write_order) {
                Ok(p) | Err(p) => p,
            };
            b.writers.try_reserve(1).map_err(|_| errors::oom())?;
            b.writers.insert(pos, entry);
            Ok(())
        })
    }

    /// `git_config_add_backend`: open `backend` at `level` and add it. On
    /// failure the backend is given back (C leaves it to the caller).
    ///
    /// # Safety
    ///
    /// As [`BackendCalls::open`] for `repo`.
    pub unsafe fn add_backend(
        &self,
        backend: Backend,
        level: c_int,
        repo: *const git_repository,
        force: bool,
    ) -> core::result::Result<(), (ErrorCode, Backend)> {
        let p = backend.raw();
        // SAFETY: `p` is the live, aligned table `backend` owns; the field
        // is copied through the raw pointer.
        let version = unsafe { (*p).version };
        if let Err(e) = check_version(version, GIT_CONFIG_BACKEND_VERSION, "git_config_backend") {
            return Err((e, backend));
        }
        // SAFETY: `repo` per this function's contract.
        if let Err(e) = unsafe { backend.open(level, repo) } {
            return Err((e, backend));
        }
        // SAFETY: `p` is the live, aligned table `backend` owns, not yet
        // shared with any other config or thread (the instance is made
        // below), and no reference covers it: C's own write of the field.
        unsafe { (*p).cfg = core::ptr::from_ref(self).cast_mut() };
        let instance = Arc::new(Instance { backend });
        match self.add_instance(&instance, level, force) {
            Ok(()) => Ok(()),
            Err(e) => match Arc::try_unwrap(instance) {
                Ok(i) => Err((e, i.backend)),
                // Not reached: nothing else holds the new instance.
                Err(_) => Err((e, unreachable_backend())),
            },
        }
    }

    /// [`add_backend`](Self::add_backend) with no repository.
    pub fn add_backend_norepo(
        &self,
        backend: Backend,
        level: c_int,
        force: bool,
    ) -> core::result::Result<(), (ErrorCode, Backend)> {
        // SAFETY: NULL is a valid repository argument.
        unsafe { self.add_backend(backend, level, core::ptr::null(), force) }
    }

    /// `git_config_set_writeorder`: the write order of the backend at
    /// `levels[j]` is `j`, and -1 for the others.
    ///
    /// As in C, the writers are not sorted again: C calls `git_vector_sort`,
    /// which does nothing on a vector flagged sorted, as the writers always
    /// are (see DEFECTS.md).
    pub fn set_writeorder(&self, levels: &[c_int]) -> Result<()> {
        git_assert!(levels.len() < c_int::MAX as usize, "len < INT_MAX");
        self.update(|b| {
            let order = |level: c_int| {
                levels
                    .iter()
                    .position(|&l| l == level)
                    .map_or(-1, |j| j as c_int)
            };
            for e in b.readers.iter_mut().chain(b.writers.iter_mut()) {
                e.write_order = order(e.level);
            }
            Ok(())
        })
    }

    /// `find_backend_by_level`.
    fn find_by_level(&self, level: c_int) -> Result<Arc<Instance>> {
        let b = self.backends();
        let found = if level == GIT_CONFIG_HIGHEST_LEVEL {
            b.readers.first()
        } else {
            b.readers.iter().find(|e| e.level == level)
        };
        match found {
            Some(e) => Ok(Arc::clone(&e.instance)),
            None => {
                let _ = git_error_set!(
                    ErrorClass::CONFIG,
                    "no configuration exists for the given level '{level}'"
                );
                Err(ErrorCode::NOT_FOUND)
            }
        }
    }

    /// `git_config_open_level`: a new config with the backend at `level`.
    pub fn open_level(&self, level: c_int) -> Result<Box<GitConfig>> {
        let instance = self.find_by_level(level)?;
        let config = GitConfig::new();
        config.add_instance(&instance, level, true)?;
        Ok(config)
    }

    /// `git_config_open_global`.
    pub fn open_global(&self) -> Result<Box<GitConfig>> {
        match self.open_level(GIT_CONFIG_LEVEL_XDG) {
            Ok(c) => Ok(c),
            Err(e) if e == ErrorCode::NOT_FOUND => self.open_level(GIT_CONFIG_LEVEL_GLOBAL),
            Err(e) => Err(e),
        }
    }

    /// `git_config_snapshot`.
    pub fn snapshot(&self) -> Result<Box<GitConfig>> {
        let config = GitConfig::new();
        let mut result = Ok(());
        for e in &self.backends().readers {
            match e.instance.backend.snapshot() {
                Err(err) => {
                    result = Err(err);
                    break;
                }
                Ok(b) => {
                    if let Err((err, b)) = config.add_backend_norepo(b, e.level, false) {
                        drop(b);
                        result = Err(err);
                        break;
                    }
                }
            }
        }
        let _ = config.set_writeorder(&[]);
        result.map(|()| config)
    }

    /// `get_entry`: the entry of the first backend that has `name`
    /// (normalized first if `normalize`).
    pub fn get_entry(&self, name: &CStr, normalize: bool, want: Want) -> Result<Option<EntryBox>> {
        let res = self.get_entry_inner(name, normalize);
        match res {
            Ok(Some(e)) => Ok(Some(e)),
            Ok(None) => {
                if want != Want::AllErrors {
                    Ok(None)
                } else {
                    Err(error_notfound(name.to_bytes()))
                }
            }
            Err(e) if e == ErrorCode::NOT_FOUND => {
                if want != Want::AllErrors {
                    Ok(None)
                } else {
                    Err(error_notfound(name.to_bytes()))
                }
            }
            Err(e) => {
                if want == Want::NoErrors {
                    errors::clear();
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn get_entry_inner(&self, name: &CStr, normalize: bool) -> Result<Option<EntryBox>> {
        let normalized;
        let key = if normalize {
            normalized = Key::normalize(name.to_bytes())?;
            normalized.as_cstr()
        } else {
            name
        };
        for (instance, _) in self.backends().readers() {
            if let Some(e) = instance.backend.get(key)? {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// `is_readonly`: whether no writer can write.
    fn is_readonly(&self) -> bool {
        self.backends()
            .writers
            .iter()
            .all(|e| e.instance.backend.is_readonly())
    }

    /// `get_writer`.
    fn writer(&self) -> Option<Arc<Instance>> {
        self.backends().writer().cloned()
    }

    /// `git_config_set_string`.
    pub fn set_string(&self, name: &CStr, value: Option<&CStr>) -> Result<()> {
        let Some(value) = value else {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "the value to set cannot be NULL"
            ));
        };
        let Some(w) = self.writer() else {
            let _ = git_error_set!(
                ErrorClass::CONFIG,
                "cannot set '{}': the configuration is read-only",
                Bytes(name.to_bytes())
            );
            return Err(ErrorCode::READ_ONLY);
        };
        let r = w.backend.set(name, value);
        if r == Ok(0) {
            let owner = self.owner();
            if !owner.is_null() {
                // SAFETY: the owner is a live repository while it owns the
                // config (it clears the owner before it is freed); it is
                // `Sync`.
                crate::config_cache::configmap_lookup_cache_clear(unsafe { &*owner });
            }
        }
        r.map(drop)
    }

    /// `git_config_set_multivar`.
    pub fn set_multivar(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
        value: Option<&CStr>,
    ) -> Result<c_int> {
        let Some(w) = self.writer() else {
            let _ = git_error_set!(
                ErrorClass::CONFIG,
                "cannot set '{}': the configuration is read-only",
                Bytes(name.to_bytes())
            );
            return Err(ErrorCode::READ_ONLY);
        };
        w.backend.set_multivar(name, regexp, value)
    }

    /// `git_config_delete_entry`.
    pub fn delete_entry(&self, name: &CStr) -> Result<c_int> {
        let Some(w) = self.writer() else {
            return Err(ErrorCode::READ_ONLY);
        };
        w.backend.del(name)
    }

    /// `git_config_delete_multivar`.
    pub fn delete_multivar(&self, name: &CStr, regexp: Option<&CStr>) -> Result<c_int> {
        let Some(w) = self.writer() else {
            return Err(ErrorCode::READ_ONLY);
        };
        w.backend.del_multivar(name, regexp)
    }

    /// `git_config__update_entry`.
    pub fn update_entry(
        &self,
        key: &CStr,
        value: Option<&CStr>,
        overwrite_existing: bool,
        only_if_existing: bool,
    ) -> Result<()> {
        let ce = self.get_entry(key, false, Want::NoMissing)?;
        let old = ce.as_ref().map(|e| e.value().map(<[u8]>::to_vec));
        match (&ce, &old) {
            (None, _) if only_if_existing => return Ok(()),
            (Some(_), _) if !overwrite_existing => return Ok(()),
            _ => {}
        }
        let old_value = old.flatten();
        if let (Some(v), Some(o)) = (value, &old_value)
            && v.to_bytes() == o.as_slice()
        {
            return Ok(());
        }
        if value.is_none() && old_value.is_none() {
            return Ok(());
        }
        let r = match value {
            None => self.delete_entry(key).map(drop),
            Some(v) => self.set_string(key, Some(v)),
        };
        drop(ce);
        r
    }

    /// `git_config_iterator_glob_new` (`git_config_iterator_new` without a
    /// pattern).
    pub fn iterator(&self, regexp: Option<&CStr>) -> Result<IterBox> {
        let regex = match regexp {
            Some(r) => Some(Regexp::compile_opt(Some(r), 0).map_err(|_| ErrorCode::GENERIC)?),
            None => None,
        };
        let backends = self.backends();
        let i = backends.readers.len();
        let it = AllIter {
            backends,
            i,
            current: None,
            regex,
        };
        let p = CTable::<ConfigIterator, AllIter>::new(it).into_c();
        // SAFETY: `p` is the table just surrendered by `into_c`, owned by
        // nothing else; its `free` slot (`release free`) releases it.
        Ok(unsafe { IterBox::from_raw(p.as_ptr()) }.unwrap_or_else(|| unreachable!()))
    }

    /// `git_config_multivar_iterator_new`.
    pub fn multivar_iterator(&self, name: &CStr, regexp: Option<&CStr>) -> Result<IterBox> {
        let inner = self.iterator(None)?;
        let name = normalize_name(name.to_bytes())?;
        let regex = match regexp {
            Some(r) => Some(Regexp::compile_opt(Some(r), 0)?),
            None => None,
        };
        let it = MultivarIter { inner, name, regex };
        let p = CTable::<ConfigIterator, MultivarIter>::new(it).into_c();
        // SAFETY: as in `iterator`.
        Ok(unsafe { IterBox::from_raw(p.as_ptr()) }.unwrap_or_else(|| unreachable!()))
    }

    /// `git_config_foreach_match`, with `cb` over each entry; a non-zero
    /// return of `cb` stops it and is the result.
    pub fn foreach_match(
        &self,
        regexp: Option<&CStr>,
        mut cb: impl FnMut(*const git_config_entry) -> c_int,
    ) -> c_int {
        let mut iter = match self.iterator(regexp) {
            Ok(i) => i,
            Err(e) => return e.raw(),
        };
        let mut error;
        loop {
            match iter.next_entry() {
                Ok(be) => {
                    error = cb(be.as_ptr().cast());
                    if error != 0 {
                        errors::set_after_callback(error, "git_config_foreach_match");
                        break;
                    }
                }
                Err(e) => {
                    error = e.raw();
                    break;
                }
            }
        }
        drop(iter);
        if error == ErrorCode::ITER_OVER.raw() {
            0
        } else {
            error
        }
    }

    /// `git_config_get_multivar_foreach`.
    pub fn get_multivar_foreach(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
        mut cb: impl FnMut(*const git_config_entry) -> c_int,
    ) -> c_int {
        let mut iter = match self.multivar_iterator(name, regexp) {
            Ok(i) => i,
            Err(e) => return e.raw(),
        };
        let mut found = false;
        let mut err;
        loop {
            match iter.next_entry() {
                Ok(be) => {
                    found = true;
                    err = cb(be.as_ptr().cast());
                    if err != 0 {
                        errors::set_after_callback(err, "git_config_get_multivar_foreach");
                        break;
                    }
                }
                Err(e) => {
                    err = e.raw();
                    break;
                }
            }
        }
        drop(iter);
        if err == ErrorCode::ITER_OVER.raw() {
            err = 0;
        }
        if !found && err == 0 {
            err = error_notfound(name.to_bytes()).raw();
        }
        err
    }

    /// `git_config_lock`: lock the writer, and write to `out` a transaction
    /// that unlocks it. The transaction points at `self` without a
    /// reference: whoever uses or frees it (C, through `unsafe` calls)
    /// keeps `self` alive meanwhile, as C's contract says.
    fn lock(&self, out: Out<'_, *mut crate::sys::transaction::git_transaction>) -> Result<()> {
        let Some(instance) = self.writer() else {
            let _ = git_error_set!(
                ErrorClass::CONFIG,
                "cannot lock: the configuration is read-only"
            );
            return Err(ErrorCode::READ_ONLY);
        };
        instance.backend.lock()?;
        let data = Arc::into_raw(Arc::clone(&instance))
            .cast_mut()
            .cast::<c_void>();
        let this = core::ptr::from_ref(self).cast_mut();
        // SAFETY: `this` is `self`, live for the call; the transaction keeps
        // it only as the pointer `git_config_unlock` is passed, which its
        // users keep alive (see above); `data` is a reference to the writer
        // instance (`Arc::into_raw`), which `git_config_unlock` takes back.
        let r = unsafe { crate::sys::transaction::config_new(out, this, data) };
        if r.is_err() {
            // SAFETY: `data` is the `Arc::into_raw` made above, which the
            // failed call did not hand over: taken back once here.
            drop(unsafe { Arc::from_raw(data.cast_const().cast::<Instance>()) });
        }
        r
    }
}

/// The typed lookups and stores of the C API (`git_config_get_bool`, …),
/// for the crate's callers (the repository) and the exports.
impl GitConfig {
    /// The value of the entry `name` (normalized), with `f`: the C
    /// exports' `get_entry(…, GET_ALL_ERRORS)`.
    pub(crate) fn with_value<T>(
        &self,
        name: &CStr,
        f: impl FnOnce(Option<&[u8]>) -> Result<T>,
    ) -> Result<T> {
        let e = self.get_entry(name, true, Want::AllErrors)?;
        let e = e.ok_or(ErrorCode::NOT_FOUND)?;
        f(e.value())
    }

    /// `git_config_get_bool`.
    pub(crate) fn get_bool(&self, name: &CStr) -> Result<bool> {
        self.with_value(name, parse_bool)
    }

    /// `git_config_get_int32`.
    pub(crate) fn get_int32(&self, name: &CStr) -> Result<i32> {
        self.with_value(name, parse_int32)
    }

    /// `git_config__get_path`.
    pub(crate) fn get_path(&self, out: &mut GitStr, name: &CStr) -> Result<()> {
        self.with_value(name, |v| {
            // `GIT_ASSERT_ARG(value)` in `git_config__parse_path`.
            let Some(v) = v else {
                return Err(errors::assert_arg_failed("value"));
            };
            parse_path(out, v)
        })
    }

    /// `git_config__get_string_buf`.
    pub(crate) fn get_string_buf(&self, out: &mut GitStr, name: &CStr) -> Result<()> {
        self.with_value(name, |v| out.puts(v.unwrap_or(b"")))
    }

    /// `git_config_set_bool`.
    pub(crate) fn set_bool(&self, name: &CStr, value: bool) -> Result<()> {
        self.set_string(name, Some(if value { c"true" } else { c"false" }))
    }

    /// `git_config_set_int64`: the decimal value, formatted on the stack
    /// (C's `char str_value[32]`).
    pub(crate) fn set_int64(&self, name: &CStr, value: i64) -> Result<()> {
        use std::io::Write as _;
        let mut buf = [0u8; 32];
        let mut w = &mut buf[..31];
        // An `i64` has at most 20 characters: this cannot fail.
        let _ = write!(w, "{value}");
        let len = 31 - w.len();
        let s = CStr::from_bytes_until_nul(&buf[..=len]).unwrap_or(c"");
        self.set_string(name, Some(s))
    }

    /// `git_config_set_int32`.
    pub(crate) fn set_int32(&self, name: &CStr, value: i32) -> Result<()> {
        self.set_int64(name, i64::from(value))
    }

    /// `git_config_foreach_match` with a Rust closure over each entry; its
    /// error stops the iteration and is the result (as C's callback's).
    pub(crate) fn foreach_match_with(
        &self,
        regexp: Option<&CStr>,
        mut f: impl FnMut(&EntryView<'_>) -> Result<()>,
    ) -> Result<()> {
        let r = self.foreach_match(regexp, |e| {
            // SAFETY: `e` is the entry the config's iterator holds (the
            // `git_config_entry` at the start of a backend entry), live
            // and unwritten, with its strings, until the next `next` call,
            // after this callback.
            let view = unsafe { EntryView::of(e.cast()) };
            match f(&view) {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        });
        ErrorCode::check(r).map(drop)
    }

    /// `git_config_get_multivar_foreach` with a Rust closure (as
    /// [`foreach_match_with`](Self::foreach_match_with)).
    pub(crate) fn multivar_foreach_with(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
        mut f: impl FnMut(&EntryView<'_>) -> Result<()>,
    ) -> Result<()> {
        let r = self.get_multivar_foreach(name, regexp, |e| {
            // SAFETY: `e` is the entry the config's iterator holds (the
            // `git_config_entry` at the start of a backend entry), live
            // and unwritten, with its strings, until the next `next` call,
            // after this callback.
            let view = unsafe { EntryView::of(e.cast()) };
            match f(&view) {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        });
        ErrorCode::check(r).map(drop)
    }
}

impl Backends {
    /// `try_remove_existing_backend`.
    fn remove_level(&mut self, level: c_int) {
        let Some(i) = self.readers.iter().position(|e| e.level == level) else {
            return;
        };
        self.readers.remove(i);
        if let Some(j) = self.writers.iter().position(|e| e.level == level) {
            self.writers.remove(j);
        }
    }
}

/// Never called: the backend of an instance that cannot be unwrapped.
#[cold]
fn unreachable_backend() -> Backend {
    unreachable!("a new instance has no other holder")
}

// ---------------------------------------------------------------------------
// Iterators
// ---------------------------------------------------------------------------

/// `all_iter`: the entries of every backend, from the lowest level up,
/// filtered by name (`git_config_iterator_glob_new`).
struct AllIter {
    backends: Arc<Backends>,
    i: usize,
    current: Option<IterBox>,
    regex: Option<Regexp>,
}

impl AllIter {
    /// `all_iter_next`.
    fn next_any(&mut self) -> Result<NonNull<git_config_backend_entry>> {
        if let Some(cur) = &mut self.current {
            match cur.next_entry() {
                Ok(be) => return Ok(be),
                Err(e) if e == ErrorCode::ITER_OVER => {}
                Err(e) => return Err(e),
            }
        }
        loop {
            if self.i == 0 {
                return Err(ErrorCode::ITER_OVER);
            }
            let instance = Arc::clone(&self.backends.readers[self.i - 1].instance);
            self.i -= 1;
            self.current = None;
            match instance.backend.iterator() {
                Err(e) if e == ErrorCode::NOT_FOUND => continue,
                Err(e) => return Err(e),
                Ok(it) => {
                    let cur = self.current.insert(it);
                    match cur.next_entry() {
                        Ok(be) => return Ok(be),
                        Err(e) if e == ErrorCode::ITER_OVER => continue,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    /// `all_iter_next`, or `all_iter_glob_next` with a pattern.
    fn next_entry(&mut self) -> Result<NonNull<git_config_backend_entry>> {
        loop {
            let be = self.next_any()?;
            let Some(regex) = &self.regex else {
                return Ok(be);
            };
            // SAFETY: `be` is the entry the current backend iterator just
            // returned: live and unwritten until its next `next` (after the
            // name's last use here).
            let name = unsafe { EntryView::of(be.as_ptr()) }.name;
            if regex.matches(name).is_ok() {
                return Ok(be);
            }
        }
    }
}

/// Write an iterator's result to C's out-parameter.
fn iter_result(
    mut out: Out<'_, *mut git_config_backend_entry>,
    r: Result<NonNull<git_config_backend_entry>>,
) -> c_int {
    match r {
        Ok(be) => {
            out.write(be.as_ptr());
            0
        }
        Err(e) => e.raw(),
    }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes an iterator's `next` the iterator, second, and
    // frees it once through `free`; both are set.
    unsafe impl CVtable<AllIter> for ConfigIterator {}
    base = git_config_iterator::default();
    /// `all_iter_next` / `all_iter_glob_next`.
    fn next(entry: *mut *mut git_config_backend_entry, @iter) -> c_int {
        on panic: slot_panicked(),
        on busy: iter_busy(),
        // SAFETY: `entry` is writable for the call (`next`'s contract,
        // sys/config.h), and nothing else references `*entry`.
        |it: &mut AllIter| iter_result(unsafe { Out::from_ptr(entry) }, it.next_entry()),
    }
    release free;
}

/// `multivar_iter`: the entries named `name` whose value matches.
struct MultivarIter {
    inner: IterBox,
    name: Vec<u8>,
    regex: Option<Regexp>,
}

impl MultivarIter {
    /// `multivar_iter_next`. A NULL value matches as `""` (C would crash).
    fn next_entry(&mut self) -> Result<NonNull<git_config_backend_entry>> {
        loop {
            let be = self.inner.next_entry()?;
            // SAFETY: `be` is the entry the inner iterator just returned:
            // live and unwritten until its next `next` (after `v`'s last
            // use).
            let v = unsafe { EntryView::of(be.as_ptr()) };
            if v.name != self.name.as_slice() {
                continue;
            }
            let Some(regex) = &self.regex else {
                return Ok(be);
            };
            if regex.matches(v.value.unwrap_or(b"")).is_ok() {
                return Ok(be);
            }
        }
    }
}

ffibox::impl_cvtable! {
    // SAFETY: as for `AllIter`.
    unsafe impl CVtable<MultivarIter> for ConfigIterator {}
    base = git_config_iterator::default();
    /// `multivar_iter_next`.
    fn next(entry: *mut *mut git_config_backend_entry, @iter) -> c_int {
        on panic: slot_panicked(),
        on busy: iter_busy(),
        // SAFETY: as for `AllIter`.
        |it: &mut MultivarIter| iter_result(unsafe { Out::from_ptr(entry) }, it.next_entry()),
    }
    release free;
}

/// `git_config_backend_foreach_match`, with `cb` over each entry.
pub fn backend_foreach_match(
    backend: &dyn BackendCalls,
    regexp: Option<&CStr>,
    mut cb: impl FnMut(*const git_config_entry) -> c_int,
) -> c_int {
    let regex = match regexp {
        Some(r) => match Regexp::compile_opt(Some(r), 0) {
            Ok(r) => Some(r),
            Err(_) => return -1,
        },
        None => None,
    };
    let Ok(mut iter) = backend.iterator() else {
        return -1;
    };
    let mut error = 0;
    while let Ok(be) = iter.next_entry() {
        if let Some(regex) = &regex {
            // SAFETY: `be` is the entry `iter` just returned: live and
            // unwritten until its next `next` (after `name`'s last use).
            let name = unsafe { EntryView::of(be.as_ptr()) }.name;
            if regex.matches(name).is_err() {
                continue;
            }
        }
        error = cb(be.as_ptr().cast());
        if error != 0 {
            errors::set_after_callback(error, "git_config_backend_foreach_match");
            break;
        }
    }
    error
}

// ---------------------------------------------------------------------------
// Names and values
// ---------------------------------------------------------------------------

/// `normalize_section`: check `s` (alphanumerics and `-`, not first, up to
/// its end) and lower-case it.
fn normalize_section(s: &mut [u8]) -> Result<()> {
    if s.is_empty() {
        return Err(ErrorCode::INVALID_SPEC);
    }
    for (i, c) in s.iter_mut().enumerate() {
        if c.is_ascii_alphanumeric() {
            *c = c.to_ascii_lowercase();
        } else if *c != b'-' || i == 0 {
            return Err(ErrorCode::INVALID_SPEC);
        }
    }
    Ok(())
}

/// `git_config__normalize_name`: `section[.subsection].name` with the
/// section and name lower-cased (`GIT_EINVALIDSPEC` if it is not one).
pub fn normalize_name(input: &[u8]) -> Result<Vec<u8>> {
    let input = until_nul(input);
    let mut name = Vec::new();
    name.try_reserve_exact(input.len())
        .map_err(|_| errors::oom())?;
    name.extend_from_slice(input);
    normalize_in_place(&mut name, input)?;
    Ok(name)
}

/// The checks and lower-casing of [`normalize_name`] on `name` (a copy of
/// `input`, which the error message quotes).
fn normalize_in_place(name: &mut [u8], input: &[u8]) -> Result<()> {
    let invalid = || {
        let _ = git_error_set!(
            ErrorClass::CONFIG,
            "invalid config item name '{}'",
            Bytes(input)
        );
        ErrorCode::INVALID_SPEC
    };
    let (Some(fdot), Some(ldot)) = (memchr::memchr(b'.', name), memchr::memrchr(b'.', name)) else {
        return Err(invalid());
    };
    if fdot == 0 || ldot + 1 == name.len() {
        return Err(invalid());
    }
    if normalize_section(&mut name[..fdot]).is_err()
        || normalize_section(&mut name[ldot + 1..]).is_err()
    {
        return Err(invalid());
    }
    if name[fdot..ldot].contains(&b'\n') {
        return Err(invalid());
    }
    Ok(())
}

/// A normalized name and its NUL, on the stack when it is short (a lookup
/// allocates nothing for it).
enum Key {
    Stack([u8; 128], usize),
    Heap(Vec<u8>),
}

impl Key {
    /// [`normalize_name`] of `input`.
    fn normalize(input: &[u8]) -> Result<Key> {
        let input = until_nul(input);
        if input.len() < 128 {
            let mut b = [0u8; 128];
            b[..input.len()].copy_from_slice(input);
            normalize_in_place(&mut b[..input.len()], input)?;
            Ok(Key::Stack(b, input.len()))
        } else {
            Ok(Key::Heap(cstring(&normalize_name(input)?)?))
        }
    }

    fn as_cstr(&self) -> &CStr {
        match self {
            Key::Stack(b, n) => as_cstr(&b[..=*n]),
            Key::Heap(v) => as_cstr(v),
        }
    }
}

/// What a C string holds: up to its first NUL.
fn until_nul(s: &[u8]) -> &[u8] {
    match memchr::memchr(0, s) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// The text C's `%s` prints for a NULL-able string.
fn or_null(value: Option<&[u8]>) -> Bytes<'_> {
    Bytes(value.unwrap_or(b"(null)"))
}

/// `git_config_parse_bool`.
pub fn parse_bool(value: Option<&[u8]>) -> Result<bool> {
    if let Some(b) = crate::util::util::parse_bool(value) {
        return Ok(b);
    }
    if let Ok(n) = parse_int32(value) {
        return Ok(n != 0);
    }
    Err(git_error_set!(
        ErrorClass::CONFIG,
        "failed to parse '{}' as a boolean",
        or_null(value)
    ))
}

/// `git_config_parse_int64`, with C's `k`/`m`/`g` suffixes.
pub fn parse_int64(value: Option<&[u8]>) -> Result<i64> {
    let fail = || {
        git_error_set!(
            ErrorClass::CONFIG,
            "failed to parse '{}' as an integer",
            or_null(value)
        )
    };
    let Some(v) = value else { return Err(fail()) };
    let Ok((mut num, end)) = crate::util::util::strntol64(v, 0) else {
        return Err(fail());
    };
    let at = |i: usize| v.get(i).copied().unwrap_or(0);
    let shifts = match at(end) {
        b'g' | b'G' => 3,
        b'm' | b'M' => 2,
        b'k' | b'K' => 1,
        0 => return Ok(num),
        _ => return Err(fail()),
    };
    for _ in 0..shifts {
        num = num.checked_mul(1024).ok_or_else(fail)?;
    }
    // Nothing may follow the suffix (C fails without a message).
    if at(end + 1) != 0 {
        return Err(ErrorCode::GENERIC);
    }
    Ok(num)
}

/// `git_config_parse_int32`.
pub fn parse_int32(value: Option<&[u8]>) -> Result<i32> {
    let fail = || {
        git_error_set!(
            ErrorClass::CONFIG,
            "failed to parse '{}' as a 32-bit integer",
            or_null(value)
        )
    };
    let tmp = parse_int64(value).map_err(|_| fail())?;
    let truncate = tmp as i32;
    if i64::from(truncate) != tmp {
        return Err(fail());
    }
    Ok(truncate)
}

/// `git_config__parse_path`: `value`, with a leading `~/` expanded to the
/// home directory.
pub fn parse_path(out: &mut GitStr, value: &[u8]) -> Result<()> {
    if value.first() == Some(&b'~') {
        match value.get(1) {
            None => return crate::sys::sysdir::expand_homedir_file(out, None),
            Some(b'/') => {
                let rest = cstring(&value[2..])?;
                return crate::sys::sysdir::expand_homedir_file(out, Some(as_cstr(&rest)));
            }
            Some(_) => {
                return Err(git_error_set!(
                    ErrorClass::CONFIG,
                    "retrieving a homedir by name is not supported"
                ));
            }
        }
    }
    out.sets(value)
}

/// A map of `git_config_get_mapped`, in Rust.
#[derive(Clone, Copy, Debug)]
pub struct RawConfigMap<'a> {
    /// `type` (`GIT_CONFIGMAP_*`).
    pub kind: c_int,
    /// `str_match`.
    pub str_match: Option<&'a [u8]>,
    /// `map_value`.
    pub map_value: c_int,
}

/// `git_config_lookup_map_value`.
pub fn lookup_map_value(maps: &[RawConfigMap<'_>], value: Option<&[u8]>) -> Result<c_int> {
    for m in maps {
        match m.kind {
            GIT_CONFIGMAP_FALSE | GIT_CONFIGMAP_TRUE => {
                if let Ok(b) = parse_bool(value)
                    && c_int::from(b) == m.kind
                {
                    return Ok(m.map_value);
                }
            }
            GIT_CONFIGMAP_INT32 => {
                if let Ok(n) = parse_int32(value) {
                    return Ok(n);
                }
            }
            GIT_CONFIGMAP_STRING => {
                if let Some(v) = value
                    && crate::util::util::strcasecmp(v, m.str_match.unwrap_or(b"")) == 0
                {
                    return Ok(m.map_value);
                }
            }
            _ => {}
        }
    }
    Err(git_error_set!(
        ErrorClass::CONFIG,
        "failed to map '{}'",
        or_null(value)
    ))
}

/// `git_config__find_global`: the global file (`~/.gitconfig`).
pub fn find_global(path: &mut GitStr) -> Result<()> {
    crate::sys::sysdir::find_global_file(path, GIT_CONFIG_FILENAME_GLOBAL)
}

/// `git_config__find_xdg`: the XDG file (`~/.config/git/config`).
pub fn find_xdg(path: &mut GitStr) -> Result<()> {
    crate::sys::sysdir::find_xdg_file(path, GIT_CONFIG_FILENAME_XDG)
}

/// `git_config__find_system`: the system file.
pub fn find_system(path: &mut GitStr) -> Result<()> {
    crate::sys::sysdir::find_system_file(path, GIT_CONFIG_FILENAME_SYSTEM)
}

/// `git_config__global_location`: the first global search path joined with
/// `.gitconfig` (-1, no message, when there is none).
pub fn global_location(buf: &mut GitStr) -> Result<()> {
    let Some(paths) = crate::sys::sysdir::get(crate::sys::sysdir::GIT_SYSDIR_GLOBAL)? else {
        return Err(ErrorCode::GENERIC);
    };
    if paths.is_empty() {
        return Err(ErrorCode::GENERIC);
    }
    let paths = until_nul(&paths);
    let mut sep = 0;
    while sep < paths.len() {
        if paths[sep] == b':' && (sep == 0 || paths[sep - 1] != b'\\') {
            break;
        }
        sep += 1;
    }
    buf.set(&paths[..sep])?;
    buf.joinpath_self(GIT_CONFIG_FILENAME_GLOBAL.to_bytes())
}

// ---------------------------------------------------------------------------
// Opening configs
// ---------------------------------------------------------------------------

/// `git_config_add_file_ondisk`.
///
/// # Safety
///
/// As [`BackendCalls::open`] for `repo`.
pub unsafe fn add_file_ondisk(
    config: &GitConfig,
    path: &CStr,
    level: c_int,
    repo: *const git_repository,
    force: bool,
) -> Result<()> {
    if let Err(e) = crate::util::posix::stat(path) {
        let errno = e.0;
        if errno != libc::ENOENT && errno != libc::ENOTDIR {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "failed to stat '{}'",
                Bytes(path.to_bytes())
            ));
        }
    }
    let file = crate::config_file::backend_from_file(path.to_bytes())?;
    // SAFETY: `repo` per this function's contract.
    unsafe { config.add_backend(file, level, repo, force) }.map_err(|(e, file)| {
        drop(file);
        e
    })
}

/// [`add_file_ondisk`] with no repository.
pub fn add_file(config: &GitConfig, path: &CStr, level: c_int, force: bool) -> Result<()> {
    // SAFETY: NULL is a valid repository argument.
    unsafe { add_file_ondisk(config, path, level, core::ptr::null(), force) }
}

/// `git_config_open_ondisk`.
pub fn open_ondisk(path: &CStr) -> Result<Box<GitConfig>> {
    let config = GitConfig::new();
    add_file(&config, path, GIT_CONFIG_LEVEL_LOCAL, false)?;
    Ok(config)
}

/// `git_config_open_default`.
pub fn open_default() -> Result<Box<GitConfig>> {
    use crate::sys::sysdir;
    let config = GitConfig::new();
    let mut buf = GitStr::new();
    let mut error: Result<()> = Ok(());
    if sysdir::find_global_file(&mut buf, GIT_CONFIG_FILENAME_GLOBAL).is_ok()
        || global_location(&mut buf).is_ok()
    {
        error = add_file(&config, str_cstr(&buf), GIT_CONFIG_LEVEL_GLOBAL, false);
    }
    if error.is_ok() && sysdir::find_xdg_file(&mut buf, GIT_CONFIG_FILENAME_XDG).is_ok() {
        error = add_file(&config, str_cstr(&buf), GIT_CONFIG_LEVEL_XDG, false);
    }
    if error.is_ok() && sysdir::find_system_file(&mut buf, GIT_CONFIG_FILENAME_SYSTEM).is_ok() {
        error = add_file(&config, str_cstr(&buf), GIT_CONFIG_LEVEL_SYSTEM, false);
    }
    error.map(|()| config)
}

/// A `git_str`'s contents as a C string.
fn str_cstr(s: &GitStr) -> &CStr {
    s.as_c_str().unwrap_or(c"")
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

/// `git_config_rename_section`: rename every variable of
/// `old_section_name` (or delete them, with no new name). A `None`
/// repository fails as `git_repository_config__weakptr`'s argument check.
pub fn rename_section(
    repo: Option<&git_repository>,
    old_section_name: &[u8],
    new_section_name: Option<&[u8]>,
) -> c_int {
    let mut pattern = GitStr::new();
    let mut replace = GitStr::new();
    let _ = pattern.puts_escape_regex(old_section_name);
    if let Err(e) = pattern.puts(b"\\..+") {
        return e.raw();
    }
    let Some(repo) = repo else {
        return errors::assert_arg_failed("repo").raw();
    };
    let config = match repo.config_weak() {
        Ok(c) => c,
        Err(e) => return e.raw(),
    };
    let old_len = old_section_name.len() + 1;
    if let Err(e) = replace.join(b'.', new_section_name, b"") {
        return e.raw();
    }
    if let Some(new) = new_section_name {
        let bytes = replace.as_mut_bytes();
        let end = memchr::memchr(b'.', bytes).unwrap_or(bytes.len());
        if normalize_section(&mut bytes[..end]).is_err() {
            let _ = git_error_set!(
                ErrorClass::CONFIG,
                "invalid config section '{}'",
                Bytes(new)
            );
            return ErrorCode::INVALID_SPEC.raw();
        }
    }
    let base_len = replace.len();
    let pattern = match cstring(pattern.as_bytes()) {
        Ok(p) => p,
        Err(e) => return e.raw(),
    };
    config.foreach_match(Some(as_cstr(&pattern)), |entry| {
        // SAFETY: the entry the iterator holds, live during the callback.
        let e = unsafe { EntryView::of(entry.cast()) };
        let r = rename_entry(config, &mut replace, base_len, old_len, &e);
        replace.truncate(base_len);
        match r {
            Ok(()) => 0,
            Err(code) => code.raw(),
        }
    })
}

/// `rename_config_entries_cb`.
fn rename_entry(
    config: &GitConfig,
    name: &mut GitStr,
    base_len: usize,
    old_len: usize,
    entry: &EntryView<'_>,
) -> Result<()> {
    if base_len > 0 {
        name.puts(entry.name.get(old_len..).unwrap_or(b""))?;
        let n = cstring(name.as_bytes())?;
        let v = entry.value.map(cstring).transpose()?;
        config.set_multivar(as_cstr(&n), Some(c"^$"), v.as_deref().map(as_cstr))?;
    }
    let mut value = GitStr::new();
    let _ = value.putc(b'^');
    if let Some(v) = entry.value {
        let _ = value.puts_escape_regex(v);
    }
    let _ = value.putc(b'$');
    if value.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    let n = cstring(entry.name)?;
    let r = cstring(value.as_bytes())?;
    config
        .delete_multivar(as_cstr(&n), Some(as_cstr(&r)))
        .map(drop)
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `config.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::CStrArg;
    use crate::util::export::barrier;

    /// A NULL-able C string argument as `%s` prints it.
    fn or_null(p: CStrArg<'_>) -> &CStr {
        p.get().unwrap_or(c"(null)")
    }

    /// Write a new config to `out`.
    fn out_config(mut out: Out<'_, *mut GitConfig>, c: Box<GitConfig>) {
        out.write(CShared::into_raw(CShared::from_box(c)));
    }

    /// `git_config_entry_free` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `entry` is NULL or an entry libgit2 handed out, freed once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_entry_free(entry: *mut git_config_entry) {
        barrier(|| {
            if !entry.is_null() {
                // SAFETY: a non-null `entry` is the `entry` of a backend
                // entry libgit2 handed out, freed once (this export's
                // contract).
                unsafe { entry_free(entry) }
            }
        })
    }

    /// `git_config_free` (NULL allowed): drop a reference.
    ///
    /// # Safety
    ///
    /// `config` is NULL or a live config whose reference the caller gives
    /// up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_free(config: *mut GitConfig) {
        // SAFETY: `config` is NULL or a live config made by `CShared`
        // (every `git_config *` is), whose reference the caller gives up
        // (this export's contract).
        barrier(|| unsafe { CShared::release(config) })
    }

    /// `git_config_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_new(out: Out<'_, *mut GitConfig>) -> c_int {
        barrier(|| {
            out_config(out, GitConfig::new());
            0
        })
    }

    /// `git_config_add_file_ondisk`.
    ///
    /// # Safety
    ///
    /// `config` is NULL or live; `path` is NULL or NUL-terminated; `repo` is
    /// NULL or a live repository that outlives the backend.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_add_file_ondisk(
        config: Option<&GitConfig>,
        path: CStrArg<'_>,
        level: c_int,
        repo: *const git_repository,
        force: c_int,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let path = git_arg!(path.get(), "path", -1);
            // SAFETY: `repo` is NULL or live for the backend (the contract).
            match unsafe { add_file_ondisk(config, path, level, repo, force != 0) } {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_open_ondisk`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `path` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_open_ondisk(
        mut out: Out<'_, *mut GitConfig>,
        path: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            out.write(core::ptr::null_mut());
            let path = git_arg!(path.get(), "path", -1);
            match open_ondisk(path) {
                Ok(c) => {
                    out_config(out, c);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_snapshot`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_snapshot(
        mut out: Out<'_, *mut GitConfig>,
        config: Option<&GitConfig>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            out.write(core::ptr::null_mut());
            match config.snapshot() {
                Ok(c) => {
                    out_config(out, c);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_open_level`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `parent` is live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_open_level(
        out: Out<'_, *mut GitConfig>,
        parent: Option<&GitConfig>,
        level: c_int,
    ) -> c_int {
        barrier(|| {
            let parent = git_arg!(parent, "parent", -1);
            match parent.open_level(level) {
                Ok(c) => {
                    out_config(out, c);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_open_global`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_open_global(
        out: Out<'_, *mut GitConfig>,
        config: Option<&GitConfig>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            match config.open_global() {
                Ok(c) => {
                    out_config(out, c);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_add_backend`: on success the config owns `backend`; on
    /// failure the caller still does.
    ///
    /// # Safety
    ///
    /// `config` is NULL or live; `backend` is NULL or a valid backend table
    /// (sys/config.h), not owned by anything else; `repo` as for
    /// `git_config_add_file_ondisk`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_add_backend(
        config: Option<&GitConfig>,
        backend: *mut git_config_backend,
        level: c_int,
        repo: *const git_repository,
        force: c_int,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            git_assert_arg!(!backend.is_null(), "backend", -1);
            // SAFETY: `backend` is a valid table that nothing else owns
            // (this export's contract), released by its own `free` slot,
            // which is `Backend`'s release.
            let Some(b) = (unsafe { Backend::from_c(backend.cast()) }) else {
                return -1;
            };
            // SAFETY: `repo` is NULL or live for the backend (the contract).
            match unsafe { config.add_backend(b, level, repo, force != 0) } {
                Ok(()) => 0,
                Err((e, b)) => {
                    // Not ours: the caller frees it.
                    let _ = b.into_c();
                    e.raw()
                }
            }
        })
    }

    /// `git_config_set_writeorder`.
    ///
    /// # Safety
    ///
    /// `config` is live; `levels` is readable for `len` values, unwritten
    /// during the call (or `len` is 0).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_writeorder(
        config: Option<&GitConfig>,
        levels: *mut c_int,
        len: usize,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            git_assert!(len < c_int::MAX as usize, "len < INT_MAX", -1);
            let levels = if len == 0 {
                &[][..]
            } else {
                // SAFETY: `levels` is aligned and readable for `len` ints,
                // which nothing writes during the call (this export's
                // contract); `len < INT_MAX` (asserted above), so the
                // slice is shorter than `isize::MAX` bytes.
                unsafe { core::slice::from_raw_parts(levels, len) }
            };
            match config.set_writeorder(levels) {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_iterator_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live (the iterator keeps its backends
    /// alive by itself).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_iterator_new(
        mut out: Out<'_, *mut git_config_iterator>,
        config: Option<&GitConfig>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            match config.iterator(None) {
                Ok(it) => {
                    out.write(it.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_iterator_glob_new`.
    ///
    /// # Safety
    ///
    /// As `git_config_iterator_new`; `regexp` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_iterator_glob_new(
        mut out: Out<'_, *mut git_config_iterator>,
        config: Option<&GitConfig>,
        regexp: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let regexp = regexp.get();
            match config.iterator(regexp) {
                Ok(it) => {
                    out.write(it.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_multivar_iterator_new`.
    ///
    /// # Safety
    ///
    /// As `git_config_iterator_glob_new`; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_multivar_iterator_new(
        mut out: Out<'_, *mut git_config_iterator>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        regexp: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let name = git_arg!(name.get(), "in", -1);
            let config = git_arg!(config, "config", -1);
            let regexp = regexp.get();
            match config.multivar_iterator(name, regexp) {
                Ok(it) => {
                    out.write(it.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_next`.
    ///
    /// # Safety
    ///
    /// `entry` is writable; `iter` is a live iterator that no other thread
    /// uses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_next(
        mut entry: Out<'_, *mut git_config_entry>,
        iter: *mut git_config_iterator,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `iter` is a live, aligned iterator (this export's
            // contract); the slot is copied through the raw pointer.
            let Some(next) = (unsafe { (*iter).next }) else {
                return no_slot("next").raw();
            };
            let mut be = core::ptr::null_mut();
            // SAFETY: `next` is the iterator's own slot, called with a local
            // out-parameter and the live iterator, which only this call uses
            // (this export's contract; the slot's, sys/config.h).
            let error = unsafe { next(&raw mut be, iter) };
            if error != 0 {
                return error;
            }
            entry.write(be.cast());
            0
        })
    }

    /// `git_config_iterator_free` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `iter` is NULL or a live iterator, freed once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_iterator_free(iter: *mut git_config_iterator) {
        barrier(|| {
            // SAFETY: `iter` is NULL or a live iterator libgit2 handed out,
            // freed once (this export's contract): ours to release.
            drop(unsafe { IterBox::from_raw(iter) });
        })
    }

    /// The C callback of the foreach functions as a closure.
    type ForeachCb = Option<unsafe extern "C" fn(*const git_config_entry, *mut c_void) -> c_int>;

    /// Call a foreach callback: C calls it unchecked (a NULL one crashes at
    /// the first entry); a NULL one fails as `GIT_ASSERT_ARG(cb)` here.
    ///
    /// # Safety
    ///
    /// `cb` is NULL or valid for `entry` and `payload`.
    unsafe fn call(cb: ForeachCb, entry: *const git_config_entry, payload: *mut c_void) -> c_int {
        match cb {
            // SAFETY: `f` may be called with `entry` and `payload` (this
            // function's contract).
            Some(f) => unsafe { f(entry, payload) },
            None => errors::assert_arg_failed("cb").raw(),
        }
    }

    /// `git_config_foreach`.
    ///
    /// # Safety
    ///
    /// `config` is live; `cb` is a valid callback for `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_foreach(
        config: Option<&GitConfig>,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: this export's contract, with a NULL pattern.
        unsafe { git_config_foreach_match(config, CStrArg::NULL, cb, payload) }
    }

    /// `git_config_foreach_match`.
    ///
    /// # Safety
    ///
    /// As `git_config_foreach`; `regexp` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_foreach_match(
        config: Option<&GitConfig>,
        regexp: CStrArg<'_>,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let regexp = regexp.get();
            config.foreach_match(regexp, |e| {
                // SAFETY: `cb` is NULL or a callback valid for `payload`
                // (this export's contract), given the entry the iterator
                // holds, live for the call.
                unsafe { call(cb, e, payload) }
            })
        })
    }

    /// `git_config_backend_foreach_match`.
    ///
    /// # Safety
    ///
    /// `backend` is NULL or a live backend, not freed during the call; the
    /// rest as `git_config_foreach_match`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_foreach_match(
        backend: *mut git_config_backend,
        regexp: CStrArg<'_>,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let regexp = regexp.get();
            let Some(backend) = NonNull::new(backend) else {
                return errors::assert_arg_failed("backend").raw();
            };
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            // SAFETY: `backend` is a live table for the call (this
            // export's contract), and `b` does not outlive the call.
            let b = unsafe { BackendPtr::new(backend) };
            backend_foreach_match(&b, regexp, |e| {
                // SAFETY: `cb` is valid for `payload` (this export's
                // contract), given the entry the iterator holds.
                unsafe { cb(e, payload) }
            })
        })
    }

    /// `git_config_get_multivar_foreach`.
    ///
    /// # Safety
    ///
    /// As `git_config_foreach_match`; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_multivar_foreach(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        regexp: CStrArg<'_>,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let name = git_arg!(name.get(), "in", -1);
            let config = git_arg!(config, "config", -1);
            let regexp = regexp.get();
            config.get_multivar_foreach(name, regexp, |e| {
                // SAFETY: `cb` is NULL or a callback valid for `payload`
                // (this export's contract), given the entry the iterator
                // holds.
                unsafe { call(cb, e, payload) }
            })
        })
    }

    /// `git_config_delete_entry`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_delete_entry(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            match config.delete_entry(name) {
                Ok(r) => r,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_set_int64`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_int64(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        value: i64,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            crate::util::export::status(config.set_int64(name, value))
        })
    }

    /// `git_config_set_int32`.
    ///
    /// # Safety
    ///
    /// As `git_config_set_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_int32(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        value: i32,
    ) -> c_int {
        // SAFETY: this export's contract is `git_config_set_int64`'s.
        unsafe { git_config_set_int64(config, name, i64::from(value)) }
    }

    /// `git_config_set_bool`.
    ///
    /// # Safety
    ///
    /// As `git_config_set_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_bool(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        value: c_int,
    ) -> c_int {
        let v = if value != 0 { c"true" } else { c"false" };
        // SAFETY: this export's contract, with a static string value.
        unsafe { git_config_set_string(config, name, CStrArg::from(v)) }
    }

    /// `git_config_set_string`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated; `value` is NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_string(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            let value = value.get();
            let r = config.set_string(name, value);
            crate::util::export::status(r)
        })
    }

    /// `git_config_set_multivar`.
    ///
    /// # Safety
    ///
    /// `config` is live; the strings are NULL or NUL-terminated (the file
    /// backend requires `name` and `regexp`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_multivar(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        regexp: CStrArg<'_>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            let regexp = regexp.get();
            let value = value.get();
            match config.set_multivar(name, regexp, value) {
                Ok(r) => r,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_delete_multivar`.
    ///
    /// # Safety
    ///
    /// As `git_config_set_multivar`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_delete_multivar(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        regexp: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            let regexp = regexp.get();
            match config.delete_multivar(name, regexp) {
                Ok(r) => r,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config__update_entry`.
    ///
    /// # Safety
    ///
    /// `config` is live; `key` is NUL-terminated; `value` is NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__update_entry(
        config: Option<&GitConfig>,
        key: CStrArg<'_>,
        value: CStrArg<'_>,
        overwrite_existing: bool,
        only_if_existing: bool,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let key = or_null(key);
            let value = value.get();
            let r = config.update_entry(key, value, overwrite_existing, only_if_existing);
            crate::util::export::status(r)
        })
    }

    /// Write an entry (or NULL) to `out`, with `get_entry`'s result.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    fn out_entry(mut out: Out<'_, *mut git_config_entry>, r: Result<Option<EntryBox>>) -> c_int {
        let (p, code) = match r {
            Ok(Some(e)) => (e.into_raw().cast(), 0),
            Ok(None) => (core::ptr::null_mut(), 0),
            Err(e) => (core::ptr::null_mut(), e.raw()),
        };
        out.write(p);
        code
    }

    /// `git_config_get_entry`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_entry(
        out: Out<'_, *mut git_config_entry>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            out_entry(out, config.get_entry(name, true, Want::AllErrors))
        })
    }

    /// `git_config__lookup_entry`: no normalization; a missing entry is
    /// NULL; with `no_errors`, any failure is.
    ///
    /// # Safety
    ///
    /// As `git_config_get_entry`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__lookup_entry(
        out: Out<'_, *mut git_config_entry>,
        config: Option<&GitConfig>,
        key: CStrArg<'_>,
        no_errors: bool,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let key = or_null(key);
            let want = if no_errors {
                Want::NoErrors
            } else {
                Want::NoMissing
            };
            out_entry(out, config.get_entry(key, false, want))
        })
    }

    /// The value of the entry `name` (normalized), with `f`; a NULL
    /// `config` (which the C dereferences) is an invalid argument.
    fn with_value<T>(
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        f: impl FnOnce(Option<&[u8]>) -> Result<T>,
    ) -> Result<T> {
        let config = git_arg!(config, "config");
        config.with_value(or_null(name), f)
    }

    /// `git_config_get_mapped`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `name` is NUL-terminated;
    /// `maps` is readable for `map_n` maps whose strings are NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_mapped(
        mut out: Out<'_, c_int>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
        maps: *const git_configmap,
        map_n: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `maps` is readable for `map_n` maps whose strings are
            // NULL or NUL-terminated, unwritten during the call (this
            // export's contract), which is `configmaps`'.
            let maps = unsafe { configmaps(maps, map_n) };
            match with_value(config, name, |v| lookup_map_value(&maps, v)) {
                Ok(v) => {
                    out.write(v);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// The C maps as Rust ones.
    ///
    /// # Safety
    ///
    /// `maps` is aligned and readable for `n` maps (or `n` is 0) whose
    /// strings are NULL or NUL-terminated, all unwritten for `'a`.
    unsafe fn configmaps<'a>(maps: *const git_configmap, n: usize) -> Vec<RawConfigMap<'a>> {
        if n == 0 {
            return Vec::new();
        }
        // SAFETY: `maps` is aligned and readable for `n` maps, one C array
        // (so at most `isize::MAX` bytes), unwritten for `'a` (this
        // function's contract).
        let maps = unsafe { core::slice::from_raw_parts(maps, n) };
        maps.iter()
            .map(|m| RawConfigMap {
                kind: m.type_,
                // SAFETY: each `str_match` is NULL or a NUL-terminated
                // string, unwritten for `'a` (this function's contract).
                str_match: unsafe { CStrArg::from_ptr(m.str_match) }
                    .get()
                    .map(CStr::to_bytes),
                map_value: m.map_value,
            })
            .collect()
    }

    /// `git_config_get_int64`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_int64(
        mut out: Out<'_, i64>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| match with_value(config, name, parse_int64) {
            Ok(v) => {
                out.write(v);
                0
            }
            Err(e) => e.raw(),
        })
    }

    /// `git_config_get_int32`.
    ///
    /// # Safety
    ///
    /// As `git_config_get_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_int32(
        mut out: Out<'_, i32>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| match with_value(config, name, parse_int32) {
            Ok(v) => {
                out.write(v);
                0
            }
            Err(e) => e.raw(),
        })
    }

    /// `git_config_get_bool`.
    ///
    /// # Safety
    ///
    /// As `git_config_get_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_bool(
        mut out: Out<'_, c_int>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| match with_value(config, name, parse_bool) {
            Ok(v) => {
                out.write(c_int::from(v));
                0
            }
            Err(e) => e.raw(),
        })
    }

    /// `git_config__get_path`.
    ///
    /// # Safety
    ///
    /// `out` is NULL (an error, as C's assertion) or a valid `git_str` that
    /// nothing else accesses during the call; `config` is live; `name` is
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_path(
        out: Option<&mut GitStr>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let parse = |v: Option<&[u8]>| {
                // `GIT_ASSERT_ARG(out)` and `(value)` in
                // `git_config__parse_path`, after the lookup, as C.
                let out = git_arg!(out, "out");
                let Some(v) = v else {
                    return Err(errors::assert_arg_failed("value"));
                };
                parse_path(out, v)
            };
            let r = with_value(config, name, parse);
            crate::util::export::status(r)
        })
    }

    /// `git_config_get_path`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf` that nothing else accesses during
    /// the call; the rest as `git_config__get_path`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_path(
        out: Option<&mut GitBuf>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let get = |s: &mut GitStr| {
                // SAFETY: `s` is a valid `git_str` that only this call
                // reaches; `config` and `name` are what
                // `git_config__get_path` requires (this export's contract).
                ErrorCode::check_zero(unsafe { git_config__get_path(Some(s), config, name) })
            };
            crate::buf::wrap(out, get)
        })
    }

    /// `git_config_parse_path`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf` that nothing else accesses during
    /// the call; `value` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_parse_path(
        out: Option<&mut GitBuf>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let value = value.get();
            crate::buf::wrap(out, |s| {
                let Some(v) = value else {
                    return Err(errors::assert_arg_failed("value"));
                };
                parse_path(s, v.to_bytes())
            })
        })
    }

    /// `git_config_get_string`: only on a read-only config (a snapshot),
    /// whose entries outlive the call.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_string(
        mut out: Out<'_, *const c_char>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let name = or_null(name);
            if !config.is_readonly() {
                return git_error_set!(
                    ErrorClass::CONFIG,
                    "get_string called on a live config object"
                )
                .raw();
            }
            let r = config.get_entry(name, true, Want::AllErrors);
            let (p, code) = match &r {
                Ok(Some(e)) => {
                    // SAFETY: `e` owns a live, aligned entry; the field is
                    // copied through the raw pointer.
                    let v = unsafe { (*e.as_entry()).value };
                    (if v.is_null() { c"".as_ptr() } else { v }, 0)
                }
                Ok(None) => (core::ptr::null(), ErrorCode::NOT_FOUND.raw()),
                Err(e) => (core::ptr::null(), e.raw()),
            };
            out.write(p);
            drop(r);
            code
        })
    }

    /// `git_config__get_string_buf`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_str`; `config` is NULL or live; `name`
    /// is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_string_buf(
        out: Option<&mut GitStr>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let out = git_arg!(out, "out", -1);
            git_assert_arg!(config.is_some(), "config", -1);
            let r = with_value(config, name, |v| out.puts(v.unwrap_or(b"")));
            crate::util::export::status(r)
        })
    }

    /// `git_config_get_string_buf`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf` that nothing else accesses during
    /// the call; the rest as `git_config__get_string_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_string_buf(
        out: Option<&mut GitBuf>,
        config: Option<&GitConfig>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let get = |s: &mut GitStr| {
                // SAFETY: `s` is a valid `git_str` that only this call
                // reaches; `config` and `name` are what
                // `git_config__get_string_buf` requires (this export's
                // contract).
                ErrorCode::check_zero(unsafe { git_config__get_string_buf(Some(s), config, name) })
            };
            crate::buf::wrap(out, get)
        })
    }

    /// `git_config__get_string_force`: a copy of the value (or of
    /// `fallback_value`), `git__malloc`ed; NULL for neither.
    ///
    /// # Safety
    ///
    /// `config` is live; `key` is NUL-terminated; `fallback_value` is NULL
    /// or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_string_force(
        config: Option<&GitConfig>,
        key: CStrArg<'_>,
        fallback_value: CStrArg<'_>,
    ) -> *mut c_char {
        barrier(|| {
            let config = git_arg!(config, "config", core::ptr::null_mut());
            let key = or_null(key);
            let fallback_value = fallback_value.get();
            let e = config.get_entry(key, false, Want::NoErrors);
            let e = e.ok().flatten();
            let v = e.as_ref().and_then(|e| e.value().map(<[u8]>::to_vec));
            let v = v.or_else(|| fallback_value.map(|f| f.to_bytes().to_vec()));
            match v {
                Some(v) => crate::util::alloc::substrdup(&v),
                None => core::ptr::null_mut(),
            }
        })
    }

    /// `git_config__get_bool_force`.
    ///
    /// # Safety
    ///
    /// `config` is live; `key` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_bool_force(
        config: Option<&GitConfig>,
        key: CStrArg<'_>,
        fallback_value: c_int,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let key = or_null(key);
            let e = config.get_entry(key, false, Want::NoErrors);
            let mut val = fallback_value;
            if let Ok(Some(e)) = e {
                match parse_bool(e.value()) {
                    Ok(b) => val = c_int::from(b),
                    Err(_) => errors::clear(),
                }
            }
            val
        })
    }

    /// `git_config__get_int_force`.
    ///
    /// # Safety
    ///
    /// As `git_config__get_bool_force`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_int_force(
        config: Option<&GitConfig>,
        key: CStrArg<'_>,
        fallback_value: c_int,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            let key = or_null(key);
            let e = config.get_entry(key, false, Want::NoErrors);
            let mut val = fallback_value;
            if let Ok(Some(e)) = e {
                match parse_int32(e.value()) {
                    Ok(n) => val = n,
                    Err(_) => errors::clear(),
                }
            }
            val
        })
    }

    /// `git_config_lookup_map_value`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `maps` as for `git_config_get_mapped`; `value` is
    /// NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_lookup_map_value(
        mut out: Out<'_, c_int>,
        maps: *const git_configmap,
        map_n: usize,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let value = value.get();
            // SAFETY: `maps` is as `configmaps` requires (this export's
            // contract, as `git_config_get_mapped`'s).
            let maps = unsafe { configmaps(maps, map_n) };
            match lookup_map_value(&maps, value.map(CStr::to_bytes)) {
                Ok(v) => {
                    out.write(v);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_lookup_map_enum`.
    ///
    /// # Safety
    ///
    /// `type_out` and `str_out` are writable; `maps` is readable for
    /// `map_n` maps.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_lookup_map_enum(
        mut type_out: Out<'_, c_int>,
        mut str_out: Out<'_, *const c_char>,
        maps: *const git_configmap,
        map_n: usize,
        enum_val: c_int,
    ) -> c_int {
        barrier(|| {
            let maps = if map_n == 0 {
                &[][..]
            } else {
                // SAFETY: `maps` is aligned and readable for `map_n` maps,
                // one C array (at most `isize::MAX` bytes), unwritten during
                // the call (this export's contract).
                unsafe { core::slice::from_raw_parts(maps, map_n) }
            };
            match maps.iter().find(|m| m.map_value == enum_val) {
                Some(m) => {
                    type_out.write(m.type_);
                    str_out.write(m.str_match);
                    0
                }
                None => {
                    let _ = git_error_set!(ErrorClass::CONFIG, "invalid enum value");
                    ErrorCode::NOT_FOUND.raw()
                }
            }
        })
    }

    /// `git_config_parse_bool`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `value` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_parse_bool(
        mut out: Out<'_, c_int>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let value = value.get();
            match parse_bool(value.map(CStr::to_bytes)) {
                Ok(b) => {
                    out.write(c_int::from(b));
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_parse_int64`.
    ///
    /// # Safety
    ///
    /// As `git_config_parse_bool`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_parse_int64(
        mut out: Out<'_, i64>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let value = value.get();
            match parse_int64(value.map(CStr::to_bytes)) {
                Ok(v) => {
                    out.write(v);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_parse_int32`.
    ///
    /// # Safety
    ///
    /// As `git_config_parse_bool`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_parse_int32(
        mut out: Out<'_, i32>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let value = value.get();
            match parse_int32(value.map(CStr::to_bytes)) {
                Ok(v) => {
                    out.write(v);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config__normalize_name`: `*out` is `git__malloc`ed.
    ///
    /// # Safety
    ///
    /// `input` is NULL or NUL-terminated; `out` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__normalize_name(
        input: CStrArg<'_>,
        mut out: Out<'_, *mut c_char>,
    ) -> c_int {
        barrier(|| {
            let input = git_arg!(input.get(), "in", -1);
            git_assert_arg!(!out.is_null(), "out", -1);
            match normalize_name(input.to_bytes()) {
                Ok(n) => {
                    let s = crate::util::alloc::substrdup(&n);
                    if s.is_null() {
                        return -1;
                    }
                    out.write(s);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_find_global`.
    ///
    /// # Safety
    ///
    /// `path` is NULL or a valid `git_buf` that nothing else accesses
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_global(path: Option<&mut GitBuf>) -> c_int {
        barrier(|| {
            crate::buf::wrap(path, |s| {
                crate::sys::sysdir::find_global_file(s, GIT_CONFIG_FILENAME_GLOBAL)
            })
        })
    }

    /// `git_config__find_global`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_global(path: Option<&mut GitStr>) -> c_int {
        barrier(|| {
            let path = git_arg!(path, "path", -1);
            crate::util::export::status(find_global(path))
        })
    }

    /// `git_config_find_xdg`.
    ///
    /// # Safety
    ///
    /// `path` is NULL or a valid `git_buf` that nothing else accesses
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_xdg(path: Option<&mut GitBuf>) -> c_int {
        barrier(|| {
            crate::buf::wrap(path, |s| {
                crate::sys::sysdir::find_xdg_file(s, GIT_CONFIG_FILENAME_XDG)
            })
        })
    }

    /// `git_config__find_xdg`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_xdg(path: Option<&mut GitStr>) -> c_int {
        barrier(|| {
            let path = git_arg!(path, "path", -1);
            crate::util::export::status(find_xdg(path))
        })
    }

    /// `git_config_find_system`.
    ///
    /// # Safety
    ///
    /// `path` is NULL or a valid `git_buf` that nothing else accesses
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_system(path: Option<&mut GitBuf>) -> c_int {
        barrier(|| {
            crate::buf::wrap(path, |s| {
                crate::sys::sysdir::find_system_file(s, GIT_CONFIG_FILENAME_SYSTEM)
            })
        })
    }

    /// `git_config__find_system`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_system(path: Option<&mut GitStr>) -> c_int {
        barrier(|| {
            let path = git_arg!(path, "path", -1);
            crate::util::export::status(find_system(path))
        })
    }

    /// `git_config__global_location`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid (non-NULL) `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__global_location(buf: Option<&mut GitStr>) -> c_int {
        barrier(|| {
            let buf = git_arg!(buf, "buf", -1);
            match global_location(buf) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    /// `git_config_open_default`.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_open_default(mut out: Out<'_, *mut GitConfig>) -> c_int {
        barrier(|| {
            let (p, code) = match open_default() {
                Ok(c) => (CShared::into_raw(CShared::from_box(c)), 0),
                Err(e) => (core::ptr::null_mut(), e.raw()),
            };
            out.write(p);
            code
        })
    }

    /// `git_config_lock`.
    ///
    /// # Safety
    ///
    /// `tx` is writable; `config` is NULL or live, and outlives the
    /// transaction.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_lock(
        tx: Out<'_, *mut crate::sys::transaction::git_transaction>,
        config: Option<&GitConfig>,
    ) -> c_int {
        barrier(|| {
            let config = git_arg!(config, "config", -1);
            crate::util::export::status(config.lock(tx))
        })
    }

    /// `git_config_unlock`: unlock the backend `data` names (from
    /// `git_config_lock`), committing its changes if `commit`.
    ///
    /// # Safety
    ///
    /// `data` is NULL or the instance `git_config_lock` gave the
    /// transaction, released once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_unlock(
        config: *mut GitConfig,
        data: *mut c_void,
        commit: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!config.is_null() && !data.is_null(), "config && data", -1);
            // SAFETY: a non-null `data` is the `Arc::into_raw` reference
            // `git_config_lock` gave the transaction, released once (this
            // export's contract): taken back here.
            let instance = unsafe { Arc::from_raw(data.cast_const().cast::<Instance>()) };
            let r = instance.backend.unlock(commit);
            drop(instance);
            match r {
                Ok(v) => v,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_rename_section`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository, not freed during the call;
    /// `old_section_name` is NULL or NUL-terminated; `new_section_name` is
    /// NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_rename_section(
        repo: Option<&git_repository>,
        old_section_name: CStrArg<'_>,
        new_section_name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let new_section_name = new_section_name.get();
            let old = old_section_name.to_bytes();
            rename_section(repo, old, new_section_name.map(CStr::to_bytes))
        })
    }

    /// `git_config_init_backend`.
    ///
    /// # Safety
    ///
    /// `backend` is writable for a `git_config_backend`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_init_backend(
        mut backend: Out<'_, git_config_backend>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            if let Err(e) = check_version(version, GIT_CONFIG_BACKEND_VERSION, "git_config_backend")
            {
                return e.raw();
            }
            backend.write(git_config_backend::default());
            0
        })
    }
}

// ---------------------------------------------------------------------------
// The safe Rust API
// ---------------------------------------------------------------------------

/// A configuration level (`git_config_level_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigLevel {
    /// `GIT_CONFIG_LEVEL_SYSTEM`.
    System,
    /// `GIT_CONFIG_LEVEL_XDG`.
    Xdg,
    /// `GIT_CONFIG_LEVEL_GLOBAL`.
    Global,
    /// `GIT_CONFIG_LEVEL_LOCAL`.
    Local,
    /// `GIT_CONFIG_LEVEL_WORKTREE`.
    Worktree,
    /// `GIT_CONFIG_LEVEL_APP`.
    App,
    /// `GIT_CONFIG_HIGHEST_LEVEL`.
    Highest,
    /// Another level (an application's own, above `App`).
    Other(i32),
}

impl ConfigLevel {
    /// The C value.
    pub(crate) fn raw(self) -> c_int {
        match self {
            ConfigLevel::System => GIT_CONFIG_LEVEL_SYSTEM,
            ConfigLevel::Xdg => GIT_CONFIG_LEVEL_XDG,
            ConfigLevel::Global => GIT_CONFIG_LEVEL_GLOBAL,
            ConfigLevel::Local => GIT_CONFIG_LEVEL_LOCAL,
            ConfigLevel::Worktree => GIT_CONFIG_LEVEL_WORKTREE,
            ConfigLevel::App => GIT_CONFIG_LEVEL_APP,
            ConfigLevel::Highest => GIT_CONFIG_HIGHEST_LEVEL,
            ConfigLevel::Other(n) => n,
        }
    }

    /// From the C value.
    pub(crate) fn from_raw(n: c_int) -> ConfigLevel {
        match n {
            GIT_CONFIG_LEVEL_SYSTEM => ConfigLevel::System,
            GIT_CONFIG_LEVEL_XDG => ConfigLevel::Xdg,
            GIT_CONFIG_LEVEL_GLOBAL => ConfigLevel::Global,
            GIT_CONFIG_LEVEL_LOCAL => ConfigLevel::Local,
            GIT_CONFIG_LEVEL_WORKTREE => ConfigLevel::Worktree,
            GIT_CONFIG_LEVEL_APP => ConfigLevel::App,
            GIT_CONFIG_HIGHEST_LEVEL => ConfigLevel::Highest,
            n => ConfigLevel::Other(n),
        }
    }
}

/// A configuration entry (`git_config_entry`), copied out of its backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigEntry {
    name: Vec<u8>,
    value: Option<Vec<u8>>,
    backend_type: Option<Vec<u8>>,
    origin_path: Option<Vec<u8>>,
    include_depth: u32,
    level: ConfigLevel,
}

impl ConfigEntry {
    /// An entry, for a [`ConfigBackend`] to return.
    pub fn new(
        name: impl Into<Vec<u8>>,
        value: Option<Vec<u8>>,
        level: ConfigLevel,
    ) -> ConfigEntry {
        ConfigEntry {
            name: name.into(),
            value,
            backend_type: None,
            origin_path: None,
            include_depth: 0,
            level,
        }
    }

    /// The entry with this backend type.
    pub fn with_backend_type(mut self, t: impl Into<Vec<u8>>) -> ConfigEntry {
        self.backend_type = Some(t.into());
        self
    }

    /// The entry with this origin.
    pub fn with_origin_path(mut self, p: impl Into<Vec<u8>>) -> ConfigEntry {
        self.origin_path = Some(p.into());
        self
    }

    fn from_view(v: &EntryView<'_>) -> ConfigEntry {
        ConfigEntry {
            name: v.name.to_vec(),
            value: v.value.map(<[u8]>::to_vec),
            backend_type: v.backend_type.map(<[u8]>::to_vec),
            origin_path: v.origin_path.map(<[u8]>::to_vec),
            include_depth: v.include_depth,
            level: ConfigLevel::from_raw(v.level),
        }
    }

    /// The normalized name.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// The name as UTF-8, if it is.
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.name).ok()
    }

    /// The value (`None` for a bare name, which is true as a boolean).
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// The value as UTF-8, if there is one and it is.
    pub fn value_str(&self) -> Option<&str> {
        self.value
            .as_deref()
            .and_then(|v| core::str::from_utf8(v).ok())
    }

    /// The type of the backend it is in (`"file"`, `"in-memory"`, …).
    pub fn backend_type(&self) -> Option<&[u8]> {
        self.backend_type.as_deref()
    }

    /// The path of the file it was read from.
    pub fn origin_path(&self) -> Option<&[u8]> {
        self.origin_path.as_deref()
    }

    /// How deep in includes it was found.
    pub fn include_depth(&self) -> u32 {
        self.include_depth
    }

    /// The level of its backend.
    pub fn level(&self) -> ConfigLevel {
        self.level
    }
}

/// What a [`ConfigMap`] matches (`git_configmap_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigMapKind {
    /// A false boolean (`GIT_CONFIGMAP_FALSE`).
    False,
    /// A true boolean (`GIT_CONFIGMAP_TRUE`).
    True,
    /// An integer, which is the result (`GIT_CONFIGMAP_INT32`).
    Int32,
    /// The string `str_match`, ignoring case (`GIT_CONFIGMAP_STRING`).
    String,
}

/// One mapping of [`Config::get_mapped`] (`git_configmap`): a value that
/// matches `kind` (and `str_match`) maps to `map_value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigMap<'a> {
    /// What the value must be.
    pub kind: ConfigMapKind,
    /// The string to match, for [`ConfigMapKind::String`].
    pub str_match: Option<&'a [u8]>,
    /// The result (ignored for [`ConfigMapKind::Int32`]).
    pub map_value: i32,
}

impl<'a> ConfigMap<'a> {
    /// A false boolean maps to `value`.
    pub const fn falsy(value: i32) -> ConfigMap<'a> {
        ConfigMap {
            kind: ConfigMapKind::False,
            str_match: None,
            map_value: value,
        }
    }

    /// A true boolean (or a bare name) maps to `value`.
    pub const fn truthy(value: i32) -> ConfigMap<'a> {
        ConfigMap {
            kind: ConfigMapKind::True,
            str_match: None,
            map_value: value,
        }
    }

    /// An integer maps to itself.
    pub const fn int32() -> ConfigMap<'a> {
        ConfigMap {
            kind: ConfigMapKind::Int32,
            str_match: None,
            map_value: 0,
        }
    }

    /// The string `s` (ignoring case) maps to `value`.
    pub const fn string(s: &'a [u8], value: i32) -> ConfigMap<'a> {
        ConfigMap {
            kind: ConfigMapKind::String,
            str_match: Some(s),
            map_value: value,
        }
    }

    fn raw(&self) -> RawConfigMap<'a> {
        RawConfigMap {
            kind: match self.kind {
                ConfigMapKind::False => GIT_CONFIGMAP_FALSE,
                ConfigMapKind::True => GIT_CONFIGMAP_TRUE,
                ConfigMapKind::Int32 => GIT_CONFIGMAP_INT32,
                ConfigMapKind::String => GIT_CONFIGMAP_STRING,
            },
            str_match: self.str_match,
            map_value: self.map_value,
        }
    }
}

type ApiResult<T> = core::result::Result<T, Error>;

/// A NUL-terminated copy of a Rust string argument (`invalid argument` if
/// it holds a NUL, which C could not pass).
fn arg(s: &[u8]) -> ApiResult<Vec<u8>> {
    if s.contains(&0) {
        return Err(Error::with(
            ErrorCode::INVALID,
            ErrorClass::INVALID,
            "a config string argument contains a NUL byte",
        ));
    }
    cstring(s).map_err(Error::from)
}

/// A path argument as bytes.
fn path_bytes(p: &std::path::Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes()
}

/// A path the library made, as a `PathBuf` (lossless on Unix).
fn to_path(s: &GitStr) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(s.as_bytes()))
}

/// A configuration (`git_config`): backends by level. `Clone` takes another
/// reference to the same configuration.
///
/// Names are bytes (`impl AsRef<[u8]>`: `&str` works), as are values; a
/// NUL inside one is refused.
///
/// `'r` is the repository the configuration came from (`'static` for one
/// made alone): a repository's file backends keep the repository for their
/// include conditions, so the configuration (and its clones, levels and
/// snapshots) cannot outlive it.
///
/// ```
/// # #[cfg(not(miri))] {
/// use libgit2_rs::{Config, ConfigLevel};
/// let c = Config::new();
/// c.add_memory(b"[core]\n\tbare = true\n", ConfigLevel::Local, false)?;
/// assert!(c.get_bool("core.bare")?);
/// # }
/// # Ok::<(), libgit2_rs::Error>(())
/// ```
#[derive(Clone)]
pub struct Config<'r>(CShared<GitConfig>, core::marker::PhantomData<&'r ()>);

impl core::fmt::Debug for Config<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Config").finish_non_exhaustive()
    }
}

impl<'r> Config<'r> {
    /// A handle of a new config (its one reference).
    pub(crate) fn from_box(b: Box<GitConfig>) -> Config<'r> {
        Config(CShared::from_box(b), core::marker::PhantomData)
    }

    /// The config (crate-internal: the repository calls it directly).
    pub(crate) fn inner(&self) -> &GitConfig {
        &self.0
    }

    /// A new reference to `raw` (`GIT_REFCOUNT_INC`).
    ///
    /// # Safety
    ///
    /// `raw` is a live config.
    pub(crate) unsafe fn new_ref(raw: NonNull<GitConfig>) -> Config<'r> {
        // SAFETY: `raw` is a live config (this function's contract), made
        // by `CShared` as every `git_config` is.
        let c = unsafe { CShared::from_borrowed(raw.as_ptr()) }.expect("non-null");
        Config(c, core::marker::PhantomData)
    }

    /// The handle's reference.
    pub(crate) fn into_shared(self) -> CShared<GitConfig> {
        self.0
    }

    /// Give this handle's reference up, as a `git_config *`.
    pub(crate) fn into_raw(self) -> NonNull<GitConfig> {
        // SAFETY: a `CShared`'s pointer is not NULL.
        unsafe { NonNull::new_unchecked(CShared::into_raw(self.0)) }
    }

    /// `git_config_new`: an empty configuration.
    pub fn new() -> Config<'static> {
        // Making the configuration is plain Rust, which cannot fail; the
        // library is initialized for what is done with it next (an init
        // that failed fails again, and is reported, there).
        let _ = crate::libgit2::ensure_init();
        Config::from_box(GitConfig::new())
    }

    /// `git_config_open_ondisk`: the file at `path`, at the local level.
    pub fn open(path: impl AsRef<std::path::Path>) -> ApiResult<Config<'static>> {
        crate::libgit2::ensure_init()?;
        let p = arg(path_bytes(path.as_ref()))?;
        Ok(Config::from_box(open_ondisk(as_cstr(&p))?))
    }

    /// `git_config_open_default`: the global, XDG and system files.
    pub fn open_default() -> ApiResult<Config<'static>> {
        crate::libgit2::ensure_init()?;
        Ok(Config::from_box(open_default()?))
    }

    /// `git_config_find_global`: the global file (`~/.gitconfig`), if it
    /// exists ([`ErrorCode::NOT_FOUND`] otherwise).
    pub fn find_global() -> ApiResult<std::path::PathBuf> {
        crate::libgit2::ensure_init()?;
        let mut s = GitStr::new();
        find_global(&mut s)?;
        Ok(to_path(&s))
    }

    /// `git_config_find_xdg`: the XDG file (`~/.config/git/config`), if it
    /// exists.
    pub fn find_xdg() -> ApiResult<std::path::PathBuf> {
        crate::libgit2::ensure_init()?;
        let mut s = GitStr::new();
        find_xdg(&mut s)?;
        Ok(to_path(&s))
    }

    /// `git_config_find_system`: the system file, if it exists.
    pub fn find_system() -> ApiResult<std::path::PathBuf> {
        crate::libgit2::ensure_init()?;
        let mut s = GitStr::new();
        find_system(&mut s)?;
        Ok(to_path(&s))
    }

    /// `git_config_add_file_ondisk`.
    pub fn add_file(
        &self,
        path: impl AsRef<std::path::Path>,
        level: ConfigLevel,
        force: bool,
    ) -> ApiResult<()> {
        let p = arg(path_bytes(path.as_ref()))?;
        Ok(add_file(self.inner(), as_cstr(&p), level.raw(), force)?)
    }

    /// `git_config_backend_from_string` + `git_config_add_backend`: the
    /// configuration text `text`, read-only, at `level`.
    pub fn add_memory(&self, text: &[u8], level: ConfigLevel, force: bool) -> ApiResult<()> {
        let b = crate::config_mem::backend_from_string(text, None, None)?;
        self.inner()
            .add_backend_norepo(b, level.raw(), force)
            .map_err(|(e, _)| Error::from(e))
    }

    /// `git_config_add_backend` of a Rust backend.
    pub fn add_backend<B: ConfigBackend>(
        &self,
        backend: B,
        level: ConfigLevel,
        force: bool,
    ) -> ApiResult<()> {
        let b = rust_backend::new(backend);
        self.inner()
            .add_backend_norepo(b, level.raw(), force)
            .map_err(|(e, _)| Error::from(e))
    }

    /// `git_config_open_level`.
    pub fn open_level(&self, level: ConfigLevel) -> ApiResult<Config<'r>> {
        Ok(Config::from_box(self.inner().open_level(level.raw())?))
    }

    /// `git_config_open_global`.
    pub fn open_global(&self) -> ApiResult<Config<'r>> {
        Ok(Config::from_box(self.inner().open_global()?))
    }

    /// `git_config_snapshot`: a read-only copy.
    pub fn snapshot(&self) -> ApiResult<Config<'r>> {
        Ok(Config::from_box(self.inner().snapshot()?))
    }

    /// `git_config_set_writeorder`.
    pub fn set_writeorder(&self, levels: &[ConfigLevel]) -> ApiResult<()> {
        let l: Vec<c_int> = levels.iter().map(|l| l.raw()).collect();
        Ok(self.inner().set_writeorder(&l)?)
    }

    /// `git_config_lock`: lock the backend writes go to (the highest
    /// writable one), for a transaction. Writes made meanwhile are kept
    /// until [`ConfigLock::commit`]; dropping the lock discards them
    /// ([`ErrorCode::READ_ONLY`] if no backend is writable).
    pub fn lock(&self) -> ApiResult<ConfigLock<'_>> {
        let Some(instance) = self.inner().writer() else {
            return Err(Error::with(
                ErrorCode::READ_ONLY,
                ErrorClass::CONFIG,
                "cannot lock: the configuration is read-only",
            ));
        };
        instance.backend.lock()?;
        Ok(ConfigLock {
            instance: Some(instance),
            _config: core::marker::PhantomData,
        })
    }

    /// `git_config_get_entry`.
    pub fn get_entry(&self, name: impl AsRef<[u8]>) -> ApiResult<ConfigEntry> {
        let name = name.as_ref();
        let n = arg(name)?;
        let e = self.inner().get_entry(as_cstr(&n), true, Want::AllErrors)?;
        let e = e.ok_or_else(|| Error::from(error_notfound(name)))?;
        Ok(ConfigEntry::from_view(&e.view()))
    }

    fn with_value<T>(
        &self,
        name: &[u8],
        f: impl FnOnce(Option<&[u8]>) -> Result<T>,
    ) -> ApiResult<T> {
        let n = arg(name)?;
        let e = self.inner().get_entry(as_cstr(&n), true, Want::AllErrors)?;
        let e = e.ok_or_else(|| Error::from(error_notfound(name)))?;
        Ok(f(e.value())?)
    }

    /// `git_config_get_bool`.
    pub fn get_bool(&self, name: impl AsRef<[u8]>) -> ApiResult<bool> {
        self.with_value(name.as_ref(), parse_bool)
    }

    /// `git_config_get_int32`.
    pub fn get_i32(&self, name: impl AsRef<[u8]>) -> ApiResult<i32> {
        self.with_value(name.as_ref(), parse_int32)
    }

    /// `git_config_get_int64`.
    pub fn get_i64(&self, name: impl AsRef<[u8]>) -> ApiResult<i64> {
        self.with_value(name.as_ref(), parse_int64)
    }

    /// `git_config_get_string_buf`: the value's bytes (empty for a bare
    /// name).
    pub fn get_bytes(&self, name: impl AsRef<[u8]>) -> ApiResult<Vec<u8>> {
        self.with_value(name.as_ref(), |v| Ok(v.unwrap_or(b"").to_vec()))
    }

    /// The value as a `String` (an error if it is not UTF-8).
    pub fn get_string(&self, name: impl AsRef<[u8]>) -> ApiResult<String> {
        let v = self.get_bytes(name)?;
        String::from_utf8(v).map_err(|_| {
            Error::with(
                ErrorCode::GENERIC,
                ErrorClass::CONFIG,
                "the config value is not valid UTF-8",
            )
        })
    }

    /// `git_config_get_path`: the value with `~/` expanded.
    pub fn get_path(&self, name: impl AsRef<[u8]>) -> ApiResult<std::path::PathBuf> {
        let mut out = GitStr::new();
        self.with_value(name.as_ref(), |v| match v {
            Some(v) => parse_path(&mut out, v),
            None => Err(errors::assert_arg_failed("value")),
        })?;
        Ok(to_path(&out))
    }

    /// `git_config_get_mapped`: the value of `name` mapped by the first
    /// of `maps` it matches (an error if none does).
    pub fn get_mapped(&self, name: impl AsRef<[u8]>, maps: &[ConfigMap<'_>]) -> ApiResult<i32> {
        let raw: Vec<RawConfigMap<'_>> = maps.iter().map(ConfigMap::raw).collect();
        self.with_value(name.as_ref(), |v| lookup_map_value(&raw, v))
    }

    /// `git_config_lookup_map_value`: `value` mapped by the first of
    /// `maps` it matches (an error if none does).
    pub fn lookup_map_value(maps: &[ConfigMap<'_>], value: Option<&[u8]>) -> ApiResult<i32> {
        crate::libgit2::ensure_init()?;
        let raw: Vec<RawConfigMap<'_>> = maps.iter().map(ConfigMap::raw).collect();
        Ok(lookup_map_value(&raw, value)?)
    }

    /// `git_config_parse_bool`: `true`/`yes`/`on`/a non-zero integer, or
    /// `false`/`no`/`off`/`0`/empty (ignoring case).
    pub fn parse_bool(value: impl AsRef<[u8]>) -> ApiResult<bool> {
        crate::libgit2::ensure_init()?;
        Ok(parse_bool(Some(value.as_ref()))?)
    }

    /// `git_config_parse_int32`: an integer with an optional `k`, `m` or
    /// `g` suffix.
    pub fn parse_i32(value: impl AsRef<[u8]>) -> ApiResult<i32> {
        crate::libgit2::ensure_init()?;
        Ok(parse_int32(Some(value.as_ref()))?)
    }

    /// `git_config_parse_int64`.
    pub fn parse_i64(value: impl AsRef<[u8]>) -> ApiResult<i64> {
        crate::libgit2::ensure_init()?;
        Ok(parse_int64(Some(value.as_ref()))?)
    }

    /// `git_config_parse_path`: `value` with a leading `~/` expanded to
    /// the home directory.
    pub fn parse_path(value: impl AsRef<[u8]>) -> ApiResult<std::path::PathBuf> {
        crate::libgit2::ensure_init()?;
        let v = arg(value.as_ref())?;
        let mut out = GitStr::new();
        parse_path(&mut out, until_nul(&v))?;
        Ok(to_path(&out))
    }

    /// `git_config_set_string`.
    pub fn set_str(&self, name: impl AsRef<[u8]>, value: &str) -> ApiResult<()> {
        self.set_bytes(name, value)
    }

    /// `git_config_set_string` of bytes.
    pub fn set_bytes(&self, name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> ApiResult<()> {
        let n = arg(name.as_ref())?;
        let v = arg(value.as_ref())?;
        Ok(self.inner().set_string(as_cstr(&n), Some(as_cstr(&v)))?)
    }

    /// `git_config_set_bool`.
    pub fn set_bool(&self, name: impl AsRef<[u8]>, value: bool) -> ApiResult<()> {
        self.set_str(name, if value { "true" } else { "false" })
    }

    /// `git_config_set_int32`.
    pub fn set_i32(&self, name: impl AsRef<[u8]>, value: i32) -> ApiResult<()> {
        self.set_i64(name, i64::from(value))
    }

    /// `git_config_set_int64`.
    pub fn set_i64(&self, name: impl AsRef<[u8]>, value: i64) -> ApiResult<()> {
        self.set_str(name, &value.to_string())
    }

    /// `git_config_set_multivar`: set the values of `name` that match
    /// `regexp` (adding one if none does).
    pub fn set_multivar(
        &self,
        name: impl AsRef<[u8]>,
        regexp: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> ApiResult<()> {
        let n = arg(name.as_ref())?;
        let r = arg(regexp.as_ref())?;
        let v = arg(value.as_ref())?;
        self.inner()
            .set_multivar(as_cstr(&n), Some(as_cstr(&r)), Some(as_cstr(&v)))?;
        Ok(())
    }

    /// `git_config_delete_entry`.
    pub fn remove(&self, name: impl AsRef<[u8]>) -> ApiResult<()> {
        let n = arg(name.as_ref())?;
        self.inner().delete_entry(as_cstr(&n))?;
        Ok(())
    }

    /// `git_config_delete_multivar`.
    pub fn remove_multivar(
        &self,
        name: impl AsRef<[u8]>,
        regexp: impl AsRef<[u8]>,
    ) -> ApiResult<()> {
        let n = arg(name.as_ref())?;
        let r = arg(regexp.as_ref())?;
        self.inner()
            .delete_multivar(as_cstr(&n), Some(as_cstr(&r)))?;
        Ok(())
    }

    /// `git_config_iterator_glob_new`: every entry (whose name matches
    /// `glob`, a regular expression), from the lowest level up.
    pub fn entries(&self, glob: Option<&[u8]>) -> ApiResult<ConfigEntries<'_>> {
        let g = glob.map(arg).transpose()?;
        let it = self.inner().iterator(g.as_deref().map(as_cstr))?;
        Ok(ConfigEntries {
            it,
            _config: core::marker::PhantomData,
        })
    }

    /// `git_config_multivar_iterator_new`: the values of `name` (that
    /// match `regexp`).
    pub fn multivar(
        &self,
        name: impl AsRef<[u8]>,
        regexp: Option<&[u8]>,
    ) -> ApiResult<ConfigEntries<'_>> {
        let n = arg(name.as_ref())?;
        let r = regexp.map(arg).transpose()?;
        let it = self
            .inner()
            .multivar_iterator(as_cstr(&n), r.as_deref().map(as_cstr))?;
        Ok(ConfigEntries {
            it,
            _config: core::marker::PhantomData,
        })
    }
}

/// A transaction on a [`Config`]'s writable backend ([`Config::lock`]):
/// [`commit`](Self::commit) keeps the writes made while it lives; dropped
/// without, it discards them.
pub struct ConfigLock<'c> {
    instance: Option<Arc<Instance>>,
    _config: core::marker::PhantomData<&'c ()>,
}

impl ConfigLock<'_> {
    /// Unlock the backend, keeping the writes (`git_transaction_commit`).
    pub fn commit(mut self) -> ApiResult<()> {
        let Some(instance) = self.instance.take() else {
            return Ok(());
        };
        instance.backend.unlock(1)?;
        Ok(())
    }
}

impl Drop for ConfigLock<'_> {
    fn drop(&mut self) {
        if let Some(instance) = self.instance.take() {
            // A discard cannot be reported from a drop.
            let _ = instance.backend.unlock(0);
        }
    }
}

impl core::fmt::Debug for ConfigLock<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConfigLock").finish_non_exhaustive()
    }
}

impl Default for Config<'static> {
    fn default() -> Self {
        Config::new()
    }
}

/// The entries of a [`Config`] (`git_config_iterator`).
pub struct ConfigEntries<'c> {
    it: IterBox,
    _config: core::marker::PhantomData<&'c ()>,
}

impl core::fmt::Debug for ConfigEntries<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConfigEntries").finish_non_exhaustive()
    }
}

impl Iterator for ConfigEntries<'_> {
    type Item = ApiResult<ConfigEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.it.next_entry() {
            // SAFETY: `be` is the entry the iterator just returned, live
            // and unwritten until its next `next`; the view is copied into
            // an owned entry at once.
            Ok(be) => Some(Ok(ConfigEntry::from_view(&unsafe {
                EntryView::of(be.as_ptr())
            }))),
            Err(e) if e == ErrorCode::ITER_OVER => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}

/// A configuration backend implemented in Rust (`git_config_backend`).
///
/// The keys it is asked for are normalized (`section.subsection.name`,
/// lower-cased but for the subsection). Every method has a default, so
/// that the trait can grow: by default the backend is empty and its
/// writes fail as read-only. A panic in a method is caught and becomes an
/// error of class [`ErrorClass::INTERNAL`]. Names it returns must not
/// contain NUL ([`ErrorCode::INVALID_SPEC`] otherwise). Its `Drop` may
/// panic: the panic is caught (and its payload leaked), as C frees a
/// backend where it cannot unwind. A backend that holds a clone of its own
/// [`Config`] keeps it alive forever (a cycle).
pub trait ConfigBackend: Send + Sync + 'static {
    /// `open`: called once when the backend is added at `level`.
    fn open(&self, level: ConfigLevel) -> ApiResult<()> {
        let _ = level;
        Ok(())
    }

    /// `get`: the entry `key` (the last one, for a multivar), or `None`;
    /// by default, `None`.
    fn get(&self, key: &[u8]) -> ApiResult<Option<ConfigEntry>> {
        let _ = key;
        Ok(None)
    }

    /// `iterator`: every entry, in order; by default, none.
    fn entries(&self) -> ApiResult<Vec<ConfigEntry>> {
        Ok(Vec::new())
    }

    /// `set` (`name` as the user gave it).
    fn set(&self, name: &[u8], value: &[u8]) -> ApiResult<()> {
        let _ = (name, value);
        Err(read_only())
    }

    /// `set_multivar`.
    fn set_multivar(&self, name: &[u8], regexp: &[u8], value: Option<&[u8]>) -> ApiResult<()> {
        let _ = (name, regexp, value);
        Err(read_only())
    }

    /// `del`.
    fn remove(&self, name: &[u8]) -> ApiResult<()> {
        let _ = name;
        Err(read_only())
    }

    /// `del_multivar`.
    fn remove_multivar(&self, name: &[u8], regexp: &[u8]) -> ApiResult<()> {
        let _ = (name, regexp);
        Err(read_only())
    }

    /// `lock` ([`Config::lock`]).
    fn lock(&self) -> ApiResult<()> {
        Err(read_only())
    }

    /// `unlock`: end the transaction, keeping its writes if `commit`.
    fn unlock(&self, commit: bool) -> ApiResult<()> {
        let _ = commit;
        Err(read_only())
    }

    /// Whether writes go elsewhere (`readonly`).
    fn readonly(&self) -> bool {
        false
    }
}

/// The error of a read-only backend's writes.
pub(crate) fn read_only() -> Error {
    Error::with(
        ErrorCode::GENERIC,
        ErrorClass::CONFIG,
        "this backend is read-only",
    )
}

/// A [`ConfigBackend`] behind a `git_config_backend` table.
mod rust_backend {
    use super::*;
    use crate::config_list::{ConfigList, EntryMeta};

    pub(super) struct State {
        backend: crate::api::NoUnwindDrop<Box<dyn ConfigBackend>>,
        level: std::sync::atomic::AtomicI32,
    }

    /// A C string argument.
    ///
    /// # Safety
    ///
    /// `p` is NULL or NUL-terminated, and unwritten for `'a` (a slot's
    /// call).
    unsafe fn b<'a>(p: *const c_char) -> &'a [u8] {
        if p.is_null() {
            return b"";
        }
        // SAFETY: a non-null `p` is NUL-terminated and unwritten for `'a`
        // (this function's contract).
        unsafe { CStr::from_ptr(p) }.to_bytes()
    }

    fn status(r: ApiResult<()>) -> c_int {
        match r {
            Ok(()) => 0,
            Err(e) => raise(&e).raw(),
        }
    }

    fn list_of(entries: &[ConfigEntry]) -> Result<Arc<ConfigList>> {
        let mut l = ConfigList::new();
        for e in entries {
            // A NUL would cut the name short in C (S-6e): refuse it.
            if e.name.contains(&0) {
                let _ = git_error_set!(
                    ErrorClass::CONFIG,
                    "the config entry name '{}' from the backend contains a NUL byte",
                    errors::Bytes(&e.name[..memchr::memchr(0, &e.name).unwrap_or(e.name.len())])
                );
                return Err(ErrorCode::INVALID_SPEC);
            }
            let backend_type = match &e.backend_type {
                Some(t) => l.intern(t)?,
                None => l.intern(b"rust")?,
            };
            let origin_path = match &e.origin_path {
                Some(p) => l.intern(p)?,
                None => core::ptr::null(),
            };
            l.push(
                &e.name,
                e.value.as_deref(),
                EntryMeta {
                    backend_type,
                    origin_path,
                    include_depth: e.include_depth,
                    level: e.level.raw(),
                },
            )?;
        }
        Ok(l.freeze())
    }

    impl State {
        fn get(&self, key: &[u8], mut out: Out<'_, *mut git_config_backend_entry>) -> c_int {
            match self.backend.get(key) {
                Err(e) => raise(&e).raw(),
                Ok(None) => ErrorCode::NOT_FOUND.raw(),
                Ok(Some(mut e)) => {
                    if e.level == ConfigLevel::Highest {
                        e.level = ConfigLevel::from_raw(
                            self.level.load(core::sync::atomic::Ordering::Relaxed),
                        );
                    }
                    match list_of(core::slice::from_ref(&e)) {
                        Err(c) => c.raw(),
                        Ok(l) => {
                            out.write(ConfigList::lend(&l, &l.entries()[0]));
                            0
                        }
                    }
                }
            }
        }

        fn iterator(&self, mut out: Out<'_, *mut git_config_iterator>) -> c_int {
            match self.backend.entries() {
                Err(e) => raise(&e).raw(),
                Ok(entries) => match list_of(&entries) {
                    Err(c) => c.raw(),
                    Ok(l) => {
                        out.write(crate::config_list::iterator_new(l));
                        0
                    }
                },
            }
        }
    }

    ffibox::impl_cvtable! {
        // SAFETY: libgit2 passes each slot its backend, in the position
        // marked; every slot libgit2 calls is set (`snapshot` and `free` too).
        unsafe impl CVtable<State> for ConfigBackendCell {}
        base = git_config_backend {
            snapshot: Some(crate::config_snapshot::ffi::git_config_backend_snapshot),
            ..git_config_backend::default()
        };
        fn open(@cfg, level: c_int, repo: *const git_repository) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            |s: &State| {
                let _ = repo;
                s.level.store(level, core::sync::atomic::Ordering::Relaxed);
                status(s.backend.open(ConfigLevel::from_raw(level)))
            },
        }
        fn get(@cfg, key: *const c_char, out: *mut *mut git_config_backend_entry) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            // SAFETY: `key` is a NUL-terminated string and `out` writable
            // for the call (the slot's contract, sys/config.h); nothing
            // else references `*out`.
            |s: &State| s.get(unsafe { b(key) }, unsafe { Out::from_ptr(out) }),
        }
        fn set(@cfg, name: *const c_char, value: *const c_char) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            // SAFETY: the strings are NUL-terminated (the slot's contract).
            |s: &State| status(s.backend.set(unsafe { b(name) }, unsafe { b(value) })),
        }
        fn set_multivar(@cfg, name: *const c_char, regexp: *const c_char, value: *const c_char) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            |s: &State| {
                // SAFETY: `name` and `regexp` are NULL or NUL-terminated
                // strings borrowed for the call (the slot's contract).
                let (name, regexp) = (unsafe { b(name) }, unsafe { b(regexp) });
                // SAFETY: as above, for `value`.
                let value = (!value.is_null()).then(|| unsafe { b(value) });
                status(s.backend.set_multivar(name, regexp, value))
            },
        }
        fn del(@cfg, name: *const c_char) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            // SAFETY: as for `set`.
            |s: &State| status(s.backend.remove(unsafe { b(name) })),
        }
        fn del_multivar(@cfg, name: *const c_char, regexp: *const c_char) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            // SAFETY: as for `set`.
            |s: &State| status(s.backend.remove_multivar(unsafe { b(name) }, unsafe { b(regexp) })),
        }
        fn iterator(out: *mut *mut git_config_iterator, @cfg) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            // SAFETY: `out` is writable for the call (the slot's contract),
            // and nothing else references `*out`.
            |s: &State| s.iterator(unsafe { Out::from_ptr(out) }),
        }
        fn lock(@cfg) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            |s: &State| status(s.backend.lock()),
        }
        fn unlock(@cfg, success: c_int) -> c_int {
            on panic: slot_panicked(),
            on busy: slot_busy(),
            |s: &State| status(s.backend.unlock(success != 0)),
        }
        release free;
    }

    /// The table of `backend`.
    pub(super) fn new<B: ConfigBackend>(backend: B) -> Backend {
        let readonly = backend.readonly();
        let state = State {
            backend: crate::api::NoUnwindDrop::new(Box::new(backend)),
            level: std::sync::atomic::AtomicI32::new(0),
        };
        let table = CTable::<ConfigBackendCell, State>::new(state);
        let p = table.as_c_ptr();
        // SAFETY: the table is ours alone here (no slot runs); `readonly`
        // is a plain field C reads.
        unsafe { (*p).readonly = c_int::from(readonly) };
        let p = table.into_c();
        // SAFETY: a table surrendered just now, released through its `free`
        // slot (`CTable::release`), which `BackendFree` calls.
        unsafe { Backend::from_c(p.as_ptr().cast()) }.unwrap_or_else(|| unreachable!())
    }
}

crate::c_layout_test! {
    headers: ["config.h", "git2/sys/config.h"],
    types: [
        git_config_entry => "git_config_entry" { name, value, backend_type, origin_path, include_depth, level },
        git_config_backend_entry => "git_config_backend_entry" { entry, free },
        git_config_iterator => "git_config_iterator" { backend, flags, next, free },
        git_config_backend => "git_config_backend" {
            version, readonly, cfg, open, get, set, set_multivar, del, del_multivar,
            iterator, snapshot, lock, unlock, free
        },
        git_config_backend_memory_options => "git_config_backend_memory_options" { version, backend_type, origin_path },
        git_configmap => "git_configmap" { type_ as "type", str_match, map_value },
    ],
    asserts: [
        "offsetof(struct git_config, rc) == 0",
        "GIT_CONFIG_BACKEND_VERSION == 1",
        "GIT_CONFIGMAP_STRING == 3",
        "GIT_CONFIG_LEVEL_APP == 7",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize() {
        let _g = crate::util::test_support::init();
        assert_eq!(normalize_name(b"Core.Bare").unwrap(), b"core.bare");
        assert_eq!(
            normalize_name(b"Remote.Origin.URL").unwrap(),
            b"remote.Origin.url"
        );
        for bad in [&b"core"[..], b".a", b"a.", b"a.-b", b"a b.c", b"a.b\nc.d"] {
            assert_eq!(
                normalize_name(bad).err(),
                Some(ErrorCode::INVALID_SPEC),
                "{bad:?}"
            );
        }
        assert_eq!(
            normalize_name(b"a.b\nc").err(),
            Some(ErrorCode::INVALID_SPEC)
        );
        assert!(normalize_name(b"a.b\nc.d").is_err());
        assert!(normalize_name(b"a.x\ny.d").is_err());
    }

    #[test]
    fn parse_values() {
        let _g = crate::util::test_support::init();
        assert_eq!(parse_int64(Some(b"1k")).unwrap(), 1024);
        assert_eq!(parse_int64(Some(b"2M")).unwrap(), 2 << 20);
        assert_eq!(parse_int64(Some(b"-1g")).unwrap(), -(1 << 30));
        assert_eq!(parse_int64(Some(b"0x10")).unwrap(), 16);
        assert!(parse_int64(Some(b"1kx")).is_err());
        assert!(parse_int64(Some(b"1q")).is_err());
        assert!(parse_int64(None).is_err());
        assert!(parse_int32(Some(b"3g")).is_err());
        assert_eq!(
            errors::last().1,
            b"failed to parse '3g' as a 32-bit integer"
        );
        assert!(parse_bool(None).unwrap());
        assert!(parse_bool(Some(b"on")).unwrap());
        assert!(!parse_bool(Some(b"0")).unwrap());
        assert!(parse_bool(Some(b"5")).unwrap());
        assert!(parse_bool(Some(b"maybe")).is_err());
        assert_eq!(errors::last().1, b"failed to parse 'maybe' as a boolean");
    }

    #[test]
    fn map_values() {
        let _g = crate::util::test_support::init();
        let maps = [
            RawConfigMap {
                kind: GIT_CONFIGMAP_FALSE,
                str_match: None,
                map_value: 10,
            },
            RawConfigMap {
                kind: GIT_CONFIGMAP_TRUE,
                str_match: None,
                map_value: 11,
            },
            RawConfigMap {
                kind: GIT_CONFIGMAP_STRING,
                str_match: Some(b"input"),
                map_value: 12,
            },
        ];
        assert_eq!(lookup_map_value(&maps, Some(b"false")).unwrap(), 10);
        assert_eq!(lookup_map_value(&maps, None).unwrap(), 11);
        assert_eq!(lookup_map_value(&maps, Some(b"INPUT")).unwrap(), 12);
        assert!(lookup_map_value(&maps, Some(b"x")).is_err());
        assert_eq!(errors::last().1, b"failed to map 'x'");
    }
}
