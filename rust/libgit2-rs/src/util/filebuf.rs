//! Port of `src/util/filebuf.c`: `git_filebuf`, buffered writes to a lock
//! file (`<path>.lock`, or a temporary file) that is renamed over the target
//! on commit, with an optional running hash and deflate.
//!
//! # Types
//!
//! [`GitFilebuf`] is `git_filebuf`, mirrored (kind 3): C embeds it by value
//! (`GIT_FILEBUF_INIT`, `{0}`) in `git_indexer`, `refdb_fs` lock files,
//! the index writer, and the tests read `buffer`. Its embedded
//! `git_hash_ctx` (`digest`: [`HashCtx`], the hash port's) and `z_stream`
//! (`zs`: zlib's, driven through `sys::zlib`, the system zlib that
//! `filebuf.c` called) keep the C layout.
//!
//! Its invariant (which [`GitFilebuf::new`] and [`GitFilebuf::open`] keep):
//! `buffer` is NULL or a `git__allocator` allocation of `buf_size`
//! initialized bytes, `buf_pos <= buf_size`; `z_buf` is NULL or one of
//! `buf_size` bytes, and `zs` points to an initialized deflate stream in
//! its own heap cell exactly when `z_buf` is set (the cell may outlive a
//! failed init until cleanup); `digest` went through `HashCtx::init` when
//! `compute_digest`; the paths are NULL or NUL-terminated `git__allocator`
//! strings; `fd` is open when `fd_is_open`. The deflate stream lives apart
//! (see `StreamSlot`), so zlib's stored address never points into the
//! filebuf and no `&mut GitFilebuf` covers it.

use core::ffi::{CStr, c_char, c_int, c_void};
use std::os::fd::AsRawFd;

use super::alloc;
use super::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use super::fs_path::{self, c_str, str_c_bytes, str_cpath};
use super::futils::{self, GIT_BUFSIZE_FILEIO};
use super::integer::alloc_add;
use super::posix::{self, Errno, GIT_PATH_MAX, mode_t};
use super::str::GitStr;
use crate::git_error_set;
use crate::sys::zlib::{self, z_stream as ZStream};
use crate::util::hash::{self, HashCtx};

/// zlib's constants that `filebuf.c` uses.
mod z {
    use core::ffi::c_int;
    pub const Z_OK: c_int = 0;
    pub const Z_STREAM_ERROR: c_int = -2;
    pub const Z_NO_FLUSH: c_int = 0;
    pub const Z_FINISH: c_int = 4;
}

/// The `zs` field: C's `z_stream` storage, which C never reads (it only
/// embeds and sizes `git_filebuf`). The stream itself lives in a heap cell
/// that `stream` points to (NULL when there is none): zlib keeps the
/// stream's address from `deflateInit` on and writes through it, so no
/// Rust reference may cover the stream between zlib calls. A `&mut
/// GitFilebuf` covers only this slot; the cell is reached through the raw
/// pointer alone. (And the stream does not move with the filebuf.)
#[repr(C)]
struct StreamSlot {
    stream: *mut ZStream,
    _rest: [u8; core::mem::size_of::<ZStream>() - core::mem::size_of::<*mut ZStream>()],
}

const _: () = assert!(core::mem::size_of::<StreamSlot>() == core::mem::size_of::<ZStream>());
const _: () = assert!(core::mem::align_of::<StreamSlot>() == core::mem::align_of::<ZStream>());

impl StreamSlot {
    const fn empty() -> StreamSlot {
        StreamSlot {
            stream: core::ptr::null_mut(),
            _rest: [0; core::mem::size_of::<ZStream>() - core::mem::size_of::<*mut ZStream>()],
        }
    }

    /// A zeroed stream in a new heap cell (Rust's allocator: 112 bytes).
    fn alloc() -> *mut ZStream {
        Box::into_raw(Box::new(zeroed_stream()))
    }

    /// Free the cell (after `deflateEnd`, or when `deflateInit` failed).
    fn free(&mut self) {
        if !self.stream.is_null() {
            // SAFETY: the cell `alloc` made, freed once; zlib holds no state
            // in it any more (the callers' order).
            drop(unsafe { Box::from_raw(self.stream) });
            self.stream = core::ptr::null_mut();
        }
    }
}

/// A zeroed `z_stream` (C's `{0}`): no state, zlib's default allocator.
const fn zeroed_stream() -> ZStream {
    ZStream {
        next_in: core::ptr::null(),
        avail_in: 0,
        total_in: 0,
        next_out: core::ptr::null_mut(),
        avail_out: 0,
        total_out: 0,
        msg: core::ptr::null_mut(),
        state: core::ptr::null_mut(),
        zalloc: None,
        zfree: None,
        opaque: core::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    }
}

/// `GIT_FILEBUF_HASH_SHA1`.
pub const GIT_FILEBUF_HASH_SHA1: c_int = 1 << 0;
/// `GIT_FILEBUF_HASH_SHA256`.
pub const GIT_FILEBUF_HASH_SHA256: c_int = 1 << 1;
/// `GIT_FILEBUF_APPEND`.
pub const GIT_FILEBUF_APPEND: c_int = 1 << 2;
/// `GIT_FILEBUF_CREATE_LEADING_DIRS`.
pub const GIT_FILEBUF_CREATE_LEADING_DIRS: c_int = 1 << 3;
/// `GIT_FILEBUF_TEMPORARY`.
pub const GIT_FILEBUF_TEMPORARY: c_int = 1 << 4;
/// `GIT_FILEBUF_DO_NOT_BUFFER`.
pub const GIT_FILEBUF_DO_NOT_BUFFER: c_int = 1 << 5;
/// `GIT_FILEBUF_FSYNC`.
pub const GIT_FILEBUF_FSYNC: c_int = 1 << 6;
/// `GIT_FILEBUF_DEFLATE_SHIFT`.
pub const GIT_FILEBUF_DEFLATE_SHIFT: c_int = 7;
/// `GIT_FILELOCK_EXTENSION` (with its NUL).
pub const GIT_FILELOCK_EXTENSION: &[u8; 6] = b".lock\0";
/// `GIT_FILELOCK_EXTLENGTH`.
pub const GIT_FILELOCK_EXTLENGTH: usize = 6;

const WRITE_BUFFER_SIZE: usize = 4096 * 2;

const BUFERR_OK: c_int = 0;
const BUFERR_WRITE: c_int = 1;
const BUFERR_ZLIB: c_int = 2;
const BUFERR_MEM: c_int = 3;

const MAX_SYMLINK_DEPTH: usize = 5;

/// `git_filebuf`'s write function (`write_normal` or `write_deflate`).
pub type WriteFn = unsafe extern "C" fn(*mut GitFilebuf, *mut c_void, usize) -> c_int;

/// `git_filebuf`, mirrored (see the [module documentation](self)). The
/// fields are private: the safe methods rely on the invariant, which only
/// [`new`](Self::new) and the methods establish (C reaches the fields
/// through the exports' contracts).
#[repr(C)]
pub struct GitFilebuf {
    /// The target path (NULL for a temporary file).
    path_original: *mut c_char,
    /// The lock (or temporary) file's path.
    path_lock: *mut c_char,
    /// The write function: `write_normal` or `write_deflate`.
    write: Option<WriteFn>,
    /// Whether `digest` runs.
    compute_digest: bool,
    /// The running hash.
    digest: HashCtx,
    /// The write buffer.
    buffer: *mut u8,
    /// The deflate output buffer.
    z_buf: *mut u8,
    /// The deflate stream.
    zs: StreamSlot,
    /// `Z_NO_FLUSH`, or `Z_FINISH` on commit.
    flush_mode: c_int,
    /// The size of `buffer` (and `z_buf`).
    buf_size: usize,
    /// The bytes in `buffer`.
    buf_pos: usize,
    /// The lock file.
    fd: c_int,
    /// Whether `fd` is open.
    fd_is_open: bool,
    /// Whether the lock file was created (and is removed on cleanup).
    created_lock: bool,
    /// Whether the lock file was renamed over the target.
    did_rename: bool,
    /// Whether writes bypass the buffer.
    do_not_buffer: bool,
    /// Whether commit `fsync`s.
    do_fsync: bool,
    /// The first write error (`BUFERR_*`).
    last_error: c_int,
}

// SAFETY: field by field: `path_original`, `path_lock`, `buffer` and
// `z_buf` are NULL or `git__allocator` allocations that only this filebuf
// points to (the invariant), and `git__allocator` frees from any thread;
// `zs.stream` is NULL or a heap cell (Rust's allocator) that only this
// filebuf and zlib's own state (which points back at it, and is reached
// only through it) refer to, and zlib streams have no thread affinity;
// `write` is a function pointer, `digest` plain data (`HashCtx`), `fd` a
// descriptor number (usable from any thread), the rest integers and
// booleans. Moving the filebuf to another thread moves every user of what
// it points to.
unsafe impl Send for GitFilebuf {}

impl Default for GitFilebuf {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for GitFilebuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitFilebuf")
            .field("path_lock", &self.lock_path())
            .field("fd", &self.fd)
            .field("buf_pos", &self.buf_pos)
            .field("last_error", &self.last_error)
            .finish_non_exhaustive()
    }
}

/// The C string at `p` (NULL: none).
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that stays allocated and
/// unwritten for `'a` (a path field of a filebuf, which owns it, while the
/// field is neither freed nor changed).
unsafe fn opt_cstr<'a>(p: *const c_char) -> Option<&'a CStr> {
    if p.is_null() {
        None
    } else {
        // SAFETY: a non-null `p` is a string live and unwritten for `'a`
        // (this fn's contract).
        Some(unsafe { CStr::from_ptr(p) })
    }
}

/// Free `*p` and clear it (`git__free(p); p = NULL`).
///
/// # Safety
///
/// `*p` is NULL or a `git__allocator` allocation that nothing else frees
/// or uses after the call (one of a filebuf's own pointers).
unsafe fn free_owned<T>(p: &mut *mut T) {
    if !p.is_null() {
        // SAFETY: a non-null `*p` is a `git__allocator` allocation that
        // nothing uses after the call (this fn's contract); it is cleared
        // below, so freed once.
        unsafe { alloc::free(p.cast()) };
    }
    *p = core::ptr::null_mut();
}

impl GitFilebuf {
    /// `GIT_FILEBUF_INIT` (`{0}`).
    pub const fn new() -> Self {
        GitFilebuf {
            path_original: core::ptr::null_mut(),
            path_lock: core::ptr::null_mut(),
            write: None,
            compute_digest: false,
            digest: HashCtx::zeroed(),
            buffer: core::ptr::null_mut(),
            z_buf: core::ptr::null_mut(),
            zs: StreamSlot::empty(),
            flush_mode: 0,
            buf_size: 0,
            buf_pos: 0,
            fd: 0,
            fd_is_open: false,
            created_lock: false,
            did_rename: false,
            do_not_buffer: false,
            do_fsync: false,
            last_error: 0,
        }
    }

    /// The lock file's descriptor (-1 after cleanup; 0 in a zeroed
    /// filebuf, as C's `GIT_FILEBUF_INIT`).
    pub fn fd(&self) -> c_int {
        self.fd
    }

    /// Whether the write buffer is allocated (`file->buffer != NULL`).
    pub fn has_buffer(&self) -> bool {
        !self.buffer.is_null()
    }

    /// The target path.
    pub fn original_path(&self) -> Option<&CStr> {
        // SAFETY: `path_original` is NULL or a string this filebuf owns
        // (the invariant), which `&self` keeps from being freed or changed
        // for the result's lifetime.
        unsafe { opt_cstr(self.path_original) }
    }

    /// The lock (or temporary) file's path.
    pub fn lock_path(&self) -> Option<&CStr> {
        // SAFETY: as in `original_path`, for `path_lock`.
        unsafe { opt_cstr(self.path_lock) }
    }

    fn verify_last_error(&self) -> Result<()> {
        match self.last_error {
            BUFERR_WRITE => Err(git_error_set!(ErrorClass::OS, "failed to write out file")),
            BUFERR_MEM => Err(errors::oom()),
            BUFERR_ZLIB => Err(git_error_set!(
                ErrorClass::ZLIB,
                "Buffer error when writing out ZLib data"
            )),
            _ => Ok(()),
        }
    }

    fn hash_update(&mut self, data: &[u8]) {
        if self.compute_digest {
            // SAFETY: `digest` went through `init` while `compute_digest`
            // (the invariant).
            if let Ok(h) = unsafe { self.digest.hasher() } {
                // C ignores `git_hash_update`'s result here.
                let _ = h.update(data);
            }
        }
    }

    fn lock_file(&mut self, flags: c_int, mode: mode_t) -> Result<()> {
        // The paths are borrowed through their own pointers, apart from
        // `&mut self`.
        // SAFETY: `path_lock` is NULL or a string this filebuf owns (the
        // invariant); this fn changes only `fd`, `fd_is_open` and `digest`,
        // so the string stays allocated and unwritten while `lock` is used.
        let lock = unsafe { opt_cstr(self.path_lock) }.unwrap_or(c"");
        if fs_path::exists(lock) {
            errors::clear();
            let _ = git_error_set!(
                ErrorClass::OS,
                "failed to lock file '{}' for writing",
                Bytes(lock.to_bytes())
            );
            return Err(ErrorCode::LOCKED);
        }
        let fd = if flags & GIT_FILEBUF_CREATE_LEADING_DIRS != 0 {
            futils::creat_locked_withpath(lock, 0o777, mode)
        } else {
            futils::creat_locked(lock, mode)
        };
        self.fd = match fd {
            Ok(fd) => fd,
            Err(e) => {
                self.fd = e.raw();
                return Err(e);
            }
        };
        self.fd_is_open = true;
        // SAFETY: as for `lock`, with `path_original`.
        let original = unsafe { opt_cstr(self.path_original) }.unwrap_or(c"");
        if flags & GIT_FILEBUF_APPEND != 0 && fs_path::exists(original) {
            let Ok(source) = posix::open_owned(original, libc::O_RDONLY, 0) else {
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "failed to open file '{}' for reading",
                    Bytes(original.to_bytes())
                ));
            };
            let mut buffer = Vec::new();
            if buffer.try_reserve_exact(GIT_BUFSIZE_FILEIO).is_err() {
                drop(source);
                return Err(errors::oom());
            }
            buffer.resize(GIT_BUFSIZE_FILEIO, 0);
            let mut write_failed = false;
            let read = loop {
                match posix::read(source.as_raw_fd(), &mut buffer) {
                    Ok(0) => break Ok(()),
                    Ok(n) => {
                        if posix::write(self.fd, &buffer[..n]).is_err() {
                            write_failed = true;
                            break Ok(());
                        }
                        self.hash_update(&buffer[..n]);
                    }
                    Err(e) => break Err(e),
                }
            };
            drop(source);
            if read.is_err() {
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "failed to read file '{}'",
                    Bytes(original.to_bytes())
                ));
            } else if write_failed {
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "failed to write file '{}'",
                    Bytes(lock.to_bytes())
                ));
            }
        }
        Ok(())
    }

    /// `git_filebuf_cleanup`: close and remove the lock file (unless it
    /// was committed), free everything, and leave the filebuf zeroed with
    /// `fd` -1.
    pub fn cleanup(&mut self) {
        if self.fd_is_open && self.fd >= 0 {
            // SAFETY: a descriptor this code opened, closed once.
            let _ = unsafe { posix::close(self.fd) };
        }
        if self.created_lock
            && !self.did_rename
            && let Some(lock) = self.lock_path()
            && fs_path::exists(lock)
        {
            let _ = posix::unlink(lock);
        }
        // (`git_hash_ctx_cleanup` has nothing to release.)
        self.compute_digest = false;
        // SAFETY: `buffer` is NULL or this filebuf's own allocation (the
        // invariant), not used after: the filebuf is reset below.
        unsafe { free_owned(&mut self.buffer) };
        if !self.z_buf.is_null() {
            // SAFETY: as for `buffer`.
            unsafe { free_owned(&mut self.z_buf) };
            // SAFETY: the stream is initialized while `z_buf` is set (the
            // invariant), and ended once; reached only through its pointer.
            unsafe { zlib::deflate_end(self.zs.stream) };
        }
        self.zs.free();
        // SAFETY: as for `buffer`.
        unsafe { free_owned(&mut self.path_original) };
        // SAFETY: as for `buffer`.
        unsafe { free_owned(&mut self.path_lock) };
        *self = GitFilebuf::new();
        self.fd = -1;
    }

    /// `flush_buffer`: hand the buffer to the write function.
    fn flush_buffer(&mut self) -> c_int {
        let pos = self.buf_pos;
        let r = if self.buffer.is_null() {
            self.dispatch_write(&[])
        } else {
            // SAFETY: `buffer` holds `buf_size >= buf_pos` initialized bytes
            // (the invariant), a separate allocation that the write
            // functions do not touch.
            let data = unsafe { core::slice::from_raw_parts(self.buffer, pos) };
            self.dispatch_write(data)
        };
        self.buf_pos = 0;
        r
    }

    /// `file->write(file, data, len)`.
    fn dispatch_write(&mut self, data: &[u8]) -> c_int {
        if self.write.is_none() {
            // C calls a NULL function: a filebuf that was never opened.
            return errors::assert_failed("file->write").raw();
        }
        let r = if self.z_buf.is_null() {
            self.write_normal(data)
        } else {
            self.write_deflate(data)
        };
        match r {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    }

    fn write_normal(&mut self, source: &[u8]) -> Result<()> {
        if !source.is_empty() {
            if posix::write(self.fd, source).is_err() {
                self.last_error = BUFERR_WRITE;
                return Err(ErrorCode::GENERIC);
            }
            self.hash_update(source);
        }
        Ok(())
    }

    fn write_deflate(&mut self, source: &[u8]) -> Result<()> {
        if !source.is_empty() || self.flush_mode == z::Z_FINISH {
            // The stream's cell is reached only through this pointer (see
            // `StreamSlot`): its fields are read and written in place.
            let zs = self.zs.stream;
            // SAFETY: the cell is live while `z_buf` is set (the invariant),
            // and nothing else accesses it during this call.
            unsafe { (*zs).next_in = source.as_ptr() };
            // SAFETY: as above.
            unsafe { (*zs).avail_in = source.len() as u32 };
            loop {
                // SAFETY: as above.
                unsafe { (*zs).next_out = self.z_buf };
                // SAFETY: as above.
                unsafe { (*zs).avail_out = self.buf_size as u32 };
                // SAFETY: the stream is initialized (`z_buf` is set) and
                // has not moved (a heap cell); `next_in` has `avail_in`
                // readable bytes, `next_out` `avail_out` writable ones.
                if unsafe { zlib::deflate(zs, self.flush_mode) } == z::Z_STREAM_ERROR {
                    self.last_error = BUFERR_ZLIB;
                    return Err(ErrorCode::GENERIC);
                }
                // SAFETY: as above, between zlib calls.
                let avail_out = unsafe { (*zs).avail_out };
                let have = self.buf_size - avail_out as usize;
                // SAFETY: `deflate` wrote `have` bytes at `z_buf`, an
                // allocation of `buf_size` bytes.
                let out = unsafe { core::slice::from_raw_parts(self.z_buf, have) };
                if posix::write(self.fd, out).is_err() {
                    self.last_error = BUFERR_WRITE;
                    return Err(ErrorCode::GENERIC);
                }
                if avail_out != 0 {
                    break;
                }
            }
            // SAFETY: as above.
            unsafe { (*zs).next_in = core::ptr::null() };
            // SAFETY: as above.
            let avail_in = unsafe { (*zs).avail_in };
            crate::git_assert!(avail_in == 0, "zs->avail_in == 0");
            self.hash_update(source);
        }
        Ok(())
    }

    /// `git_filebuf_open`.
    pub fn open(&mut self, path: &[u8], flags: c_int, mode: mode_t) -> Result<()> {
        self.open_withsize(path, flags, mode, WRITE_BUFFER_SIZE)
    }

    /// `git_filebuf_open_withsize`: start writing `path` (through
    /// `path.lock`, or a temporary file `path_git2_*` with
    /// [`GIT_FILEBUF_TEMPORARY`]) with a buffer of `size` bytes.
    ///
    /// The filebuf may move while open: the deflate stream, whose address
    /// zlib keeps, lives in its own heap cell (`StreamSlot`), and
    /// everything else is plain data or pointers to separate allocations
    /// (the `HashCtx` included). (This was an `unsafe fn` whose contract
    /// was "does not move until cleaned up", for the stream that used to
    /// live in place.)
    pub fn open_withsize(
        &mut self,
        path: &[u8],
        flags: c_int,
        mode: mode_t,
        size: usize,
    ) -> Result<()> {
        crate::git_assert!(self.buffer.is_null(), "file->buffer == ((void*)0)");
        *self = GitFilebuf::new();
        let r = self.open_inner(path, flags, mode, size);
        if r.is_err() {
            self.cleanup();
        }
        r
    }

    fn open_inner(&mut self, path: &[u8], flags: c_int, mode: mode_t, size: usize) -> Result<()> {
        let path = c_str(path);
        self.do_not_buffer = flags & GIT_FILEBUF_DO_NOT_BUFFER != 0;
        self.do_fsync = flags & GIT_FILEBUF_FSYNC != 0;
        self.buf_size = size;
        self.buf_pos = 0;
        self.fd = -1;
        self.last_error = BUFERR_OK;
        if !self.do_not_buffer {
            // Zeroed (C: uninitialized), so that the buffer's bytes are
            // always initialized.
            self.buffer = alloc::calloc(self.buf_size, 1).cast();
            if self.buffer.is_null() {
                return Err(ErrorCode::GENERIC);
            }
        }
        let algorithm = if flags & GIT_FILEBUF_HASH_SHA1 != 0 {
            Some(hash::HashAlgorithm::Sha1)
        } else if flags & GIT_FILEBUF_HASH_SHA256 != 0 {
            Some(hash::HashAlgorithm::Sha256)
        } else {
            None
        };
        if let Some(a) = algorithm {
            self.compute_digest = true;
            if self.digest.init(a).is_err() {
                return Err(ErrorCode::GENERIC);
            }
        }
        let compression = flags >> GIT_FILEBUF_DEFLATE_SHIFT;
        if compression != 0 {
            self.zs.stream = StreamSlot::alloc();
            // SAFETY: a zeroed stream in its own heap cell, which does not
            // move and is reached only through this pointer.
            if unsafe { zlib::deflate_init(self.zs.stream, compression) } != z::Z_OK {
                // No zlib state: cleanup frees the cell.
                return Err(git_error_set!(
                    ErrorClass::ZLIB,
                    "failed to initialize zlib"
                ));
            }
            self.z_buf = alloc::malloc(self.buf_size).cast();
            if self.z_buf.is_null() {
                // C returns here without ending the stream (a leak):
                // end it, as cleanup will not (rust/DEFECTS.md).
                // SAFETY: initialized just above, ended once; cleanup then
                // frees the cell (`z_buf` is NULL, so it does not end it).
                unsafe { zlib::deflate_end(self.zs.stream) };
                return Err(ErrorCode::GENERIC);
            }
            self.flush_mode = z::Z_NO_FLUSH;
            self.write = Some(ffi::write_deflate);
        } else {
            self.write = Some(ffi::write_normal);
        }
        if flags & GIT_FILEBUF_TEMPORARY != 0 {
            let mut tmp_path = GitStr::new();
            match futils::mktmp(&mut tmp_path, path, mode) {
                Ok(fd) => self.fd = fd,
                Err(_) => {
                    self.fd = -1;
                    return Err(ErrorCode::GENERIC);
                }
            }
            self.fd_is_open = true;
            self.created_lock = true;
            self.path_original = core::ptr::null_mut();
            self.path_lock = match tmp_path.detach() {
                Some(p) => p.into_raw(),
                None => return Err(errors::oom()),
            };
        } else {
            let mut resolved = GitStr::new();
            resolve_symlink(&mut resolved, path)?;
            let path_len = resolved.len();
            self.path_original = resolved
                .detach()
                .map_or(core::ptr::null_mut(), |p| p.into_raw());
            let alloc_len = alloc_add(path_len, GIT_FILELOCK_EXTLENGTH)?;
            let lock = alloc::malloc(alloc_len).cast::<u8>();
            if lock.is_null() {
                return Err(ErrorCode::GENERIC);
            }
            if path_len > 0 {
                // SAFETY: `path_original` holds `path_len` bytes (and its
                // NUL), `lock` `path_len + 6`; distinct allocations.
                unsafe {
                    core::ptr::copy_nonoverlapping(self.path_original.cast::<u8>(), lock, path_len)
                };
            }
            // SAFETY: the last 6 bytes of `lock`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    GIT_FILELOCK_EXTENSION.as_ptr(),
                    lock.wrapping_add(path_len),
                    GIT_FILELOCK_EXTLENGTH,
                )
            };
            self.path_lock = lock.cast();
            if let Some(orig) = self.original_path()
                && fs_path::isdir(orig)
            {
                let _ = git_error_set!(
                    ErrorClass::FILESYSTEM,
                    "path '{}' is a directory",
                    Bytes(orig.to_bytes())
                );
                return Err(ErrorCode::DIRECTORY);
            }
            self.lock_file(flags, mode)?;
            self.created_lock = true;
        }
        Ok(())
    }

    /// `git_filebuf_hash`: the hash of what was written (flushing first);
    /// `out` receives the algorithm's digest (20 or 32 bytes).
    pub fn hash(&mut self, out: &mut [u8; hash::MAX_SIZE]) -> Result<()> {
        // SAFETY: `out` has room for the largest digest.
        unsafe { self.hash_into(out.as_mut_ptr()) }
    }

    /// [`hash`](Self::hash) into raw memory.
    ///
    /// # Safety
    ///
    /// `out` is writable for the digest size of the filebuf's algorithm.
    pub(crate) unsafe fn hash_into(&mut self, out: *mut u8) -> Result<()> {
        crate::git_assert_arg!(self.compute_digest, "file->compute_digest");
        let _ = self.flush_buffer();
        self.verify_last_error()?;
        // C ignores `git_hash_final`'s result (a SHA-1 collision is
        // reported only through `git_hash_final`'s return value).
        // SAFETY: `digest` went through `init` (`compute_digest`).
        if let Ok(h) = unsafe { self.digest.hasher() } {
            let mut d = [0u8; hash::MAX_SIZE];
            let _ = h.finalize_raw(&mut d);
            let n = h.algorithm().size();
            // SAFETY: `out` has room for this algorithm's digest (the
            // caller); `d` is a distinct local.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, n) };
        }
        self.compute_digest = false;
        Ok(())
    }

    /// `git_filebuf_commit_at`: commit to `path` instead.
    pub fn commit_at(&mut self, path: &CStr) -> Result<()> {
        // SAFETY: `path_original` is NULL or this filebuf's own string (the
        // invariant), replaced right below; `path` is the caller's, so not
        // that string (a `&CStr` of the filebuf's own path borrows `self`).
        unsafe { free_owned(&mut self.path_original) };
        self.path_original = alloc::strdup(path);
        if self.path_original.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        self.commit()
    }

    /// `git_filebuf_commit`: flush, close and rename the lock file over the
    /// target; the filebuf is cleaned up either way.
    pub fn commit(&mut self) -> Result<()> {
        crate::git_assert!(!self.path_original.is_null(), "file->path_original");
        let r = self.commit_inner();
        self.cleanup();
        r
    }

    fn commit_inner(&mut self) -> Result<()> {
        self.flush_mode = z::Z_FINISH;
        let _ = self.flush_buffer();
        self.verify_last_error()?;
        self.fd_is_open = false;
        // SAFETY: as in `lock_file`: `path_lock` is NULL or this filebuf's
        // own string, and this fn changes neither path (only `fd`,
        // `fd_is_open`, `did_rename`, and the buffer through the flush
        // above), so it stays allocated and unwritten while used.
        let lock = unsafe { opt_cstr(self.path_lock) }.unwrap_or(c"");
        if self.do_fsync && posix::fsync(self.fd).is_err() {
            let _ = git_error_set!(
                ErrorClass::OS,
                "failed to fsync '{}'",
                Bytes(lock.to_bytes())
            );
            // C leaves the descriptor open (a leak): close it.
            // SAFETY: a descriptor this code opened, closed once.
            let _ = unsafe { posix::close(self.fd) };
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: a descriptor this code opened, closed once.
        if unsafe { posix::close(self.fd) }.is_err() {
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to close file at '{}'",
                Bytes(lock.to_bytes())
            ));
        }
        self.fd = -1;
        // SAFETY: as for `lock`, with `path_original`.
        let original = unsafe { opt_cstr(self.path_original) }.unwrap_or(c"");
        if posix::rename(lock, original).is_err() {
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to rename lockfile to '{}'",
                Bytes(original.to_bytes())
            ));
        }
        if self.do_fsync {
            futils::fsync_parent(original.to_bytes())?;
        }
        self.did_rename = true;
        Ok(())
    }

    /// `git_filebuf_write`.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.last_error != BUFERR_OK {
            return Err(ErrorCode::GENERIC);
        }
        if self.do_not_buffer {
            return ErrorCode::check_zero(self.dispatch_write(data));
        }
        let mut buf = data;
        loop {
            let space_left = self.buf_size - self.buf_pos;
            if space_left > buf.len() {
                self.add_to_cache(buf);
                return Ok(());
            }
            self.add_to_cache(&buf[..space_left]);
            if self.flush_buffer() < 0 {
                return Err(ErrorCode::GENERIC);
            }
            buf = &buf[space_left..];
        }
    }

    fn add_to_cache(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        assert!(!self.buffer.is_null() && data.len() <= self.buf_size - self.buf_pos);
        // SAFETY: `buffer` has `buf_size` bytes (the invariant), of which
        // `data.len()` fit at `buf_pos` (checked); `data` lies elsewhere.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.buffer.wrapping_add(self.buf_pos),
                data.len(),
            )
        };
        self.buf_pos += data.len();
    }

    /// `git_filebuf_reserve`: `len` bytes of the buffer to fill, at the
    /// returned offset (for the export, which hands out `buffer + offset`).
    fn reserve_offset(&mut self, len: usize) -> Result<usize> {
        let space_left = self.buf_size - self.buf_pos;
        if self.last_error != BUFERR_OK {
            return Err(ErrorCode::GENERIC);
        }
        if len > self.buf_size {
            self.last_error = BUFERR_MEM;
            return Err(ErrorCode::GENERIC);
        }
        if space_left <= len && self.flush_buffer() < 0 {
            return Err(ErrorCode::GENERIC);
        }
        let at = self.buf_pos;
        self.buf_pos += len;
        Ok(at)
    }

    /// `git_filebuf_reserve`: `len` bytes of the buffer, to fill (they are
    /// written out as they are at the next flush).
    pub fn reserve(&mut self, len: usize) -> Result<&mut [u8]> {
        let at = self.reserve_offset(len)?;
        if self.buffer.is_null() {
            return Ok(&mut []);
        }
        // SAFETY: `buffer` has `buf_size` initialized bytes (the
        // invariant); `at + len <= buf_size`, borrowed with `self`.
        Ok(unsafe { core::slice::from_raw_parts_mut(self.buffer.wrapping_add(at), len) })
    }

    /// `git_filebuf_printf` with the formatted bytes (`None`: C's
    /// formatting failed).
    pub fn printf_formatted(&mut self, data: Option<&[u8]>) -> Result<()> {
        if self.last_error != BUFERR_OK {
            return Err(ErrorCode::GENERIC);
        }
        let Some(data) = data else {
            self.last_error = BUFERR_MEM;
            return Err(ErrorCode::GENERIC);
        };
        let len = data.len();
        if !self.buffer.is_null() && len < self.buf_size - self.buf_pos {
            self.add_to_cache(data);
            return Ok(());
        }
        if self.flush_buffer() < 0 {
            return Err(ErrorCode::GENERIC);
        }
        if !self.buffer.is_null() && len < self.buf_size - self.buf_pos {
            self.add_to_cache(data);
            return Ok(());
        }
        if len.checked_add(1).is_none() {
            self.last_error = BUFERR_MEM;
            return Err(ErrorCode::GENERIC);
        }
        self.write(data)
    }

    /// `git_filebuf_flush`.
    pub fn flush(&mut self) -> c_int {
        self.flush_buffer()
    }

    /// `git_filebuf_stats`: the file's modification time and size.
    pub fn stats(&self) -> Result<(libc::time_t, usize)> {
        let st = if self.fd_is_open {
            posix::fstat(self.fd)
        } else if let Some(p) = self.original_path() {
            posix::stat(p)
        } else {
            // C passes the NULL path of a temporary file to `stat`.
            errors::set_errno(libc::EFAULT);
            Err(Errno(libc::EFAULT))
        };
        match st {
            Ok(st) => Ok((st.st_mtime, st.st_size as usize)),
            Err(_) => Err(git_error_set!(
                ErrorClass::OS,
                "could not get stat info for '{}'",
                Bytes(self.original_path().map_or(&b"(null)"[..], CStr::to_bytes))
            )),
        }
    }
}

/// `resolve_symlink`: follow up to five symlinks from `path`.
fn resolve_symlink(out: &mut GitStr, path: &[u8]) -> Result<()> {
    let mut curpath = GitStr::new();
    let mut target = [0u8; GIT_PATH_MAX + 1];
    curpath.puts(path)?;
    for _ in 0..MAX_SYMLINK_DEPTH {
        let cp = str_cpath(&curpath);
        let st = match posix::lstat(&cp) {
            Ok(st) => st,
            Err(Errno(libc::ENOENT)) => return out.puts(cp.to_bytes()),
            Err(_) => {
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "failed to stat '{}'",
                    Bytes(cp.to_bytes())
                ));
            }
        };
        if !posix::s_islnk(st.st_mode) {
            return out.puts(cp.to_bytes());
        }
        let ret = posix::readlink(&cp, &mut target[..GIT_PATH_MAX]).map_err(|_| {
            git_error_set!(
                ErrorClass::OS,
                "failed to read symlink '{}'",
                Bytes(cp.to_bytes())
            )
        })?;
        if ret == GIT_PATH_MAX {
            return Err(git_error_set!(
                ErrorClass::INVALID,
                "symlink target too long"
            ));
        }
        let t = c_str(&target[..ret]);
        if fs_path::root(t) >= 0 {
            curpath.sets(t)?;
        } else {
            let mut dir = GitStr::new();
            fs_path::dirname_r(Some(&mut dir), Some(str_c_bytes(&curpath)))?;
            curpath.swap(&mut dir);
            // C tests `apply_relative(…) < 0`, which its 0-or-1 result
            // never is: a failure is ignored.
            let _ = fs_path::apply_relative(&mut curpath, t);
        }
    }
    Err(git_error_set!(
        ErrorClass::INVALID,
        "maximum symlink depth reached"
    ))
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A lock file being written (`git_filebuf`), for Rust callers: dropped
/// without [`commit`](Self::commit), it removes the lock file.
#[derive(Debug)]
pub struct Filebuf(Box<GitFilebuf>);

impl Filebuf {
    /// Start writing `path` with `flags` (`GIT_FILEBUF_*`) and file `mode`.
    pub fn open(
        path: &[u8],
        flags: c_int,
        mode: mode_t,
    ) -> core::result::Result<Filebuf, errors::Error> {
        let mut f = Box::new(GitFilebuf::new());
        f.open(path, flags, mode).map_err(errors::Error::from)?;
        Ok(Filebuf(f))
    }

    /// Append `data`.
    pub fn write(&mut self, data: &[u8]) -> core::result::Result<(), errors::Error> {
        self.0.write(data).map_err(errors::Error::from)
    }

    /// Reserve `len` bytes to fill in place.
    pub fn reserve(&mut self, len: usize) -> core::result::Result<&mut [u8], errors::Error> {
        self.0.reserve(len).map_err(errors::Error::from)
    }

    /// Append formatted text (`git_filebuf_printf`).
    pub fn write_fmt(
        &mut self,
        args: core::fmt::Arguments<'_>,
    ) -> core::result::Result<(), errors::Error> {
        let s = std::fmt::format(args);
        self.0
            .printf_formatted(Some(s.as_bytes()))
            .map_err(errors::Error::from)
    }

    /// The lock file's path.
    pub fn lock_path(&self) -> Option<&[u8]> {
        self.0.lock_path().map(CStr::to_bytes)
    }

    /// The hash of what was written so far (the filebuf must have been
    /// opened with a hash flag); the digest's bytes.
    pub fn hash(&mut self) -> core::result::Result<[u8; hash::MAX_SIZE], errors::Error> {
        let mut out = [0u8; hash::MAX_SIZE];
        self.0.hash(&mut out).map_err(errors::Error::from)?;
        Ok(out)
    }

    /// Rename the lock file over the target.
    pub fn commit(mut self) -> core::result::Result<(), errors::Error> {
        self.0.commit().map_err(errors::Error::from)
    }

    /// Rename the lock file over `path`.
    pub fn commit_at(mut self, path: &[u8]) -> core::result::Result<(), errors::Error> {
        let p = fs_path::to_cstring(path);
        self.0.commit_at(&p).map_err(errors::Error::from)
    }
}

impl Drop for Filebuf {
    fn drop(&mut self) {
        self.0.cleanup();
    }
}

/// The C ABI of `filebuf.c`.
pub mod ffi;

crate::c_layout_test! {
    headers: ["filebuf.h"],
    types: [
        GitFilebuf => "git_filebuf" {
            path_original, path_lock, write, compute_digest, digest, buffer, z_buf, zs,
            flush_mode, buf_size, buf_pos, fd, fd_is_open, created_lock, did_rename,
            do_not_buffer, do_fsync, last_error
        },
        ZStream => "z_stream" {
            next_in, avail_in, total_in, next_out, avail_out, total_out, msg, state,
            zalloc, zfree, opaque, data_type, adler, reserved
        },
    ],
    asserts: [
        "GIT_FILEBUF_HASH_SHA1 == 1", "GIT_FILEBUF_HASH_SHA256 == 2", "GIT_FILEBUF_APPEND == 4",
        "GIT_FILEBUF_CREATE_LEADING_DIRS == 8", "GIT_FILEBUF_TEMPORARY == 16",
        "GIT_FILEBUF_DO_NOT_BUFFER == 32", "GIT_FILEBUF_FSYNC == 64",
        "GIT_FILEBUF_DEFLATE_SHIFT == 7", "GIT_FILELOCK_EXTLENGTH == 6",
        "Z_OK == 0", "Z_STREAM_ERROR == -2", "Z_NO_FLUSH == 0", "Z_FINISH == 4",
        "GIT_HASH_ALGORITHM_SHA1 == 1", "GIT_HASH_ALGORITHM_SHA256 == 2",
        "GIT_HASH_MAX_SIZE == 32",
    ],
}

#[cfg(test)]
mod tests {
    //! What runs without a filesystem (and so under Miri): a filebuf that
    //! was never opened, and the formatting placement's guards. The rest is
    //! in `tests/util_fs.rs` and `rust/difftest/fs_diff.c`.
    use super::*;
    use crate::util::test_support;

    #[test]
    fn unopened_filebuf_fails_cleanly() {
        let _g = test_support::init();
        let mut f = GitFilebuf::new();
        assert_eq!(f.fd(), 0);
        assert!(!f.has_buffer());
        // C calls the NULL write function (rust/DEFECTS.md).
        assert_eq!(f.write(b"x"), Err(ErrorCode::GENERIC));
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: 'file->write'"
        );
        assert!(f.flush() < 0);
        assert!(f.printf_formatted(Some(b"abc")).is_err());
        // A reservation larger than the (empty) buffer poisons it.
        assert!(f.reserve(1).is_err());
        assert_eq!(f.write(b"x"), Err(ErrorCode::GENERIC));
        let mut out = [0u8; hash::MAX_SIZE];
        assert!(f.hash(&mut out).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid argument: 'file->compute_digest'"
        );
        assert_eq!(f.commit(), Err(ErrorCode::GENERIC));
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: 'file->path_original'"
        );
        f.cleanup();
        f.cleanup();
        assert_eq!(f.fd(), -1);
        assert!(f.lock_path().is_none() && f.original_path().is_none());
    }

    #[test]
    fn formatting_failure_is_out_of_memory() {
        let _g = test_support::init();
        let mut f = GitFilebuf::new();
        assert!(f.printf_formatted(None).is_err());
        assert_eq!(f.last_error, BUFERR_MEM);
        // Later writes fail without a new message.
        errors::clear();
        assert!(f.write(b"x").is_err());
        assert!(!errors::exists());
        assert!(f.verify_last_error().is_err());
        assert_eq!(errors::last_message().unwrap(), b"Out of memory");
    }

    #[test]
    fn stream_cell_lives_apart() {
        // The cell the deflate stream lives in: allocated zeroed (no zlib
        // state, default allocator), reached through its pointer only, freed
        // once. (`deflate` itself is C: not under Miri.)
        let mut slot = StreamSlot::empty();
        slot.stream = StreamSlot::alloc();
        // SAFETY: the fresh cell, read through its pointer.
        let state = unsafe { (*slot.stream).state };
        assert!(state.is_null());
        // SAFETY: as above.
        unsafe { (*slot.stream).avail_in = 3 };
        // A `&mut` to the slot does not cover the cell: moving the slot
        // leaves the stream where zlib would have its address.
        let p = slot.stream;
        let mut moved = slot;
        assert_eq!(moved.stream, p);
        // SAFETY: as above.
        assert_eq!(unsafe { (*p).avail_in }, 3);
        moved.free();
        moved.free();
        assert!(moved.stream.is_null());
    }

    #[test]
    fn zeroed_state_is_c_init() {
        let f = GitFilebuf::new();
        assert!(f.zs.stream.is_null());
        assert!(f.write.is_none() && !f.compute_digest && f.z_buf.is_null());
        assert_eq!((f.buf_size, f.buf_pos, f.last_error), (0, 0, BUFERR_OK));
    }
}
