//! Port of `src/libgit2/mwindow.c`: memory-mapped windows over pack files,
//! with a process-wide budget, and the process-wide cache of open packs.
//!
//! # Types
//!
//! | C | Rust | Kind (PORTING §4.1) |
//! |---|---|---|
//! | `git_mwindow` | [`GitMwindow`] | opaque: C only holds cursors (`git_mwindow *`) |
//! | `git_mwindow_file` | [`GitMwindowFile`] | mirrored: embedded in `git_pack_file`, whose `mwf.fd` and `mwf.size` the indexer reads and writes |
//! | `git_mwindow_ctl` | [`GitMwindowCtl`] | mirrored: the global `git_mwindow__mem_ctl`, which a test reads under the mutex |
//! | `git_mwindow_packmap` | [`PackMap`] | mirrored: the global `git_mwindow__pack_cache`, which a test iterates |
//!
//! # Locking
//!
//! `git_mwindow__mutex` (C-visible: a test takes it) protects the global
//! control block, the pack cache, every registered file's list of windows
//! and every window's fields. [`Ctl`] is the guard that proves it is held:
//! every access to those goes through one. Each file's own `lock`
//! (`git_mwindow_file.lock`) protects its descriptor; the lock order is the
//! C's: a pack's lock, then its file's lock, then the global mutex. Nothing
//! takes a file lock while holding the global mutex, except with
//! `try_lock` (see [`file_register`]).
//!
//! # Mapped memory
//!
//! A window is a private read-only mapping of a pack. A caller reads it
//! through a [`Cursor`], which holds the window (`inuse_cnt`) while the
//! returned slice is borrowed: a window in use is never unmapped (neither
//! by the LRU closing, which skips used windows, nor by
//! [`free_all`], which refuses to free one, as C's assertion does). The
//! slice is therefore valid for as long as it is borrowed, as far as this
//! process is concerned.
//!
//! What a mapping cannot rule out is another process changing the file: git
//! and libgit2 never modify a pack in place (they write a new file and
//! rename it), and this module, like the C, relies on that. A file
//! truncated under the mapping makes a read of the lost pages fault
//! (`SIGBUS`), as in the C; a file rewritten in place may change the bytes
//! of a borrowed slice. That is the assumption every `mmap` reader makes;
//! the Rust adds no other. The indexer (C) appends to the pack it is
//! writing while it reads windows of it: it writes only past the windows'
//! ends, and rewrites the trailer only once it has freed the windows, and
//! the Rust holds no slice across its writes.

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicI32, AtomicI64, AtomicPtr, AtomicUsize, Ordering};

use crate::oid::git_oid_t;
use crate::pack::{self, GitPackFile};
use crate::util::errors::{ErrorClass, Result};
use crate::util::futils;
use crate::util::hashmap::{CStrKey, GitHashMap};
use crate::util::posix::{self, GitMap};
use crate::util::thread::{GitMutex, GitMutexGuard};
use crate::util::vector::GitVector;
use crate::{git_assert, git_error_set};

/// `DEFAULT_WINDOW_SIZE` (64-bit).
pub const DEFAULT_WINDOW_SIZE: usize = 1024 * 1024 * 1024;
/// `DEFAULT_MAPPED_LIMIT` (64-bit).
pub const DEFAULT_MAPPED_LIMIT: usize = 1024 * 1024 * 8192;
/// `DEFAULT_FILE_LIMIT`: unlimited.
pub const DEFAULT_FILE_LIMIT: usize = 0;

/// `git_mwindow__window_size` (`GIT_OPT_SET_MWINDOW_SIZE`). `settings.c`
/// writes it with plain stores; an `AtomicUsize` has `size_t`'s layout.
#[unsafe(no_mangle)]
pub static git_mwindow__window_size: AtomicUsize = AtomicUsize::new(DEFAULT_WINDOW_SIZE);
/// `git_mwindow__mapped_limit` (`GIT_OPT_SET_MWINDOW_MAPPED_LIMIT`).
#[unsafe(no_mangle)]
pub static git_mwindow__mapped_limit: AtomicUsize = AtomicUsize::new(DEFAULT_MAPPED_LIMIT);
/// `git_mwindow__file_limit` (`GIT_OPT_SET_MWINDOW_FILE_LIMIT`).
#[unsafe(no_mangle)]
pub static git_mwindow__file_limit: AtomicUsize = AtomicUsize::new(DEFAULT_FILE_LIMIT);

/// `git_mwindow__mutex`: protects [`git_mwindow__mem_ctl`],
/// [`git_mwindow__pack_cache`], and every window (see the module docs).
/// Statically initialized, so `git_mwindow_global_init` need not (and does
/// not) initialize it again.
#[unsafe(no_mangle)]
pub static git_mwindow__mutex: GitMutex = GitMutex::new();

/// A global that C sees and that is only accessed with
/// [`git_mwindow__mutex`] held.
#[repr(transparent)]
pub struct MutexProtected<T>(UnsafeCell<T>);

// SAFETY: every access to the contents, Rust's ([`Ctl`]) and C's (the
// tests), is made with `git_mwindow__mutex` held, which serializes them and
// orders them across threads. The contents (raw pointers to files and
// packs that live on the heap) are not tied to a thread.
unsafe impl<T> Sync for MutexProtected<T> {}

impl<T> MutexProtected<T> {
    const fn new(v: T) -> Self {
        MutexProtected(UnsafeCell::new(v))
    }
}

/// `git_mwindow_ctl`, mirrored.
#[repr(C)]
pub struct GitMwindowCtl {
    /// `mapped`: bytes mapped by all windows.
    pub mapped: usize,
    /// `open_windows`.
    pub open_windows: c_uint,
    /// `mmap_calls`.
    pub mmap_calls: c_uint,
    /// `peak_open_windows`.
    pub peak_open_windows: c_uint,
    /// `peak_mapped`.
    pub peak_mapped: usize,
    /// `used_ctr`: the clock of `last_used`.
    pub used_ctr: usize,
    /// `windowfiles`: the registered files (`git_mwindow_file *`).
    pub windowfiles: GitVector,
}

/// `git_mwindow__mem_ctl`.
#[unsafe(no_mangle)]
pub static git_mwindow__mem_ctl: MutexProtected<GitMwindowCtl> =
    MutexProtected::new(GitMwindowCtl {
        mapped: 0,
        open_windows: 0,
        mmap_calls: 0,
        peak_open_windows: 0,
        peak_mapped: 0,
        used_ctr: 0,
        windowfiles: GitVector::new(),
    });

/// `git_mwindow_packmap`: pack name to pack. The key is the pack's own
/// `pack_name`, which lives as long as the pack, and a pack is in the map
/// only while it lives.
pub type PackMap = GitHashMap<CStrKey<'static>, *mut GitPackFile>;

/// `git_mwindow__pack_cache`: every pack opened through
/// [`get_pack`], by name, so that repositories share them.
#[unsafe(no_mangle)]
pub static git_mwindow__pack_cache: MutexProtected<PackMap> =
    MutexProtected::new(GitHashMap::new());

crate::git_hashmap_exports! {
    map: PackMap, key: *const c_char, val: *mut GitPackFile,
    size: git_mwindow_packmap_size, contains: git_mwindow_packmap_contains,
    remove: git_mwindow_packmap_remove, clear: git_mwindow_packmap_clear,
    dispose: git_mwindow_packmap_dispose, get: git_mwindow_packmap_get,
    put: git_mwindow_packmap_put, iterate: git_mwindow_packmap_iterate,
    foreach: git_mwindow_packmap_foreach,
}

/// `git_mwindow`: one mapped window of a file. Opaque to C. Every field is
/// read and written only with the global mutex held (see [`Ctl`]).
/// (`#[repr(C)]` only so that C may hold pointers to it.)
#[repr(C)]
pub struct GitMwindow {
    next: *mut GitMwindow,
    window_map: GitMap,
    offset: i64,
    last_used: usize,
    inuse_cnt: usize,
}

/// `git_mwindow_file`, mirrored: what is mapped of one file.
///
/// `fd` and `size` are atomics (with `int`'s and `off64_t`'s layout)
/// because C writes them while other threads may read them: `fd` under
/// `lock`, `size` by the indexer on its own pack. `windows` is read and
/// written only with the global mutex held.
#[repr(C)]
pub struct GitMwindowFile {
    /// `lock`: protects `fd`.
    pub lock: GitMutex,
    windows: AtomicPtr<GitMwindow>,
    /// `fd`: the open file, or -1.
    pub fd: AtomicI32,
    /// `size`: the file's size.
    pub size: AtomicI64,
}

impl GitMwindowFile {
    /// A file with no descriptor and no windows.
    pub const fn new() -> GitMwindowFile {
        GitMwindowFile {
            lock: GitMutex::new(),
            windows: AtomicPtr::new(ptr::null_mut()),
            fd: AtomicI32::new(-1),
            size: AtomicI64::new(0),
        }
    }

    /// The descriptor (`fd`).
    #[inline]
    pub fn fd(&self) -> c_int {
        self.fd.load(Ordering::Relaxed)
    }

    /// Set the descriptor; the caller holds `lock`, or the file is its own.
    #[inline]
    pub(crate) fn set_fd(&self, fd: c_int) {
        self.fd.store(fd, Ordering::Relaxed);
    }

    /// The size (`size`).
    #[inline]
    pub fn size(&self) -> i64 {
        self.size.load(Ordering::Relaxed)
    }

    /// Set the size.
    #[inline]
    pub(crate) fn set_size(&self, size: i64) {
        self.size.store(size, Ordering::Relaxed);
    }
}

impl Default for GitMwindowFile {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// The global mutex and what it protects
// ---------------------------------------------------------------------------

/// The global mutex, held: the only way to reach the control block, the
/// pack cache, the files' window lists and the windows.
pub(crate) struct Ctl {
    _guard: GitMutexGuard<'static>,
}

impl Ctl {
    /// Lock `git_mwindow__mutex` (C's `git_mutex_lock(&git_mwindow__mutex)`,
    /// which cannot fail for a default mutex).
    pub(crate) fn lock() -> Ctl {
        Ctl {
            _guard: git_mwindow__mutex.lock(),
        }
    }

    /// The control block.
    fn ctl(&mut self) -> &mut GitMwindowCtl {
        // SAFETY: the global mutex is held (`self`), so no other thread
        // reaches the block, and C reads it only under the mutex; `&mut
        // self` makes this the only reference on this thread.
        unsafe { &mut *git_mwindow__mem_ctl.0.get() }
    }

    /// The pack cache.
    pub(crate) fn packs(&mut self) -> &mut PackMap {
        // SAFETY: as in `ctl`, for `git_mwindow__pack_cache`.
        unsafe { &mut *git_mwindow__pack_cache.0.get() }
    }

    fn windows(&mut self, mwf: &GitMwindowFile) -> *mut GitMwindow {
        let _ = self;
        mwf.windows.load(Ordering::Relaxed)
    }

    fn set_windows(&mut self, mwf: &GitMwindowFile, w: *mut GitMwindow) {
        let _ = self;
        mwf.windows.store(w, Ordering::Relaxed);
    }

    /// A window, for the extent of a borrow of the lock.
    ///
    /// # Safety
    ///
    /// `w` is a live window (one of a file's list, or held by a cursor).
    #[allow(clippy::mut_from_ref)]
    unsafe fn win(&mut self, w: *mut GitMwindow) -> &mut GitMwindow {
        // SAFETY: a live window (this fn's contract), whose fields are reached only
        // with the global mutex held, which `self` proves; `&mut self`
        // makes the reference unique on this thread.
        unsafe { &mut *w }
    }

    /// The registered file at `i`.
    fn file(&mut self, i: usize) -> Option<&GitMwindowFile> {
        let p = self.ctl().windowfiles.get(i)?;
        // SAFETY: a registered file stays live and at its address until it
        // is deregistered ([`file_register`]'s contract), which needs the
        // mutex; the reference ends with the borrow of the lock.
        Some(unsafe { &*p.cast::<GitMwindowFile>() })
    }
}

/// `git_mwindow_contains`: whether `[offset, offset + extra]` lies in the
/// window.
#[inline]
fn contains(w: &GitMwindow, offset: i64, extra: i64) -> bool {
    contains_range(w.offset, w.window_map.len, offset, extra)
}

/// [`contains`] over a window's offset and length.
#[inline]
fn contains_range(win_off: i64, win_len: usize, offset: i64, extra: i64) -> bool {
    // C's `offset + extra` overflows (UB) for huge values; it does not
    // hold them here.
    let end = win_off.saturating_add(win_len as i64);
    win_off <= offset && offset.checked_add(extra).is_some_and(|e| e <= end)
}

const LRU: isize = -1;
const MRU: isize = 1;

/// `git_mwindow_scan_recently_used`: the least (`LRU`) or most (`MRU`)
/// recently used window of `mwf` that is not in use, compared against
/// `*out_window` if it is set. With `only_unused`, a file with a window in
/// use has none.
fn scan_recently_used(
    ctl: &mut Ctl,
    mwf: &GitMwindowFile,
    out_window: &mut *mut GitMwindow,
    out_last: Option<&mut *mut GitMwindow>,
    only_unused: bool,
    comparison_sign: isize,
) -> bool {
    let mut lru_window = *out_window;
    let mut lru_last = out_last.as_deref().copied().unwrap_or(ptr::null_mut());
    let mut found = false;

    let mut w_last: *mut GitMwindow = ptr::null_mut();
    let mut w = ctl.windows(mwf);
    while !w.is_null() {
        // SAFETY: `w` is in `mwf`'s list.
        let (inuse, last_used, next) = unsafe {
            let win = ctl.win(w);
            (win.inuse_cnt, win.last_used, win.next)
        };
        if inuse != 0 {
            if only_unused {
                return false;
            }
        } else {
            // C compares `comparison_sign * w->last_used`, a `size_t`
            // product: for the LRU scan, `-last_used` modulo 2^64.
            let lru_used = if lru_window.is_null() {
                0
            } else {
                // SAFETY: `lru_window` is a window of a registered file.
                unsafe { ctl.win(lru_window).last_used }
            };
            if lru_window.is_null() || (comparison_sign as usize).wrapping_mul(last_used) > lru_used
            {
                lru_window = w;
                lru_last = w_last;
                found = true;
            }
        }
        w_last = w;
        w = next;
    }

    if !found {
        return false;
    }
    *out_window = lru_window;
    if let Some(l) = out_last {
        *l = lru_last;
    }
    true
}

/// `git_mwindow_close_lru_window_locked`: unmap the least recently used
/// window not in use, of all files.
fn close_lru_window_locked(ctl: &mut Ctl) -> Result<()> {
    let mut lru_window: *mut GitMwindow = ptr::null_mut();
    let mut lru_last: *mut GitMwindow = ptr::null_mut();
    let mut list: Option<*const GitMwindowFile> = None;

    let mut i = 0;
    while let Some(cur) = ctl.file(i) {
        let cur: *const GitMwindowFile = cur;
        // SAFETY: a registered file (as in `Ctl::file`).
        let cur_ref = unsafe { &*cur };
        if scan_recently_used(
            ctl,
            cur_ref,
            &mut lru_window,
            Some(&mut lru_last),
            false,
            LRU,
        ) {
            list = Some(cur);
        }
        i += 1;
    }

    let Some(list) = list else {
        return Err(git_error_set!(
            ErrorClass::OS,
            "failed to close memory window; couldn't find LRU"
        ));
    };
    // SAFETY: `list` is a registered file.
    let list = unsafe { &*list };
    // SAFETY: `lru_window` is a window of `list`.
    let (len, next) = unsafe {
        let w = ctl.win(lru_window);
        (w.window_map.len, w.next)
    };
    ctl.ctl().mapped -= len;
    if lru_last.is_null() {
        ctl.set_windows(list, next);
    } else {
        // SAFETY: `lru_last` is the window before it in `list`.
        unsafe { ctl.win(lru_last).next = next };
    }
    // SAFETY: unlinked above; not in use (the scan skips used windows).
    unsafe { free_window(lru_window) };
    ctl.ctl().open_windows -= 1;
    Ok(())
}

/// Unmap and free a window that is unlinked from its list and not in use.
///
/// # Safety
///
/// `w` came from [`new_window_locked`], is in no list, and no cursor holds
/// it.
unsafe fn free_window(w: *mut GitMwindow) {
    // SAFETY: `w` is the `Box::into_raw` of `new_window_locked` (this fn's
    // contract), freed only here; in no list and held by no cursor, no
    // pointer to it remains in use, so the box is ours alone.
    let mut w = unsafe { Box::from_raw(w) };
    // SAFETY: the window's own mapping, unmapped once (the box goes with
    // it); no slice into it lives, as slices are lent only by a cursor
    // holding the window (`Cursor::open`), and none does.
    unsafe { posix::munmap(&mut w.window_map) };
}

/// `git_mwindow_find_lru_file_locked`: the file all of whose windows are
/// unused, whose most recently used window is the least recently used of
/// all such files.
/// Files in `skip` are not considered (the closer could not lock them).
fn find_lru_file_locked(
    ctl: &mut Ctl,
    skip: &[*const GitMwindowFile],
) -> Result<*const GitMwindowFile> {
    let mut lru_file: *const GitMwindowFile = ptr::null();
    let mut lru_window: *mut GitMwindow = ptr::null_mut();
    let mut i = 0;
    while let Some(current) = ctl.file(i) {
        let current: *const GitMwindowFile = current;
        i += 1;
        if skip.contains(&current) {
            continue;
        }
        let mut mru_window = ptr::null_mut();
        // SAFETY: a registered file.
        if !scan_recently_used(ctl, unsafe { &*current }, &mut mru_window, None, true, MRU) {
            continue;
        }
        // SAFETY: a window of a registered file.
        let mru_used = unsafe { ctl.win(mru_window).last_used };
        // SAFETY: as above.
        let replace = lru_window.is_null() || unsafe { ctl.win(lru_window).last_used } > mru_used;
        if replace {
            lru_window = mru_window;
            lru_file = current;
        }
    }
    if lru_file.is_null() {
        return Err(git_error_set!(
            ErrorClass::OS,
            "failed to close memory window file; couldn't find LRU"
        ));
    }
    Ok(lru_file)
}

/// Where `new_window_locked` puts the window for `offset` of a file of
/// `size` bytes: its offset (aligned to half the window size) and length
/// (negative past the end of the file).
fn window_bounds(size: i64, offset: i64) -> (i64, i64) {
    let window_size = git_mwindow__window_size.load(Ordering::Relaxed);
    // C divides by `window_size / 2`, which is 0 for a window size below 2
    // (SIGFPE): see DEFECTS.md.
    let walign = (window_size / 2).max(1) as i64;
    let w_offset = (offset / walign) * walign;
    let mut len = size.wrapping_sub(w_offset);
    if len > window_size as i64 {
        len = window_size as i64;
    }
    (w_offset, len)
}

/// `new_window_locked`: map a window of `fd` (of `size` bytes) that starts
/// at or before `offset`, closing LRU windows while over the mapped limit.
fn new_window_locked(ctl: &mut Ctl, fd: c_int, size: i64, offset: i64) -> Option<*mut GitMwindow> {
    let (w_offset, len) = window_bounds(size, offset);
    // C casts a negative length (an offset past the end) to `size_t`; the
    // mapping then fails.
    let len = len as usize;

    ctl.ctl().mapped = ctl.ctl().mapped.wrapping_add(len);
    // Close LRU windows while over the limit, and there are some.
    while git_mwindow__mapped_limit.load(Ordering::Relaxed) < ctl.ctl().mapped {
        if close_lru_window_locked(ctl).is_err() {
            break;
        }
    }

    // `mapped_limit` is a soft limit: if no window can be closed, map
    // anyway.
    let map = match futils::mmap_ro(fd, w_offset, len) {
        Ok(m) => m,
        Err(_) => {
            // The first failure may be fragmentation: free what we can and
            // try again.
            while close_lru_window_locked(ctl).is_ok() {
                continue;
            }
            match futils::mmap_ro(fd, w_offset, len) {
                Ok(m) => m,
                Err(_) => {
                    // C leaves `len` counted in `mapped`: see DEFECTS.md.
                    ctl.ctl().mapped = ctl.ctl().mapped.wrapping_sub(len);
                    return None;
                }
            }
        }
    };

    let c = ctl.ctl();
    c.mmap_calls = c.mmap_calls.wrapping_add(1);
    c.open_windows = c.open_windows.wrapping_add(1);
    if c.mapped > c.peak_mapped {
        c.peak_mapped = c.mapped;
    }
    if c.open_windows > c.peak_open_windows {
        c.peak_open_windows = c.open_windows;
    }

    Some(Box::into_raw(Box::new(GitMwindow {
        next: ptr::null_mut(),
        window_map: map,
        offset: w_offset,
        last_used: 0,
        inuse_cnt: 0,
    })))
}

// ---------------------------------------------------------------------------
// Cursors
// ---------------------------------------------------------------------------

/// A cursor over the windows of a file (C's `git_mwindow *` variable,
/// passed around as `git_mwindow **`): NULL, or a window it holds in use.
///
/// The window it holds is never unmapped (see the module docs), so the
/// slice [`open`](Self::open) returns is valid while it is borrowed; the
/// borrow also keeps the cursor from moving to another window or closing
/// meanwhile. A cursor of Rust's own closes on drop; one that C owns (seen
/// through [`from_raw_mut`](Self::from_raw_mut)) is closed by C.
#[repr(transparent)]
pub struct Cursor(*mut GitMwindow);

// SAFETY: the one field is NULL or a window the cursor holds: a heap
// `GitMwindow` whose fields any thread reads and writes only under the
// global mutex (`Ctl`), and whose mapping, not tied to a thread, stays
// mapped while it is held. So a cursor may move to, and release its
// window on, another thread. (Not `Sync`: `open` takes `&mut self`.)
unsafe impl Send for Cursor {}

impl Cursor {
    /// A cursor that holds no window.
    pub const fn new() -> Cursor {
        Cursor(ptr::null_mut())
    }

    /// A `git_mwindow **` from C, as a cursor.
    ///
    /// # Safety
    ///
    /// `p` is a valid, aligned, writable `git_mwindow *` that is NULL or a
    /// window a cursor holds (that `git_mwindow_open` stored and no
    /// `git_mwindow_close` released), not accessed otherwise for `'a`.
    pub unsafe fn from_raw_mut<'a>(p: *mut *mut GitMwindow) -> &'a mut Cursor {
        // SAFETY: `Cursor` is `#[repr(transparent)]` over `*mut GitMwindow`,
        // so `p` is valid and aligned for it; its value is NULL or a held
        // window, the cursor's invariant; nothing else accesses it for `'a`
        // (this fn's contract), so the `&mut` is unique.
        unsafe { &mut *p.cast::<Cursor>() }
    }

    /// Whether the cursor holds a window.
    pub fn is_open(&self) -> bool {
        !self.0.is_null()
    }

    /// `git_mwindow_open`: the bytes of `mwf` from `offset` to the end of a
    /// window that holds `[offset, offset + extra]` (or that a new window
    /// gives, which may hold less: C does not check), or `None`.
    ///
    /// C reports their count as an `unsigned int`; the slice has that
    /// (truncated) length.
    ///
    /// `mwf` stays at its address while its windows exist (it is in a
    /// heap-allocated pack, or registered: [`file_register`]).
    pub(crate) fn open<'c>(
        &'c mut self,
        mwf: &GitMwindowFile,
        offset: i64,
        extra: usize,
    ) -> Option<&'c [u8]> {
        let (p, left) = self.open_raw(mwf, offset, extra)?;
        // SAFETY: `p` points into the mapping of the window this cursor now
        // holds, which stays mapped and unwritten (module docs) while the
        // cursor holds it, and the cursor is borrowed for `'c`; `left` bytes
        // from `p` are within the mapping (`open_raw`).
        Some(unsafe { core::slice::from_raw_parts(p, left as usize) })
    }

    /// [`open`](Self::open) without the borrow: the pointer and C's `left`.
    fn open_raw(
        &mut self,
        mwf: &GitMwindowFile,
        offset: i64,
        extra: usize,
    ) -> Option<(*const u8, c_uint)> {
        let mut ctl = Ctl::lock();
        let mut w = self.0;

        // SAFETY: a held window is live.
        let hit = !w.is_null() && contains(unsafe { ctl.win(w) }, offset, extra as i64);
        if !hit {
            if !w.is_null() {
                // SAFETY: the cursor holds `w`.
                unsafe { ctl.win(w).inuse_cnt -= 1 };
                // C leaves the cursor pointing at the released window when
                // the open then fails (and its caller closes it again): see
                // DEFECTS.md.
                self.0 = ptr::null_mut();
            }

            w = ctl.windows(mwf);
            while !w.is_null() {
                // SAFETY: `w` is in `mwf`'s list.
                let win = unsafe { ctl.win(w) };
                if contains(win, offset, extra as i64) {
                    break;
                }
                w = win.next;
            }

            if w.is_null() {
                // A new window holds `offset` only if `offset` is in the
                // file; C does not check, and returns a pointer outside the
                // mapping (see DEFECTS.md). A window that cannot be mapped at
                // all (past the end) fails in `mmap`, as in C.
                let size = mwf.size();
                let (w_offset, len) = window_bounds(size, offset);
                if len > 0 && !(w_offset <= offset && offset < w_offset + len) {
                    let _ = git_error_set!(
                        ErrorClass::ODB,
                        "invalid pack window offset {} (file size {})",
                        offset,
                        size
                    );
                    return None;
                }
                w = new_window_locked(&mut ctl, mwf.fd(), size, offset)?;
                let head = ctl.windows(mwf);
                // SAFETY: the new window, not yet in any list.
                unsafe { ctl.win(w).next = head };
                ctl.set_windows(mwf, w);
            }
        }

        if w != self.0 {
            let used = ctl.ctl().used_ctr;
            ctl.ctl().used_ctr = used.wrapping_add(1);
            // SAFETY: `w` is in `mwf`'s list.
            let win = unsafe { ctl.win(w) };
            win.last_used = used;
            win.inuse_cnt += 1;
            self.0 = w;
        }

        // SAFETY: the window the cursor now holds.
        let win = unsafe { ctl.win(w) };
        // `offset` is within the mapping: a window found holds it
        // (`contains`), and a new one does too (checked above). Checked
        // again, as the slice's bounds rest on it.
        let len = win.window_map.len;
        let rel = usize::try_from(offset - win.offset)
            .ok()
            .filter(|&r| r <= len)
            .expect("the window holds the offset");
        let left = (len - rel) as c_uint;
        let p = win
            .window_map
            .data
            .cast::<u8>()
            .cast_const()
            .wrapping_add(rel);
        Some((p, left))
    }

    /// `git_mwindow_close`: release the window, if any.
    pub fn close(&mut self) {
        if !self.0.is_null() {
            let mut ctl = Ctl::lock();
            // SAFETY: the cursor holds the window, so it is live.
            unsafe { ctl.win(self.0).inuse_cnt -= 1 };
            drop(ctl);
            self.0 = ptr::null_mut();
        }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Cursor {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// `git_mwindow_free_all_locked`: deregister `mwf` and unmap its windows;
/// fails (as C's assertion) at a window still in use, which it leaves.
fn free_all_locked(ctl: &mut Ctl, mwf: &GitMwindowFile) -> Result<()> {
    let target: *const GitMwindowFile = mwf;
    let c = ctl.ctl();
    let found = c
        .windowfiles
        .iter()
        .position(|p| ptr::eq(p.cast_const().cast(), target));
    if let Some(i) = found {
        let _ = c.windowfiles.remove(i);
    }
    if c.windowfiles.is_empty() {
        c.windowfiles.dispose();
    }

    loop {
        let w = ctl.windows(mwf);
        if w.is_null() {
            break;
        }
        // SAFETY: `w` is in `mwf`'s list.
        let (inuse, len, next) = unsafe {
            let win = ctl.win(w);
            (win.inuse_cnt, win.window_map.len, win.next)
        };
        git_assert!(inuse == 0, "w->inuse_cnt == 0");
        let c = ctl.ctl();
        c.mapped -= len;
        c.open_windows -= 1;
        ctl.set_windows(mwf, next);
        // SAFETY: unlinked above, and not in use.
        unsafe { free_window(w) };
    }
    Ok(())
}

/// `git_mwindow_free_all`: [`free_all_locked`] under the mutex.
pub fn free_all(mwf: &GitMwindowFile) -> Result<()> {
    let mut ctl = Ctl::lock();
    free_all_locked(&mut ctl, mwf)
}

/// `git_mwindow_file_register`: add `mwf` to the files whose windows the
/// LRU may close; with a file limit, first close the least recently used
/// files that are not in use until under it.
///
/// C closes those files' descriptors after releasing the mutex, under each
/// file's lock: the file may be freed meanwhile (a use after free). The
/// Rust closes them before releasing it, taking each file's lock only if it
/// is free (`try_lock`, so the lock order cannot deadlock). A file whose
/// lock is held is left registered and open, and the next least recently
/// used file is closed instead (see DEFECTS.md); every open file stays
/// registered, so the mapped limit and the file limit keep applying.
///
/// # Safety
///
/// `mwf` stays live and at its address until it is deregistered
/// ([`file_deregister`], [`free_all`]).
pub unsafe fn file_register(mwf: &GitMwindowFile) -> Result<()> {
    let mut ctl = Ctl::lock();

    if ctl.ctl().windowfiles.is_empty() {
        ctl.ctl().windowfiles = GitVector::with_capacity(8, None)?;
    }

    let limit = git_mwindow__file_limit.load(Ordering::Relaxed);
    if limit != 0 {
        // Files whose lock is held (a reader is opening a window of it):
        // left registered and open, and another file is closed instead.
        let mut busy: Vec<*const GitMwindowFile> = Vec::new();
        while limit <= ctl.ctl().windowfiles.len() {
            let Ok(lru_file) = find_lru_file_locked(&mut ctl, &busy) else {
                break;
            };
            // SAFETY: a registered file.
            let file = unsafe { &*lru_file };
            let Some(_g) = file.lock.try_lock() else {
                if busy.try_reserve(1).is_err() {
                    // As C: exceeding the limit beats any worse outcome.
                    break;
                }
                busy.push(lru_file);
                continue;
            };
            // Its windows are all unused (`find_lru_file_locked`), and stay
            // so: windows are opened under the file's lock, which we hold.
            let _ = free_all_locked(&mut ctl, file);
            // SAFETY: the file's descriptor, which it owns, closed with its
            // lock held and then set to -1 (as C does): the next use opens
            // and registers it again.
            let _ = unsafe { posix::close(file.fd()) };
            file.set_fd(-1);
        }
    }

    let p: *const GitMwindowFile = mwf;
    ctl.ctl().windowfiles.insert(p.cast_mut().cast())
}

/// `git_mwindow_file_deregister`.
pub fn file_deregister(mwf: &GitMwindowFile) {
    let mut ctl = Ctl::lock();
    let target: *const GitMwindowFile = mwf;
    let c = ctl.ctl();
    let found = c
        .windowfiles
        .iter()
        .position(|p| ptr::eq(p.cast_const().cast(), target));
    if let Some(i) = found {
        let _ = c.windowfiles.remove(i);
    }
}

// ---------------------------------------------------------------------------
// The pack cache
// ---------------------------------------------------------------------------

/// `git_mwindow_get_pack`: the pack of the index `path`, shared with every
/// other user of it in the process, with one more reference.
pub fn get_pack(path: &CStr, oid_type: git_oid_t) -> Result<*mut GitPackFile> {
    let packname = pack::packfile_name(path.to_bytes())?;
    let mut ctl = Ctl::lock();

    // SAFETY: `packname` is a NUL-terminated string that lives (unwritten)
    // until the lookup is done; the map does not keep the key.
    let key = unsafe { CStrKey::from_ptr(NonNull::from(packname.as_c_str()).cast()) };
    let found = ctl.packs().get(&key).copied();
    if let Some(pack) = found {
        drop(packname);
        // SAFETY: a pack in the cache is live (it leaves the cache before
        // it is freed, under the mutex we hold).
        unsafe { (*pack).refcount.inc() };
        return Ok(pack);
    }
    drop(packname);

    let pack = pack::packfile_alloc(path.to_bytes(), oid_type)?;
    // SAFETY: the pack we just allocated, live.
    let p = unsafe { &*pack };
    p.refcount.inc();
    let key = p.pack_name_key();
    if let Err(e) = ctl.packs().put(key, pack) {
        drop(ctl);
        // SAFETY: ours alone: never shared.
        unsafe { pack::packfile_free(pack, false) };
        return Err(e);
    }
    Ok(pack)
}

/// `git_mwindow_put_pack`: drop a reference from [`get_pack`]; the last
/// one removes the pack from the cache and frees it.
///
/// # Safety
///
/// `pack` is a pack from [`get_pack`] whose reference the caller gives up.
pub unsafe fn put_pack(pack: *mut GitPackFile) -> Result<()> {
    let mut ctl = Ctl::lock();
    // SAFETY: the caller holds a reference, so the pack is live.
    let p = unsafe { &*pack };
    let key = p.pack_name_key();
    git_assert!(
        ctl.packs().contains(&key),
        "git_mwindow_packmap_contains(&git_mwindow__pack_cache, pack->pack_name)"
    );
    let count = p.refcount.dec();
    let to_delete = if count == 0 {
        let _ = ctl.packs().remove(&key);
        pack
    } else {
        ptr::null_mut()
    };
    drop(ctl);
    if !to_delete.is_null() {
        // SAFETY: the last reference, and out of the cache: no one else
        // reaches the pack.
        unsafe { pack::packfile_free(to_delete, false) };
    }
    Ok(())
}

/// A snapshot of the process's window accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MwindowStats {
    /// Bytes mapped by all windows.
    pub mapped: usize,
    /// Windows mapped.
    pub open_windows: u32,
    /// The most windows mapped at once.
    pub peak_open_windows: u32,
    /// Files registered (whose windows the LRU may close).
    pub registered_files: usize,
    /// Packs of the pack cache whose file is open.
    pub open_pack_files: usize,
    /// Packs of the pack cache whose file is open and registered.
    pub registered_pack_files: usize,
}

/// The window accounting now (under the global mutex).
pub fn stats() -> MwindowStats {
    let mut ctl = Ctl::lock();
    let c = ctl.ctl();
    let (mapped, open_windows, peak_open_windows) = (c.mapped, c.open_windows, c.peak_open_windows);
    let files: Vec<*const GitMwindowFile> = c
        .windowfiles
        .iter()
        .map(|p| p.cast_const().cast())
        .collect();
    let mut open_pack_files = 0;
    let mut registered_pack_files = 0;
    for &p in ctl.packs().values() {
        // SAFETY: a pack in the cache is live while the mutex is held.
        let mwf = unsafe { &(*p).mwf };
        if mwf.fd() >= 0 {
            open_pack_files += 1;
            if files.contains(&ptr::from_ref(mwf)) {
                registered_pack_files += 1;
            }
        }
    }
    MwindowStats {
        mapped,
        open_windows,
        peak_open_windows,
        registered_files: files.len(),
        open_pack_files,
        registered_pack_files,
    }
}

extern "C" fn global_shutdown() {
    let mut ctl = Ctl::lock();
    ctl.packs().dispose();
}

/// `git_mwindow_global_init`.
pub fn global_init() -> Result<()> {
    crate::util::runtime::shutdown_register(global_shutdown)
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// The exports of `mwindow.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};

    /// `git_mwindow_global_init`.
    ///
    /// # Safety
    ///
    /// Called by `git_libgit2_init`, with the runtime's lock held.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_global_init() -> c_int {
        barrier(|| status(global_init()))
    }

    /// `git_mwindow_contains`.
    ///
    /// (`win` stays a raw pointer: another thread may hold a `&mut` to the
    /// window, under the mutex, to update its use count and clock, which
    /// an `&GitMwindow` here would alias. Only `offset` and `window_map`,
    /// written once before the window is published, are read.)
    ///
    /// # Safety
    ///
    /// `win` is NULL or a live window that stays mapped during the call
    /// (the caller holds it in a cursor, or holds `git_mwindow__mutex`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_contains(
        win: *const GitMwindow,
        offset: i64,
        extra: i64,
    ) -> c_int {
        if win.is_null() {
            let _ = crate::util::errors::assert_arg_failed("win");
            return -1;
        }
        // SAFETY: `win` is a live window (this export's contract; not NULL,
        // checked), so aligned and allocated for the call; `offset` and
        // `window_map.len` are written only by `new_window_locked`, before
        // the window is in any list or cursor, so these two place reads
        // race with nothing (the other fields, which other threads may
        // write under the mutex, are not touched, and no reference to the
        // whole window is made).
        let win_off = unsafe { (*win).offset };
        // SAFETY: as the line above, for `window_map.len`.
        let win_len = unsafe { (*win).window_map.len };
        contains_range(win_off, win_len, offset, extra) as c_int
    }

    /// `git_mwindow_free_all`.
    ///
    /// # Safety
    ///
    /// `mwf` is a live `git_mwindow_file`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_free_all(mwf: Option<&GitMwindowFile>) -> c_int {
        barrier(|| {
            let mwf = git_arg!(mwf, "mwf", -1);
            status(free_all(mwf))
        })
    }

    /// `git_mwindow_open`: a pointer to the byte at `offset` in a window
    /// of `mwf`, which `*cursor` then holds; `*left` (if not NULL) gets the
    /// bytes from there to the window's end. NULL on failure.
    ///
    /// # Safety
    ///
    /// `mwf` is a live `git_mwindow_file` that stays at its address while it
    /// has windows; `cursor` is NULL or a valid cursor (a `git_mwindow *`
    /// that is NULL or a window a previous call stored, as
    /// [`Cursor::from_raw_mut`] states), used by nothing else during the
    /// call; `left` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_open(
        mwf: Option<&GitMwindowFile>,
        cursor: Option<&mut Cursor>,
        offset: i64,
        extra: usize,
        mut left: Out<'_, c_uint>,
    ) -> *mut u8 {
        barrier(|| {
            let mwf = git_arg!(mwf, "mwf", core::ptr::null_mut());
            // (C dereferences a NULL cursor.)
            let cursor = git_arg!(cursor, "cursor", core::ptr::null_mut());
            match cursor.open_raw(mwf, offset, extra) {
                Some((p, l)) => {
                    if !left.is_null() {
                        left.write(l);
                    }
                    p.cast_mut()
                }
                None => ptr::null_mut(),
            }
        })
    }

    /// `git_mwindow_file_register`.
    ///
    /// # Safety
    ///
    /// `mwf` is a live `git_mwindow_file` that stays at its address until
    /// `git_mwindow_file_deregister` or `git_mwindow_free_all`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_file_register(mwf: Option<&GitMwindowFile>) -> c_int {
        barrier(|| {
            let mwf = git_arg!(mwf, "mwf", -1);
            // SAFETY: `mwf` stays at its address until deregistered (this
            // export's contract), as `file_register` requires.
            status(unsafe { file_register(mwf) })
        })
    }

    /// `git_mwindow_file_deregister`.
    ///
    /// # Safety
    ///
    /// `mwf` is a live `git_mwindow_file`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_file_deregister(mwf: Option<&GitMwindowFile>) {
        let mwf = git_arg!(mwf, "mwf", ());
        barrier(|| file_deregister(mwf))
    }

    /// `git_mwindow_close`.
    ///
    /// # Safety
    ///
    /// `window` is NULL or a valid cursor (as in [`git_mwindow_open`]).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_close(window: Option<&mut Cursor>) {
        barrier(|| git_arg!(window, "window", ()).close())
    }

    /// `git_mwindow_get_pack`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `path` is NULL or a NUL-terminated
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_get_pack(
        mut out: Out<'_, *mut GitPackFile>,
        path: CStrArg<'_>,
        oid_type: git_oid_t,
    ) -> c_int {
        barrier(|| {
            // (C dereferences a NULL `path`.)
            let path = git_arg!(path.get(), "path", -1);
            match get_pack(path, oid_type) {
                Ok(p) => {
                    out.write(p);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_mwindow_put_pack`.
    ///
    /// # Safety
    ///
    /// `pack` is a pack from `git_mwindow_get_pack` whose reference the
    /// caller gives up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mwindow_put_pack(pack: *mut GitPackFile) -> c_int {
        // SAFETY: a pack from `git_mwindow_get_pack` whose reference the
        // caller gives up (this export's contract), as `put_pack` requires.
        barrier(|| status(unsafe { put_pack(pack) }))
    }
}

crate::c_layout_test! {
    headers: ["mwindow.h"],
    types: [
        GitMwindowFile => "git_mwindow_file" { lock, windows, fd, size },
        GitMwindowCtl => "git_mwindow_ctl" {
            mapped, open_windows, mmap_calls, peak_open_windows, peak_mapped, used_ctr, windowfiles
        },
    ],
    asserts: ["sizeof(git_mwindow_packmap) == 40 && _Alignof(git_mwindow_packmap) == 8"],
}

const _: () = assert!(size_of::<PackMap>() == 40 && align_of::<PackMap>() == 8);

#[cfg(test)]
mod tests {
    use super::*;

    fn win(offset: i64, len: usize) -> GitMwindow {
        GitMwindow {
            next: ptr::null_mut(),
            window_map: GitMap {
                data: ptr::null_mut(),
                len,
            },
            offset,
            last_used: 0,
            inuse_cnt: 0,
        }
    }

    #[test]
    fn contains_is_cs() {
        let w = win(4096, 100);
        assert!(contains(&w, 4096, 0));
        assert!(contains(&w, 4096, 100));
        assert!(!contains(&w, 4096, 101));
        assert!(contains(&w, 4196, 0));
        assert!(!contains(&w, 4095, 1));
        // No overflow panic for huge extras.
        assert!(!contains(&w, i64::MAX, i64::MAX));
    }
}
