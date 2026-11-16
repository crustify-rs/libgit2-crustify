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

// SAFETY: libgit2's contract for config backends (sys/config.h): a backend
// may be used from any thread, and from several at once, as a repository's
// config is (`tests/libgit2/threads`); its slots synchronise what they
// share. The handles only call slots.
unsafe impl Send for ConfigBackendCell {}
// SAFETY: as above.
unsafe impl Sync for ConfigBackendCell {}
// SAFETY: an iterator is used by one thread at a time (libgit2's contract
// for non-shared objects), and may move between threads; Rust reaches it
// only through `&mut` (exclusive slots).
unsafe impl Send for ConfigIterator {}
// SAFETY: as above: no slot is reachable through a shared borrow.
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
    let _ = errors::set_str(e.class(), e.message_bytes());
    e.code()
}

// ---------------------------------------------------------------------------
// Backends, entries and iterators held from Rust
// ---------------------------------------------------------------------------

/// Calls a table's `free` slot (the release of [`Backend`]).
///
/// # Safety
///
/// `b` is a live backend, released once.
unsafe extern "C" fn backend_free(b: *mut git_config_backend) {
    // SAFETY: per the contract, `b` is live; the slot is read raw.
    if let Some(free) = unsafe { (*b).free } {
        // SAFETY: C's contract: `free` releases the backend.
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

/// Read slot `$f` of the backend at `$p` (raw, as C may write the table).
macro_rules! slot {
    ($p:expr, $f:ident) => {{
        let p: *mut git_config_backend = $p;
        // SAFETY: `p` is a live table (the caller's invariant); the field is
        // read through a raw pointer.
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
/// use libgit2_rs::config::{BackendCalls, git_config_backend};
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
        // SAFETY: a live table; the field is read raw.
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
        let f = slot!(p, open).ok_or_else(|| no_slot("open"))?;
        // SAFETY: the slot's contract (see the trait).
        ErrorCode::check(unsafe { f(p, level, repo) }).map(drop)
    }

    /// `get`: `Ok(None)` for `GIT_ENOTFOUND` (the backend's message kept).
    fn get(&self, key: &CStr) -> Result<Option<EntryBox>> {
        let p = self.raw();
        let f = slot!(p, get).ok_or_else(|| no_slot("get"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: the slot's contract; `out` is writable.
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
        let f = slot!(p, set).ok_or_else(|| no_slot("set"))?;
        // SAFETY: the slot's contract.
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
        let f = slot!(p, set_multivar).ok_or_else(|| no_slot("set_multivar"))?;
        // SAFETY: the slot's contract (the file backend checks `regexp`).
        ErrorCode::check(unsafe { f(p, name.as_ptr(), opt_ptr(regexp), opt_ptr(value)) })
    }

    /// `del`.
    fn del(&self, name: &CStr) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(p, del).ok_or_else(|| no_slot("del"))?;
        // SAFETY: the slot's contract.
        ErrorCode::check(unsafe { f(p, name.as_ptr()) })
    }

    /// `del_multivar`.
    fn del_multivar(&self, name: &CStr, regexp: Option<&CStr>) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(p, del_multivar).ok_or_else(|| no_slot("del_multivar"))?;
        // SAFETY: the slot's contract.
        ErrorCode::check(unsafe { f(p, name.as_ptr(), opt_ptr(regexp)) })
    }

    /// `iterator`.
    fn iterator(&self) -> Result<IterBox> {
        let p = self.raw();
        let f = slot!(p, iterator).ok_or_else(|| no_slot("iterator"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: the slot's contract; `out` is writable.
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
        let f = slot!(p, snapshot).ok_or_else(|| no_slot("snapshot"))?;
        let mut out = core::ptr::null_mut();
        // SAFETY: the slot's contract; `out` is writable.
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
        let f = slot!(p, lock).ok_or_else(|| no_slot("lock"))?;
        // SAFETY: the slot's contract.
        ErrorCode::check(unsafe { f(p) })
    }

    /// `unlock`.
    fn unlock(&self, success: c_int) -> Result<c_int> {
        let p = self.raw();
        let f = slot!(p, unlock).ok_or_else(|| no_slot("unlock"))?;
        // SAFETY: the slot's contract.
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

// SAFETY: entries are released on any thread (C's contract: they keep
// their backend's data alive by reference counts, which are atomic).
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
        // SAFETY: a live entry (owned); its value is NULL or a C string that
        // lives as long as the entry and that nothing writes.
        let v = unsafe { (*self.0.as_ptr()).entry.value };
        // SAFETY: as above.
        (!v.is_null()).then(|| unsafe { CStr::from_ptr(v) }.to_bytes())
    }

    /// The entry's fields, read once.
    pub fn view(&self) -> EntryView<'_> {
        // SAFETY: a live entry (owned), which nothing writes: its fields
        // are read by value.
        let e = unsafe { self.0.as_ptr().cast::<git_config_entry>().read() };
        // SAFETY: as above; the strings live as long as the entry.
        unsafe { EntryView::new(&e) }
    }
}

impl Drop for EntryBox {
    fn drop(&mut self) {
        // SAFETY: an owned entry, released once.
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
    // SAFETY: per the contract; the slot is read raw.
    if let Some(free) = unsafe { (*be).free } {
        // SAFETY: C's contract: `free` releases the entry.
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
            // SAFETY: per the contract.
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
    /// `be` is a live entry, as [`new`](Self::new) requires.
    pub unsafe fn of(be: *const git_config_backend_entry) -> EntryView<'a> {
        // SAFETY: per the contract; read by value.
        let e = unsafe { be.cast::<git_config_entry>().read() };
        // SAFETY: per the contract.
        unsafe { EntryView::new(&e) }
    }
}

/// A C iterator, released through its `free`.
#[derive(Debug)]
pub struct IterBox(NonNull<git_config_iterator>);

// SAFETY: as `ConfigIterator`: used by one thread at a time.
unsafe impl Send for IterBox {}
// SAFETY: `IterBox` has no method taking `&self` that calls a slot.
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
        // SAFETY: a live iterator, ours; the slot is read raw.
        let Some(f) = (unsafe { (*p).next }) else {
            return Err(no_slot("next"));
        };
        let mut out = core::ptr::null_mut();
        // SAFETY: C's contract for `next`; `out` is writable.
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
        // SAFETY: an owned iterator, released once; the slot is read raw.
        if let Some(free) = unsafe { (*p).free } {
            // SAFETY: as above.
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
        // SAFETY: a live table (owned).
        let version = unsafe { (*p).version };
        if let Err(e) = check_version(version, GIT_CONFIG_BACKEND_VERSION, "git_config_backend") {
            return Err((e, backend));
        }
        // SAFETY: `repo` per this function's contract.
        if let Err(e) = unsafe { backend.open(level, repo) } {
            return Err((e, backend));
        }
        // SAFETY: a live table (owned); C writes the field the same way.
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
        // SAFETY: a new iterator, ours.
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
        // SAFETY: a new iterator, ours.
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

    /// `git_config_lock`: lock the writer, and a transaction that unlocks
    /// it.
    ///
    /// # Safety
    ///
    /// `out` is writable; `this` is `self`'s C pointer.
    unsafe fn lock(&self, out: *mut *mut crate::sys::transaction::git_transaction) -> Result<()> {
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
        // SAFETY: `out` is writable (contract); `this` is live; `data` is an
        // instance reference that `git_config_unlock` releases.
        let r = unsafe { crate::sys::transaction::config_new(out, this, data) };
        if r.is_err() {
            // SAFETY: the reference made above, not handed over.
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
            // SAFETY: the entry the iterator holds, live for the call.
            let e = unsafe { &*e };
            // SAFETY: as above: its strings are C's, live for the call.
            let view = unsafe { EntryView::new(e) };
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
            // SAFETY: the entry the iterator holds, live for the call.
            let e = unsafe { &*e };
            // SAFETY: as above: its strings are C's, live for the call.
            let view = unsafe { EntryView::new(e) };
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
            // SAFETY: an entry the current backend iterator holds.
            let name = unsafe { EntryView::of(be.as_ptr()) }.name;
            if regex.matches(name).is_ok() {
                return Ok(be);
            }
        }
    }
}

/// Write an iterator's result to C's out-parameter.
///
/// # Safety
///
/// `out` is writable.
unsafe fn iter_result(
    out: *mut *mut git_config_backend_entry,
    r: Result<NonNull<git_config_backend_entry>>,
) -> c_int {
    match r {
        Ok(be) => {
            // SAFETY: per the contract.
            unsafe { out.write(be.as_ptr()) };
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
        // SAFETY: `entry` is C's writable out-parameter.
        |it: &mut AllIter| unsafe { iter_result(entry, it.next_entry()) },
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
            // SAFETY: an entry the inner iterator holds.
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
        // SAFETY: `entry` is C's writable out-parameter.
        |it: &mut MultivarIter| unsafe { iter_result(entry, it.next_entry()) },
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
            // SAFETY: an entry the iterator holds.
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
pub struct ConfigMap<'a> {
    /// `type` (`GIT_CONFIGMAP_*`).
    pub kind: c_int,
    /// `str_match`.
    pub str_match: Option<&'a [u8]>,
    /// `map_value`.
    pub map_value: c_int,
}

/// `git_config_lookup_map_value`.
pub fn lookup_map_value(maps: &[ConfigMap<'_>], value: Option<&[u8]>) -> Result<c_int> {
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
    use crate::util::export::{barrier, cstr_bytes};

    /// A C string argument, borrowed for the call.
    ///
    /// # Safety
    ///
    /// `p` is NULL or NUL-terminated, not written during the call.
    unsafe fn c<'a>(p: *const c_char) -> Option<&'a CStr> {
        // SAFETY: per the contract.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
    }

    /// A NULL-able C string argument as `%s` prints it.
    ///
    /// # Safety
    ///
    /// As [`c`].
    unsafe fn or_null_c<'a>(p: *const c_char) -> &'a CStr {
        // SAFETY: per the contract.
        unsafe { c(p) }.unwrap_or(c"(null)")
    }

    /// The config at `p`.
    ///
    /// # Safety
    ///
    /// `p` is a live config, for `'a`.
    unsafe fn cfg<'a>(p: *const GitConfig) -> &'a GitConfig {
        // SAFETY: per the contract; `GitConfig` is `Sync` and only read
        // through `&` (its state has interior mutability).
        unsafe { &*p }
    }

    /// Write a new config to `out`.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    unsafe fn out_config(out: *mut *mut GitConfig, c: Box<GitConfig>) {
        // SAFETY: per the contract.
        unsafe { out.write(CShared::into_raw(CShared::from_box(c))) };
    }

    /// `GIT_BUF_WRAP_PRIVATE`: run `f` on a `git_str` made from `buf`, and
    /// move the result back.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_buf`.
    pub(crate) unsafe fn wrap_buf(
        buf: *mut GitBuf,
        f: impl FnOnce(&mut GitStr) -> Result<()>,
    ) -> c_int {
        let mut s = GitStr::new();
        // SAFETY: `s` is a valid `git_str`; `buf` per the contract.
        let mut error = unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, buf) };
        if error == 0 {
            error = match f(&mut s) {
                Ok(()) => 0,
                Err(e) => e.raw(),
            };
            if error == 0 {
                // SAFETY: as above.
                error = unsafe { crate::buf::ffi::git_buf_fromstr(buf, &raw mut s) };
            }
        }
        drop(s);
        error
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
                // SAFETY: per the contract.
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
        // SAFETY: the contract.
        barrier(|| unsafe { CShared::release(config) })
    }

    /// `git_config_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_new(out: *mut *mut GitConfig) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            unsafe { out_config(out, GitConfig::new()) };
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
        config: *mut GitConfig,
        path: *const c_char,
        level: c_int,
        repo: *const git_repository,
        force: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!config.is_null(), "config", -1);
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `path` is NUL-terminated (the contract).
            let path = unsafe { CStr::from_ptr(path) };
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
        out: *mut *mut GitConfig,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `out` is writable.
            unsafe { out.write(core::ptr::null_mut()) };
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: per the contract.
            match open_ondisk(unsafe { CStr::from_ptr(path) }) {
                // SAFETY: `out` is writable.
                Ok(c) => unsafe {
                    out_config(out, c);
                    0
                },
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
        out: *mut *mut GitConfig,
        config: *mut GitConfig,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `out` is writable.
            unsafe { out.write(core::ptr::null_mut()) };
            match config.snapshot() {
                // SAFETY: `out` is writable.
                Ok(c) => unsafe {
                    out_config(out, c);
                    0
                },
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
        out: *mut *mut GitConfig,
        parent: *const GitConfig,
        level: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            match unsafe { cfg(parent) }.open_level(level) {
                // SAFETY: `out` is writable.
                Ok(c) => unsafe {
                    out_config(out, c);
                    0
                },
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
        out: *mut *mut GitConfig,
        config: *mut GitConfig,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            match config.open_global() {
                // SAFETY: `out` is writable.
                Ok(c) => unsafe {
                    out_config(out, c);
                    0
                },
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
        config: *mut GitConfig,
        backend: *mut git_config_backend,
        level: c_int,
        repo: *const git_repository,
        force: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!config.is_null(), "config", -1);
            git_assert_arg!(!backend.is_null(), "backend", -1);
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: per the contract: a backend the config may own.
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
    /// `config` is live; `levels` is readable for `len` values (or `len` is
    /// 0).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_writeorder(
        config: *mut GitConfig,
        levels: *mut c_int,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            git_assert!(len < c_int::MAX as usize, "len < INT_MAX", -1);
            let levels = if len == 0 {
                &[][..]
            } else {
                // SAFETY: per the contract.
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
        out: *mut *mut git_config_iterator,
        config: *const GitConfig,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            match config.iterator(None) {
                // SAFETY: `out` is writable.
                Ok(it) => unsafe {
                    out.write(it.into_raw());
                    0
                },
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
        out: *mut *mut git_config_iterator,
        config: *const GitConfig,
        regexp: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            match config.iterator(regexp) {
                // SAFETY: `out` is writable.
                Ok(it) => unsafe {
                    out.write(it.into_raw());
                    0
                },
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
        out: *mut *mut git_config_iterator,
        config: *const GitConfig,
        name: *const c_char,
        regexp: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!name.is_null(), "in", -1);
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            // SAFETY: per the contract.
            let r = unsafe { config.multivar_iterator(CStr::from_ptr(name), regexp) };
            match r {
                // SAFETY: `out` is writable.
                Ok(it) => unsafe {
                    out.write(it.into_raw());
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_next`.
    ///
    /// # Safety
    ///
    /// `entry` is writable; `iter` is a live iterator.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_next(
        entry: *mut *mut git_config_entry,
        iter: *mut git_config_iterator,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live iterator (contract); the slot is read raw.
            let Some(next) = (unsafe { (*iter).next }) else {
                return no_slot("next").raw();
            };
            let mut be = core::ptr::null_mut();
            // SAFETY: C's contract for `next`.
            let error = unsafe { next(&raw mut be, iter) };
            if error != 0 {
                return error;
            }
            // SAFETY: `entry` is writable.
            unsafe { entry.write(be.cast()) };
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
            // SAFETY: per the contract.
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
            // SAFETY: per the contract.
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
        config: *const GitConfig,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: per the contract.
        unsafe { git_config_foreach_match(config, core::ptr::null(), cb, payload) }
    }

    /// `git_config_foreach_match`.
    ///
    /// # Safety
    ///
    /// As `git_config_foreach`; `regexp` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_foreach_match(
        config: *const GitConfig,
        regexp: *const c_char,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            // SAFETY: per the contract; the callback gets the entry the
            // iterator holds and the caller's payload.
            unsafe { config.foreach_match(regexp, |e| call(cb, e, payload)) }
        })
    }

    /// `git_config_backend_foreach_match`.
    ///
    /// # Safety
    ///
    /// `backend` is NULL or a live backend; the rest as
    /// `git_config_foreach_match`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_foreach_match(
        backend: *mut git_config_backend,
        regexp: *const c_char,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            let Some(backend) = NonNull::new(backend) else {
                return errors::assert_arg_failed("backend").raw();
            };
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            // SAFETY: a live backend (contract).
            let b = unsafe { BackendPtr::new(backend) };
            // SAFETY: per the contract, as `git_config_foreach_match`.
            unsafe { backend_foreach_match(&b, regexp, |e| cb(e, payload)) }
        })
    }

    /// `git_config_get_multivar_foreach`.
    ///
    /// # Safety
    ///
    /// As `git_config_foreach_match`; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_multivar_foreach(
        config: *const GitConfig,
        name: *const c_char,
        regexp: *const c_char,
        cb: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!name.is_null(), "in", -1);
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            // SAFETY: `name` is NUL-terminated (the contract).
            let name = unsafe { CStr::from_ptr(name) };
            // SAFETY: the callback is valid for the payload (the contract).
            config.get_multivar_foreach(name, regexp, |e| unsafe { call(cb, e, payload) })
        })
    }

    /// `git_config_delete_entry`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_delete_entry(
        config: *mut GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
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
        config: *mut GitConfig,
        name: *const c_char,
        value: i64,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
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
        config: *mut GitConfig,
        name: *const c_char,
        value: i32,
    ) -> c_int {
        // SAFETY: per the contract.
        unsafe { git_config_set_int64(config, name, i64::from(value)) }
    }

    /// `git_config_set_bool`.
    ///
    /// # Safety
    ///
    /// As `git_config_set_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_bool(
        config: *mut GitConfig,
        name: *const c_char,
        value: c_int,
    ) -> c_int {
        let v = if value != 0 { c"true" } else { c"false" };
        // SAFETY: per the contract.
        unsafe { git_config_set_string(config, name, v.as_ptr()) }
    }

    /// `git_config_set_string`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated; `value` is NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_set_string(
        config: *mut GitConfig,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
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
        config: *mut GitConfig,
        name: *const c_char,
        regexp: *const c_char,
        value: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
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
        config: *mut GitConfig,
        name: *const c_char,
        regexp: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
            // SAFETY: `regexp` is NULL or NUL-terminated (the contract).
            let regexp = unsafe { c(regexp) };
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
        config: *mut GitConfig,
        key: *const c_char,
        value: *const c_char,
        overwrite_existing: bool,
        only_if_existing: bool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `key` is NULL or NUL-terminated (the contract).
            let key = unsafe { or_null_c(key) };
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            let r = config.update_entry(key, value, overwrite_existing, only_if_existing);
            crate::util::export::status(r)
        })
    }

    /// Write an entry (or NULL) to `out`, with `get_entry`'s result.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    unsafe fn out_entry(out: *mut *mut git_config_entry, r: Result<Option<EntryBox>>) -> c_int {
        let (p, code) = match r {
            Ok(Some(e)) => (e.into_raw().cast(), 0),
            Ok(None) => (core::ptr::null_mut(), 0),
            Err(e) => (core::ptr::null_mut(), e.raw()),
        };
        // SAFETY: per the contract.
        unsafe { out.write(p) };
        code
    }

    /// `git_config_get_entry`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_entry(
        out: *mut *mut git_config_entry,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
            // SAFETY: per the contract.
            unsafe { out_entry(out, config.get_entry(name, true, Want::AllErrors)) }
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
        out: *mut *mut git_config_entry,
        config: *const GitConfig,
        key: *const c_char,
        no_errors: bool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `key` is NULL or NUL-terminated (the contract).
            let key = unsafe { or_null_c(key) };
            let want = if no_errors {
                Want::NoErrors
            } else {
                Want::NoMissing
            };
            // SAFETY: per the contract.
            unsafe { out_entry(out, config.get_entry(key, false, want)) }
        })
    }

    /// The value of the entry `name` (normalized), with `f`.
    ///
    /// # Safety
    ///
    /// `config` is live; `name` is NUL-terminated.
    unsafe fn with_value<T>(
        config: *const GitConfig,
        name: *const c_char,
        f: impl FnOnce(Option<&[u8]>) -> Result<T>,
    ) -> Result<T> {
        // SAFETY: per the contract.
        let config = unsafe { cfg(config) };
        // SAFETY: per the contract.
        let name = unsafe { or_null_c(name) };
        config.with_value(name, f)
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
        out: *mut c_int,
        config: *const GitConfig,
        name: *const c_char,
        maps: *const git_configmap,
        map_n: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let maps = unsafe { configmaps(maps, map_n) };
            // SAFETY: per the contract.
            match unsafe { with_value(config, name, |v| lookup_map_value(&maps, v)) } {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// The C maps as Rust ones.
    ///
    /// # Safety
    ///
    /// `maps` is readable for `n` maps whose strings are NULL or
    /// NUL-terminated, for `'a`.
    unsafe fn configmaps<'a>(maps: *const git_configmap, n: usize) -> Vec<ConfigMap<'a>> {
        if n == 0 {
            return Vec::new();
        }
        // SAFETY: per the contract.
        let maps = unsafe { core::slice::from_raw_parts(maps, n) };
        maps.iter()
            .map(|m| ConfigMap {
                kind: m.type_,
                // SAFETY: per the contract.
                str_match: unsafe { c(m.str_match) }.map(CStr::to_bytes),
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
        out: *mut i64,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            match unsafe { with_value(config, name, parse_int64) } {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_get_int32`.
    ///
    /// # Safety
    ///
    /// As `git_config_get_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_int32(
        out: *mut i32,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            match unsafe { with_value(config, name, parse_int32) } {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_get_bool`.
    ///
    /// # Safety
    ///
    /// As `git_config_get_int64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_bool(
        out: *mut c_int,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            match unsafe { with_value(config, name, parse_bool) } {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(c_int::from(v));
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config__get_path`.
    ///
    /// # Safety
    ///
    /// `out` is NULL (an error, as C's assertion) or a valid `git_str`;
    /// `config` is live; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__get_path(
        out: *mut GitStr,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let parse = |v: Option<&[u8]>| {
                // `GIT_ASSERT_ARG(out)` and `(value)` in
                // `git_config__parse_path`, after the lookup, as C.
                if out.is_null() {
                    return Err(errors::assert_arg_failed("out"));
                }
                let Some(v) = v else {
                    return Err(errors::assert_arg_failed("value"));
                };
                // SAFETY: a valid, exclusive `git_str` (the contract; NULL
                // excluded above).
                parse_path(unsafe { &mut *out }, v)
            };
            // SAFETY: `config` is live, `name` NUL-terminated (the contract).
            let r = unsafe { with_value(config, name, parse) };
            crate::util::export::status(r)
        })
    }

    /// `git_config_get_path`.
    ///
    /// # Safety
    ///
    /// `out` is a valid `git_buf`; the rest as `git_config__get_path`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_path(
        out: *mut GitBuf,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let get = |s: &mut GitStr| {
                // SAFETY: `s` is a valid `git_str`; `config` and `name` are
                // as `git_config__get_path` requires (the contract).
                ErrorCode::check_zero(unsafe { git_config__get_path(s, config, name) })
            };
            // SAFETY: `out` is NULL or a valid `git_buf` (the contract).
            unsafe { wrap_buf(out, get) }
        })
    }

    /// `git_config_parse_path`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`; `value` is NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_parse_path(
        out: *mut GitBuf,
        value: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            // SAFETY: per the contract.
            unsafe {
                wrap_buf(out, |s| {
                    let Some(v) = value else {
                        return Err(errors::assert_arg_failed("value"));
                    };
                    parse_path(s, v.to_bytes())
                })
            }
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
        out: *mut *const c_char,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `name` is NULL or NUL-terminated (the contract).
            let name = unsafe { or_null_c(name) };
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
                    // SAFETY: a live entry; read by value.
                    let v = unsafe { (*e.as_entry()).value };
                    (if v.is_null() { c"".as_ptr() } else { v }, 0)
                }
                Ok(None) => (core::ptr::null(), ErrorCode::NOT_FOUND.raw()),
                Err(e) => (core::ptr::null(), e.raw()),
            };
            // SAFETY: `out` is writable.
            unsafe { out.write(p) };
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
        out: *mut GitStr,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!config.is_null(), "config", -1);
            // SAFETY: per the contract.
            let out = unsafe { &mut *out };
            // SAFETY: per the contract.
            let r = unsafe { with_value(config, name, |v| out.puts(v.unwrap_or(b""))) };
            crate::util::export::status(r)
        })
    }

    /// `git_config_get_string_buf`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`; the rest as
    /// `git_config__get_string_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_get_string_buf(
        out: *mut GitBuf,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let get = |s: &mut GitStr| {
                // SAFETY: `s` is a valid `git_str`; `config` and `name` are
                // as `git_config__get_string_buf` requires (the contract).
                ErrorCode::check_zero(unsafe { git_config__get_string_buf(s, config, name) })
            };
            // SAFETY: `out` is NULL or a valid `git_buf` (the contract).
            unsafe { wrap_buf(out, get) }
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
        config: *const GitConfig,
        key: *const c_char,
        fallback_value: *const c_char,
    ) -> *mut c_char {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `key` is NULL or NUL-terminated (the contract).
            let key = unsafe { or_null_c(key) };
            // SAFETY: `fallback_value` is NULL or NUL-terminated (the contract).
            let fallback_value = unsafe { c(fallback_value) };
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
        config: *const GitConfig,
        key: *const c_char,
        fallback_value: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `key` is NULL or NUL-terminated (the contract).
            let key = unsafe { or_null_c(key) };
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
        config: *const GitConfig,
        key: *const c_char,
        fallback_value: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: `key` is NULL or NUL-terminated (the contract).
            let key = unsafe { or_null_c(key) };
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
        out: *mut c_int,
        maps: *const git_configmap,
        map_n: usize,
        value: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            // SAFETY: per the contract.
            let (maps, value) = unsafe { (configmaps(maps, map_n), value) };
            match lookup_map_value(&maps, value.map(CStr::to_bytes)) {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
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
        type_out: *mut c_int,
        str_out: *mut *const c_char,
        maps: *const git_configmap,
        map_n: usize,
        enum_val: c_int,
    ) -> c_int {
        barrier(|| {
            let maps = if map_n == 0 {
                &[][..]
            } else {
                // SAFETY: per the contract.
                unsafe { core::slice::from_raw_parts(maps, map_n) }
            };
            match maps.iter().find(|m| m.map_value == enum_val) {
                Some(m) => {
                    // SAFETY: the outputs are writable.
                    unsafe { type_out.write(m.type_) };
                    // SAFETY: as above.
                    unsafe { str_out.write(m.str_match) };
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
    pub unsafe extern "C" fn git_config_parse_bool(out: *mut c_int, value: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            match parse_bool(value.map(CStr::to_bytes)) {
                // SAFETY: `out` is writable.
                Ok(b) => unsafe {
                    out.write(c_int::from(b));
                    0
                },
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
    pub unsafe extern "C" fn git_config_parse_int64(out: *mut i64, value: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            match parse_int64(value.map(CStr::to_bytes)) {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
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
    pub unsafe extern "C" fn git_config_parse_int32(out: *mut i32, value: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: `value` is NULL or NUL-terminated (the contract).
            let value = unsafe { c(value) };
            match parse_int32(value.map(CStr::to_bytes)) {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
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
        input: *const c_char,
        out: *mut *mut c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!input.is_null(), "in", -1);
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: per the contract.
            match normalize_name(unsafe { cstr_bytes(input) }) {
                Ok(n) => {
                    let s = crate::util::alloc::substrdup(&n);
                    if s.is_null() {
                        return -1;
                    }
                    // SAFETY: `out` is writable.
                    unsafe { out.write(s) };
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
    /// `path` is NULL or a valid `git_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_global(path: *mut GitBuf) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            unsafe {
                wrap_buf(path, |s| {
                    crate::sys::sysdir::find_global_file(s, GIT_CONFIG_FILENAME_GLOBAL)
                })
            }
        })
    }

    /// `git_config__find_global`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_global(path: *mut GitStr) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let path = unsafe { &mut *path };
            crate::util::export::status(find_global(path))
        })
    }

    /// `git_config_find_xdg`.
    ///
    /// # Safety
    ///
    /// `path` is NULL or a valid `git_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_xdg(path: *mut GitBuf) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            unsafe {
                wrap_buf(path, |s| {
                    crate::sys::sysdir::find_xdg_file(s, GIT_CONFIG_FILENAME_XDG)
                })
            }
        })
    }

    /// `git_config__find_xdg`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_xdg(path: *mut GitStr) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let path = unsafe { &mut *path };
            crate::util::export::status(find_xdg(path))
        })
    }

    /// `git_config_find_system`.
    ///
    /// # Safety
    ///
    /// `path` is NULL or a valid `git_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_find_system(path: *mut GitBuf) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            unsafe {
                wrap_buf(path, |s| {
                    crate::sys::sysdir::find_system_file(s, GIT_CONFIG_FILENAME_SYSTEM)
                })
            }
        })
    }

    /// `git_config__find_system`.
    ///
    /// # Safety
    ///
    /// `path` is a valid (non-NULL) `git_str`, as C requires (it crashes on
    /// NULL).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__find_system(path: *mut GitStr) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let path = unsafe { &mut *path };
            crate::util::export::status(find_system(path))
        })
    }

    /// `git_config__global_location`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid (non-NULL) `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__global_location(buf: *mut GitStr) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let buf = unsafe { &mut *buf };
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
    pub unsafe extern "C" fn git_config_open_default(out: *mut *mut GitConfig) -> c_int {
        barrier(|| {
            let (p, code) = match open_default() {
                Ok(c) => (CShared::into_raw(CShared::from_box(c)), 0),
                Err(e) => (core::ptr::null_mut(), e.raw()),
            };
            // SAFETY: `out` is writable.
            unsafe { out.write(p) };
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
        tx: *mut *mut crate::sys::transaction::git_transaction,
        config: *mut GitConfig,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!config.is_null(), "config", -1);
            // SAFETY: `config` is a live config (the contract).
            let config = unsafe { cfg(config) };
            // SAFETY: per the contract.
            crate::util::export::status(unsafe { config.lock(tx) })
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
            // SAFETY: per the contract: the reference `lock` made.
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
    /// `repo` is a live repository; `old_section_name` is NUL-terminated;
    /// `new_section_name` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_rename_section(
        repo: *mut git_repository,
        old_section_name: *const c_char,
        new_section_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `new_section_name` is NULL or NUL-terminated (the contract).
            let new_section_name = unsafe { c(new_section_name) };
            // SAFETY: `old_section_name` is NUL-terminated (the contract).
            let old = unsafe { cstr_bytes(old_section_name) };
            // SAFETY: `repo` is NULL or a live repository (the contract),
            // which is `Sync`.
            let repo = unsafe { repo.as_ref() };
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
        backend: *mut git_config_backend,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            if let Err(e) = check_version(version, GIT_CONFIG_BACKEND_VERSION, "git_config_backend")
            {
                return e.raw();
            }
            // SAFETY: per the contract.
            unsafe { backend.write(git_config_backend::default()) };
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
    pub fn raw(self) -> c_int {
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
    pub fn from_raw(n: c_int) -> ConfigLevel {
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

type ApiResult<T> = core::result::Result<T, Error>;

/// A NUL-terminated copy of a Rust string argument (`invalid argument` if
/// it holds a NUL, which C could not pass).
fn arg(s: &[u8]) -> ApiResult<Vec<u8>> {
    if s.contains(&0) {
        return Err(Error::new(
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

/// A configuration (`git_config`): backends by level. `Clone` takes another
/// reference to the same configuration.
///
/// `'r` is the repository the configuration came from (`'static` for one
/// made alone): a repository's file backends keep the repository for their
/// include conditions, so the configuration (and its clones, levels and
/// snapshots) cannot outlive it.
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
        // SAFETY: the caller's contract.
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
        Config::from_box(GitConfig::new())
    }

    /// `git_config_open_ondisk`: the file at `path`, at the local level.
    pub fn open(path: &std::path::Path) -> ApiResult<Config<'static>> {
        let p = arg(path_bytes(path))?;
        Ok(Config::from_box(open_ondisk(as_cstr(&p))?))
    }

    /// `git_config_open_default`: the global, XDG and system files.
    pub fn open_default() -> ApiResult<Config<'static>> {
        Ok(Config::from_box(open_default()?))
    }

    /// `git_config_add_file_ondisk`.
    pub fn add_file(
        &self,
        path: &std::path::Path,
        level: ConfigLevel,
        force: bool,
    ) -> ApiResult<()> {
        let p = arg(path_bytes(path))?;
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

    /// `git_config_get_entry`.
    pub fn get_entry(&self, name: &str) -> ApiResult<ConfigEntry> {
        let n = arg(name.as_bytes())?;
        let e = self.inner().get_entry(as_cstr(&n), true, Want::AllErrors)?;
        let e = e.ok_or_else(|| Error::from(error_notfound(name.as_bytes())))?;
        Ok(ConfigEntry::from_view(&e.view()))
    }

    fn with_value<T>(
        &self,
        name: &str,
        f: impl FnOnce(Option<&[u8]>) -> Result<T>,
    ) -> ApiResult<T> {
        let n = arg(name.as_bytes())?;
        let e = self.inner().get_entry(as_cstr(&n), true, Want::AllErrors)?;
        let e = e.ok_or_else(|| Error::from(error_notfound(name.as_bytes())))?;
        Ok(f(e.value())?)
    }

    /// `git_config_get_bool`.
    pub fn get_bool(&self, name: &str) -> ApiResult<bool> {
        self.with_value(name, parse_bool)
    }

    /// `git_config_get_int32`.
    pub fn get_i32(&self, name: &str) -> ApiResult<i32> {
        self.with_value(name, parse_int32)
    }

    /// `git_config_get_int64`.
    pub fn get_i64(&self, name: &str) -> ApiResult<i64> {
        self.with_value(name, parse_int64)
    }

    /// `git_config_get_string_buf`: the value's bytes (empty for a bare
    /// name).
    pub fn get_bytes(&self, name: &str) -> ApiResult<Vec<u8>> {
        self.with_value(name, |v| Ok(v.unwrap_or(b"").to_vec()))
    }

    /// The value as a `String` (an error if it is not UTF-8).
    pub fn get_string(&self, name: &str) -> ApiResult<String> {
        let v = self.get_bytes(name)?;
        String::from_utf8(v).map_err(|_| {
            Error::new(
                ErrorCode::GENERIC,
                ErrorClass::CONFIG,
                "the config value is not valid UTF-8",
            )
        })
    }

    /// `git_config_get_path`: the value with `~/` expanded.
    pub fn get_path(&self, name: &str) -> ApiResult<std::path::PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        let mut out = GitStr::new();
        self.with_value(name, |v| match v {
            Some(v) => parse_path(&mut out, v),
            None => Err(errors::assert_arg_failed("value")),
        })?;
        Ok(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            out.as_bytes(),
        )))
    }

    /// `git_config_get_mapped`: the value mapped by `maps`
    /// (`(GIT_CONFIGMAP_*, str_match, value)` triples).
    pub fn get_mapped(&self, name: &str, maps: &[ConfigMap<'_>]) -> ApiResult<i32> {
        self.with_value(name, |v| lookup_map_value(maps, v))
    }

    /// `git_config_set_string`.
    pub fn set_str(&self, name: &str, value: &str) -> ApiResult<()> {
        self.set_bytes(name, value.as_bytes())
    }

    /// `git_config_set_string` of bytes.
    pub fn set_bytes(&self, name: &str, value: &[u8]) -> ApiResult<()> {
        let n = arg(name.as_bytes())?;
        let v = arg(value)?;
        Ok(self.inner().set_string(as_cstr(&n), Some(as_cstr(&v)))?)
    }

    /// `git_config_set_bool`.
    pub fn set_bool(&self, name: &str, value: bool) -> ApiResult<()> {
        self.set_str(name, if value { "true" } else { "false" })
    }

    /// `git_config_set_int32`.
    pub fn set_i32(&self, name: &str, value: i32) -> ApiResult<()> {
        self.set_i64(name, i64::from(value))
    }

    /// `git_config_set_int64`.
    pub fn set_i64(&self, name: &str, value: i64) -> ApiResult<()> {
        self.set_str(name, &value.to_string())
    }

    /// `git_config_set_multivar`: set the values of `name` that match
    /// `regexp` (adding one if none does).
    pub fn set_multivar(&self, name: &str, regexp: &str, value: &str) -> ApiResult<()> {
        let n = arg(name.as_bytes())?;
        let r = arg(regexp.as_bytes())?;
        let v = arg(value.as_bytes())?;
        self.inner()
            .set_multivar(as_cstr(&n), Some(as_cstr(&r)), Some(as_cstr(&v)))?;
        Ok(())
    }

    /// `git_config_delete_entry`.
    pub fn remove(&self, name: &str) -> ApiResult<()> {
        let n = arg(name.as_bytes())?;
        self.inner().delete_entry(as_cstr(&n))?;
        Ok(())
    }

    /// `git_config_delete_multivar`.
    pub fn remove_multivar(&self, name: &str, regexp: &str) -> ApiResult<()> {
        let n = arg(name.as_bytes())?;
        let r = arg(regexp.as_bytes())?;
        self.inner()
            .delete_multivar(as_cstr(&n), Some(as_cstr(&r)))?;
        Ok(())
    }

    /// `git_config_iterator_glob_new`: every entry (whose name matches
    /// `glob`, a regular expression), from the lowest level up.
    pub fn entries(&self, glob: Option<&str>) -> ApiResult<ConfigEntries<'_>> {
        let g = glob.map(|g| arg(g.as_bytes())).transpose()?;
        let it = self.inner().iterator(g.as_deref().map(as_cstr))?;
        Ok(ConfigEntries {
            it,
            _config: core::marker::PhantomData,
        })
    }

    /// `git_config_multivar_iterator_new`: the values of `name` (that
    /// match `regexp`).
    pub fn multivar(&self, name: &str, regexp: Option<&str>) -> ApiResult<ConfigEntries<'_>> {
        let n = arg(name.as_bytes())?;
        let r = regexp.map(|r| arg(r.as_bytes())).transpose()?;
        let it = self
            .inner()
            .multivar_iterator(as_cstr(&n), r.as_deref().map(as_cstr))?;
        Ok(ConfigEntries {
            it,
            _config: core::marker::PhantomData,
        })
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
            // SAFETY: the entry the iterator holds, copied at once.
            Ok(be) => Some(Ok(ConfigEntry::from_view(&unsafe {
                EntryView::of(be.as_ptr())
            }))),
            Err(e) if e == ErrorCode::ITER_OVER => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}

/// `git_config_parse_bool` of a string.
pub fn config_parse_bool(value: &str) -> ApiResult<bool> {
    Ok(parse_bool(Some(value.as_bytes()))?)
}

/// `git_config_parse_int32` of a string.
pub fn config_parse_i32(value: &str) -> ApiResult<i32> {
    Ok(parse_int32(Some(value.as_bytes()))?)
}

/// `git_config_parse_int64` of a string.
pub fn config_parse_i64(value: &str) -> ApiResult<i64> {
    Ok(parse_int64(Some(value.as_bytes()))?)
}

/// A configuration backend implemented in Rust (`git_config_backend`).
///
/// The keys it is asked for are normalized (`section.subsection.name`,
/// lower-cased but for the subsection). Writing methods default to
/// "read-only"; `get` and `entries` are required.
pub trait ConfigBackend: Send + Sync + 'static {
    /// `open`: called once when the backend is added at `level`.
    fn open(&self, level: ConfigLevel) -> ApiResult<()> {
        let _ = level;
        Ok(())
    }

    /// `get`: the entry `key` (the last one, for a multivar), or `None`.
    fn get(&self, key: &[u8]) -> ApiResult<Option<ConfigEntry>>;

    /// `iterator`: every entry, in order.
    fn entries(&self) -> ApiResult<Vec<ConfigEntry>>;

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

    /// `lock`.
    fn lock(&self) -> ApiResult<()> {
        Err(read_only())
    }

    /// `unlock`.
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
pub fn read_only() -> Error {
    Error::new(
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
        backend: Box<dyn ConfigBackend>,
        level: std::sync::atomic::AtomicI32,
    }

    /// A C string argument.
    ///
    /// # Safety
    ///
    /// `p` is NULL or NUL-terminated, for the call.
    unsafe fn b<'a>(p: *const c_char) -> &'a [u8] {
        if p.is_null() {
            return b"";
        }
        // SAFETY: per the contract.
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
        fn get(&self, key: &[u8], out: *mut *mut git_config_backend_entry) -> c_int {
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
                            let p = ConfigList::lend(&l, &l.entries()[0]);
                            // SAFETY: `out` is C's writable out-parameter.
                            unsafe { out.write(p) };
                            0
                        }
                    }
                }
            }
        }

        fn iterator(&self, out: *mut *mut git_config_iterator) -> c_int {
            match self.backend.entries() {
                Err(e) => raise(&e).raw(),
                Ok(entries) => match list_of(&entries) {
                    Err(c) => c.raw(),
                    Ok(l) => {
                        // SAFETY: `out` is C's writable out-parameter.
                        unsafe { out.write(crate::config_list::iterator_new(l)) };
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
            // SAFETY: `key` is NUL-terminated (the slot's contract).
            |s: &State| s.get(unsafe { b(key) }, out),
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
            // SAFETY: as for `set`; `value` may be NULL.
            |s: &State| {
                // SAFETY: as for `set`.
                let (name, regexp) = (unsafe { b(name) }, unsafe { b(regexp) });
                // SAFETY: as above; `value` may be NULL.
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
            |s: &State| s.iterator(out),
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
            backend: Box::new(backend),
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
            ConfigMap {
                kind: GIT_CONFIGMAP_FALSE,
                str_match: None,
                map_value: 10,
            },
            ConfigMap {
                kind: GIT_CONFIGMAP_TRUE,
                str_match: None,
                map_value: 11,
            },
            ConfigMap {
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
