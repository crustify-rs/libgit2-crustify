//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

use core::ffi::{CStr, c_char, c_int, c_uint};

use crate::util::errors::Result;
use crate::util::str::GitStr;

// ---------------------------------------------------------------------------
// futils (src/util/futils.c)
// ---------------------------------------------------------------------------

/// `git_futils_filestamp` (`futils.h`), mirrored: what the stamp functions
/// compare. PENDING(u3): replace with futils' own type when it lands.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileStamp {
    /// `mtime` (`struct timespec`).
    pub mtime: libc::timespec,
    /// `size`.
    pub size: u64,
    /// `ino`.
    pub ino: c_uint,
}

impl Default for FileStamp {
    fn default() -> Self {
        FileStamp {
            mtime: libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            size: 0,
            ino: 0,
        }
    }
}

unsafe extern "C" {
    // PENDING(u3)
    fn git_futils_filestamp_check(stamp: *mut FileStamp, path: *const c_char) -> c_int;
    // PENDING(u3)
    fn git_futils_open_ro(path: *const c_char) -> c_int;
    // PENDING(u3)
    fn git_futils_readbuffer_fd(obj: *mut GitStr, fd: c_int, len: usize) -> c_int;
    // PENDING(u3): fs_path (assumed to be ported with futils).
    fn git_fs_path_find_executable(fullpath: *mut GitStr, executable: *const c_char) -> c_int;
}

/// `git_futils_filestamp_check`: 0 if `path` is unchanged since `stamp`,
/// 1 if changed (and `stamp` updated), `GIT_ENOTFOUND` if it does not
/// exist (no error message set). PENDING(u3).
pub fn futils_filestamp_check(stamp: &mut FileStamp, path: &CStr) -> c_int {
    // SAFETY: `stamp` is a valid, exclusive `git_futils_filestamp` for the
    // call (the C only reads and writes its fields) and `path` a
    // NUL-terminated string; the C keeps neither.
    unsafe { git_futils_filestamp_check(stamp, path.as_ptr()) }
}

/// `git_futils_open_ro`: a read-only descriptor, or an error code (with
/// the message set). PENDING(u3).
pub fn futils_open_ro(path: &CStr) -> Result<c_int> {
    // SAFETY: `path` is a NUL-terminated string the C only reads.
    crate::util::errors::ErrorCode::check(unsafe { git_futils_open_ro(path.as_ptr()) })
}

/// `git_futils_readbuffer_fd`: read `len` bytes of `fd` into `buf`.
/// PENDING(u3).
pub fn futils_readbuffer_fd(buf: &mut GitStr, fd: c_int, len: usize) -> Result<()> {
    // SAFETY: `buf` is a valid, exclusive `git_str` for the call, which the
    // C grows and fills through libgit2's allocator, as `GitStr` expects;
    // `fd` is only read.
    crate::util::errors::ErrorCode::check_zero(unsafe { git_futils_readbuffer_fd(buf, fd, len) })
}

/// `git_fs_path_find_executable`: the full path of `executable`, looked up
/// in `PATH` unless it contains a `/`. PENDING(u3).
pub fn fs_path_find_executable(out: &mut GitStr, executable: &CStr) -> c_int {
    // SAFETY: `out` is a valid, exclusive `git_str`, `executable` a
    // NUL-terminated string the C only reads.
    unsafe { git_fs_path_find_executable(out, executable.as_ptr()) }
}

crate::c_layout_test! {
    headers: ["futils.h"],
    types: [
        FileStamp => "git_futils_filestamp" { mtime, size, ino },
    ],
}
