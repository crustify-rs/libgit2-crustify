//! Port of `src/util/futils.c`: file utilities (reading and writing whole
//! files, `mkdir -p`, `rm -r`, copying trees, temporary and lock files,
//! file stamps, `fsync`).
//!
//! # Types
//!
//! - [`Filestamp`] is `git_futils_filestamp`, mirrored (kind 3: embedded in
//!   `git_index`, `attr_file`, `config_file`, `refdb_fs`).
//! - [`MkdirOptions`] / [`MkdirPerfdata`] are `struct
//!   git_futils_mkdir_options` / `_perfdata`, mirrored (C makes them on its
//!   stack: `checkout.c`). The Rust's own functions take [`MkdirOpts`],
//!   whose path cache is a trait object ([`MkdirCache`]); the export adapts
//!   C's `git_pool` + `git_hashset_str` to it.

use core::ffi::{CStr, c_int, c_uint, c_void};

use super::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use super::fs_path::{self, WalkStep, c_str, str_c_bytes, str_cpath, to_cstring};
use super::integer::alloc_add;
use super::posix::{self, Errno, GitMap, Stat, mode_t};
use super::str::GitStr;
use crate::git_error_set;
use crate::util::hash;

/// `GIT_FILEMODE_DEFAULT` (of `futils.c`).
const GIT_FILEMODE_DEFAULT: mode_t = 0o100666;
/// `GIT_BUFSIZE_FILEIO`.
pub const GIT_BUFSIZE_FILEIO: usize = 65536;
/// `O_FSYNC` (`O_SYNC` on Linux).
pub const O_FSYNC: c_int = libc::O_SYNC;

/// `GIT_MKDIR_EXCL`.
pub const GIT_MKDIR_EXCL: u32 = 1;
/// `GIT_MKDIR_PATH`.
pub const GIT_MKDIR_PATH: u32 = 2;
/// `GIT_MKDIR_CHMOD`.
pub const GIT_MKDIR_CHMOD: u32 = 4;
/// `GIT_MKDIR_CHMOD_PATH`.
pub const GIT_MKDIR_CHMOD_PATH: u32 = 8;
/// `GIT_MKDIR_SKIP_LAST`.
pub const GIT_MKDIR_SKIP_LAST: u32 = 16;
/// `GIT_MKDIR_SKIP_LAST2`.
pub const GIT_MKDIR_SKIP_LAST2: u32 = 32;
/// `GIT_MKDIR_VERIFY_DIR`.
pub const GIT_MKDIR_VERIFY_DIR: u32 = 64;
/// `GIT_MKDIR_REMOVE_FILES`.
pub const GIT_MKDIR_REMOVE_FILES: u32 = 128;
/// `GIT_MKDIR_REMOVE_SYMLINKS`.
pub const GIT_MKDIR_REMOVE_SYMLINKS: u32 = 256;

/// `GIT_RMDIR_EMPTY_HIERARCHY`.
pub const GIT_RMDIR_EMPTY_HIERARCHY: u32 = 0;
/// `GIT_RMDIR_REMOVE_FILES`.
pub const GIT_RMDIR_REMOVE_FILES: u32 = 1 << 0;
/// `GIT_RMDIR_SKIP_NONEMPTY`.
pub const GIT_RMDIR_SKIP_NONEMPTY: u32 = 1 << 1;
/// `GIT_RMDIR_EMPTY_PARENTS`.
pub const GIT_RMDIR_EMPTY_PARENTS: u32 = 1 << 2;
/// `GIT_RMDIR_REMOVE_BLOCKERS`.
pub const GIT_RMDIR_REMOVE_BLOCKERS: u32 = 1 << 3;
/// `GIT_RMDIR_SKIP_ROOT`.
pub const GIT_RMDIR_SKIP_ROOT: u32 = 1 << 4;

/// `GIT_CPDIR_CREATE_EMPTY_DIRS`.
pub const GIT_CPDIR_CREATE_EMPTY_DIRS: u32 = 1 << 0;
/// `GIT_CPDIR_COPY_SYMLINKS`.
pub const GIT_CPDIR_COPY_SYMLINKS: u32 = 1 << 1;
/// `GIT_CPDIR_COPY_DOTFILES`.
pub const GIT_CPDIR_COPY_DOTFILES: u32 = 1 << 2;
/// `GIT_CPDIR_OVERWRITE`.
pub const GIT_CPDIR_OVERWRITE: u32 = 1 << 3;
/// `GIT_CPDIR_CHMOD_DIRS`.
pub const GIT_CPDIR_CHMOD_DIRS: u32 = 1 << 4;
/// `GIT_CPDIR_SIMPLE_TO_MODE`.
pub const GIT_CPDIR_SIMPLE_TO_MODE: u32 = 1 << 5;
/// `GIT_CPDIR_LINK_FILES`.
pub const GIT_CPDIR_LINK_FILES: u32 = 1 << 6;
const GIT_CPDIR_MKDIR_DONE_FOR_TO_ROOT: u32 = 1 << 10;

/// `GIT_PERMS_IS_EXEC`.
#[inline]
pub const fn perms_is_exec(mode: mode_t) -> bool {
    mode & 0o100 != 0
}
/// `GIT_PERMS_CANONICAL`.
#[inline]
pub const fn perms_canonical(mode: mode_t) -> mode_t {
    if perms_is_exec(mode) { 0o755 } else { 0o644 }
}
/// `GIT_PERMS_FOR_WRITE`.
#[inline]
pub const fn perms_for_write(mode: mode_t) -> mode_t {
    if perms_is_exec(mode) { 0o777 } else { 0o666 }
}

// ---------------------------------------------------------------------------
// Creating and opening files
// ---------------------------------------------------------------------------

/// `git_futils_mkpath2file`: make the directories that contain `file_path`.
pub fn mkpath2file(file_path: &[u8], mode: mode_t) -> Result<()> {
    mkdir(
        file_path,
        mode,
        GIT_MKDIR_PATH | GIT_MKDIR_SKIP_LAST | GIT_MKDIR_VERIFY_DIR,
    )
}

/// `git_futils_mktmp`: create and open `<filename>_git2_<random>`, whose
/// path is left in `path_out`.
pub fn mktmp(path_out: &mut GitStr, filename: &[u8], mode: mode_t) -> Result<c_int> {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC;
    let filename = c_str(filename);
    for _ in 0..32 {
        let rand = crate::util::rand::next();
        let _ = path_out.sets(filename);
        let _ = path_out.puts(b"_git2_");
        let _ = path_out.encode_hexstr(&rand.to_ne_bytes());
        if path_out.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        if let Ok(fd) = posix::open(&str_cpath(path_out), flags, mode) {
            return Ok(fd);
        }
    }
    let _ = git_error_set!(
        ErrorClass::OS,
        "failed to create temporary file '{}'",
        Bytes(str_c_bytes(path_out))
    );
    path_out.dispose();
    Err(ErrorCode::GENERIC)
}

/// `git_futils_creat_withpath`.
pub fn creat_withpath(path: &CStr, dirmode: mode_t, mode: mode_t) -> Result<c_int> {
    if mkpath2file(path.to_bytes(), dirmode).is_err() {
        return Err(ErrorCode::GENERIC);
    }
    posix::creat(path, mode).map_err(|_| {
        git_error_set!(
            ErrorClass::OS,
            "failed to create file '{}'",
            Bytes(path.to_bytes())
        )
    })
}

/// `git_futils_creat_locked`: create `path` exclusively.
pub fn creat_locked(path: &CStr, mode: mode_t) -> Result<c_int> {
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC;
    posix::open(path, flags, mode).map_err(|Errno(e)| {
        let _ = git_error_set!(
            ErrorClass::OS,
            "failed to create locked file '{}'",
            Bytes(path.to_bytes())
        );
        match e {
            libc::EEXIST => ErrorCode::LOCKED,
            libc::ENOENT => ErrorCode::NOT_FOUND,
            _ => ErrorCode::GENERIC,
        }
    })
}

/// `git_futils_creat_locked_withpath`.
pub fn creat_locked_withpath(path: &CStr, dirmode: mode_t, mode: mode_t) -> Result<c_int> {
    if mkpath2file(path.to_bytes(), dirmode).is_err() {
        return Err(ErrorCode::GENERIC);
    }
    creat_locked(path, mode)
}

/// `git_futils_open_ro`.
pub fn open_ro(path: &CStr) -> Result<c_int> {
    posix::open(path, libc::O_RDONLY, 0)
        .map_err(|Errno(e)| fs_path::set_error(e, path.to_bytes(), "open"))
}

/// `git_futils_truncate`: truncate `path`, creating it with `mode`.
pub fn truncate(path: &CStr, mode: c_int) -> Result<()> {
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC;
    let fd = posix::open(path, flags, mode as mode_t)
        .map_err(|Errno(e)| fs_path::set_error(e, path.to_bytes(), "open"))?;
    // SAFETY: a descriptor this code opened, closed once.
    let _ = unsafe { posix::close(fd) };
    Ok(())
}

/// `git_futils_filesize`.
pub fn filesize(fd: c_int) -> Result<u64> {
    let st = posix::fstat(fd)
        .map_err(|_| git_error_set!(ErrorClass::OS, "failed to stat file descriptor"))?;
    if st.st_size < 0 {
        return Err(git_error_set!(ErrorClass::INVALID, "invalid file size"));
    }
    Ok(st.st_size as u64)
}

/// `git_futils_canonical_mode`: a mode git stores, for a mode from the OS.
pub fn canonical_mode(raw_mode: mode_t) -> mode_t {
    if posix::s_isreg(raw_mode) {
        libc::S_IFREG | perms_canonical(raw_mode)
    } else if posix::s_islnk(raw_mode) {
        libc::S_IFLNK
    } else if posix::s_isgitlink(raw_mode) {
        posix::S_IFGITLINK
    } else if posix::s_isdir(raw_mode) {
        libc::S_IFDIR
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Reading and writing whole files
// ---------------------------------------------------------------------------

/// Read `len` bytes from `fd` into the room after the contents of `buf`
/// (which has at least `len + 1` bytes of room); the count read, or -1.
fn read_into_spare(buf: &mut GitStr, fd: c_int, len: usize) -> isize {
    let spare = buf.spare_capacity_mut();
    assert!(spare.len() > len, "room was made for the read");
    // SAFETY: `spare` is writable for more than `len` bytes; `read_raw`
    // writes at most `len` (it only initializes them).
    unsafe { posix::read_raw(fd, spare.as_mut_ptr().cast(), len) }
}

/// `git_futils_readbuffer_fd`: read exactly `len` bytes of `fd` into `buf`.
pub fn readbuffer_fd(buf: &mut GitStr, fd: c_int, len: usize) -> Result<()> {
    buf.clear();
    // C's `git__is_ssizet(len)` check is always true on LP64: a huge `len`
    // fails the size computation below, with the out-of-memory error.
    let alloc_len = alloc_add(len, 1)?;
    buf.grow(alloc_len)?;
    let start = buf.len();
    let read_size = read_into_spare(buf, fd, len);
    if read_size < 0 {
        let _ = git_error_set!(ErrorClass::OS, "failed to read descriptor");
        buf.dispose();
        return Err(ErrorCode::GENERIC);
    }
    if read_size as usize != len {
        let _ = git_error_set!(
            ErrorClass::FILESYSTEM,
            "could not read (expected {} bytes, read {})",
            len,
            read_size as usize
        );
        buf.dispose();
        return Err(ErrorCode::GENERIC);
    }
    let n = read_size as usize;
    // C's `buf->ptr[read_size] = '\0'`: the room has `alloc_len > n` bytes.
    buf.spare_capacity_mut()[n].write(0);
    // SAFETY: owned (grown above); its first `start + n` bytes are
    // initialized (the old contents, empty, and what `read` wrote), and
    // `start + n < capacity` (followed by the NUL just written).
    unsafe { buf.set_len(start + n) };
    Ok(())
}

/// `git_futils_readbuffer_fd_full`: read `fd` to its end into `buf`.
///
/// C reads every block at the start of the buffer (rust/DEFECTS.md); this
/// keeps each block after the previous ones. The buffer grows by blocks as
/// the C's does.
pub fn readbuffer_fd_full(buf: &mut GitStr, fd: c_int) -> Result<()> {
    const BLOCKSIZE: usize = 10240;
    let mut alloc_len = 0usize;
    buf.clear();
    loop {
        alloc_len = alloc_add(alloc_len, BLOCKSIZE)?;
        buf.grow(alloc_len)?;
        let total = buf.len();
        // Each block goes after the previous ones, straight into the
        // string's room (`capacity >= alloc_len = total + BLOCKSIZE`).
        let spare = buf.spare_capacity_mut();
        assert!(spare.len() >= BLOCKSIZE, "room was made for the block");
        // SAFETY: `spare` is writable for at least `BLOCKSIZE` bytes, and
        // `read_raw` writes at most that many (it only initializes them).
        let read_size = unsafe { posix::read_raw(fd, spare.as_mut_ptr().cast(), BLOCKSIZE) };
        if read_size < 0 {
            let _ = git_error_set!(ErrorClass::OS, "failed to read descriptor");
            buf.dispose();
            return Err(ErrorCode::GENERIC);
        }
        let n = total + read_size as usize;
        // SAFETY: owned; its first `n` bytes are the earlier blocks and what
        // `read` just wrote, and `n <= alloc_len <= capacity` (a full block
        // leaves the string without its NUL until the next growth, which
        // writes it).
        unsafe { buf.set_len(n) };
        if (read_size as usize) < BLOCKSIZE {
            break;
        }
    }
    // `size < alloc_len <= capacity`: the NUL's place is in the room.
    buf.spare_capacity_mut()[0].write(0);
    Ok(())
}

/// `git_futils_readbuffer_updated`: read `path` into `out`, unless its
/// SHA-256 equals `checksum` (which is then updated); `updated` says
/// whether `out` was replaced.
pub fn readbuffer_updated(
    out: &mut GitStr,
    path: &CStr,
    checksum: Option<&mut [u8; hash::SHA256_SIZE]>,
    mut updated: Option<&mut c_int>,
) -> Result<()> {
    if let Some(u) = updated.as_deref_mut() {
        *u = 0;
    }
    let st =
        posix::stat(path).map_err(|Errno(e)| fs_path::set_error(e, path.to_bytes(), "stat"))?;
    if posix::s_isdir(st.st_mode) {
        let _ = git_error_set!(ErrorClass::INVALID, "requested file is a directory");
        return Err(ErrorCode::NOT_FOUND);
    }
    // (C's `git__is_sizet(st_size + 1)` check is always true on LP64.)
    let fd = open_ro(path)?;
    let mut buf = GitStr::new();
    let r = readbuffer_fd(&mut buf, fd, st.st_size as usize);
    // SAFETY: a descriptor this code opened, closed once.
    let _ = unsafe { posix::close(fd) };
    if r.is_err() {
        return Err(ErrorCode::GENERIC);
    }
    if let Some(checksum) = checksum {
        let digest = hash::hash_buf(buf.as_bytes(), hash::HashAlgorithm::Sha256)?;
        let mut new = [0u8; hash::SHA256_SIZE];
        new.copy_from_slice(digest.as_bytes());
        if *checksum == new {
            if let Some(u) = updated.as_deref_mut() {
                *u = 0;
            }
            return Ok(());
        }
        *checksum = new;
    }
    if let Some(u) = updated {
        *u = 1;
    }
    out.swap(&mut buf);
    Ok(())
}

/// `git_futils_readbuffer`.
pub fn readbuffer(buf: &mut GitStr, path: &CStr) -> Result<()> {
    readbuffer_updated(buf, path, None, None)
}

/// `git_futils_writebuffer`: write `buf` to `path` (with `open` flags
/// `flags`, 0 for create-truncate-write, and [`O_FSYNC`] to `fsync` the
/// file and its directory; `mode` 0 for 0100666).
pub fn writebuffer(buf: &[u8], path: &CStr, flags: c_int, mode: mode_t) -> Result<()> {
    let mut flags = if flags == 0 {
        libc::O_CREAT | libc::O_TRUNC | libc::O_WRONLY
    } else {
        flags
    };
    let do_fsync = flags & O_FSYNC != 0;
    flags &= !O_FSYNC;
    let mode = if mode == 0 {
        GIT_FILEMODE_DEFAULT
    } else {
        mode
    };
    let p = Bytes(path.to_bytes());
    let fd = posix::open(path, flags, mode)
        .map_err(|_| git_error_set!(ErrorClass::OS, "could not open '{p}' for writing"))?;
    if posix::write(fd, buf).is_err() {
        let _ = git_error_set!(ErrorClass::OS, "could not write to '{p}'");
        // SAFETY: a descriptor this code opened, closed once.
        let _ = unsafe { posix::close(fd) };
        return Err(ErrorCode::GENERIC);
    }
    if do_fsync && posix::fsync(fd).is_err() {
        let _ = git_error_set!(ErrorClass::OS, "could not fsync '{p}'");
        // SAFETY: a descriptor this code opened, closed once.
        let _ = unsafe { posix::close(fd) };
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a descriptor this code opened, closed once.
    if unsafe { posix::close(fd) }.is_err() {
        return Err(git_error_set!(ErrorClass::OS, "error while closing '{p}'"));
    }
    if do_fsync && flags & libc::O_CREAT != 0 {
        return fsync_parent(path.to_bytes());
    }
    Ok(())
}

/// `git_futils_mv_withpath`: rename `from` to `to`, making `to`'s
/// directories first.
pub fn mv_withpath(from: &CStr, to: &CStr, dirmode: mode_t) -> Result<()> {
    if mkpath2file(to.to_bytes(), dirmode).is_err() {
        return Err(ErrorCode::GENERIC);
    }
    posix::rename(from, to).map_err(|_| {
        git_error_set!(
            ErrorClass::OS,
            "failed to rename '{}' to '{}'",
            Bytes(from.to_bytes()),
            Bytes(to.to_bytes())
        )
    })
}

/// `git_futils_mmap_ro`.
pub fn mmap_ro(fd: c_int, begin: i64, len: usize) -> Result<GitMap> {
    posix::mmap(len, posix::GIT_PROT_READ, posix::GIT_MAP_PRIVATE, fd, begin)
}

/// `git_futils_mmap_ro_file`.
pub fn mmap_ro_file(path: &CStr) -> Result<GitMap> {
    let fd = open_ro(path)?;
    let r = (|| {
        let len = filesize(fd)?;
        // (C's `git__is_sizet(len)` check is always true on LP64.)
        mmap_ro(fd, 0, len as usize)
    })();
    // SAFETY: a descriptor this code opened, closed once.
    let _ = unsafe { posix::close(fd) };
    r
}

// ---------------------------------------------------------------------------
// mkdir
// ---------------------------------------------------------------------------

/// `struct git_futils_mkdir_perfdata`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MkdirPerfdata {
    /// `stat` calls made.
    pub stat_calls: usize,
    /// `mkdir` calls made.
    pub mkdir_calls: usize,
    /// `chmod` calls made.
    pub chmod_calls: usize,
}

/// `struct git_futils_mkdir_options`, mirrored: C's pool and path set
/// (`git_pool *`, `git_hashset_str *`), opaque here.
#[repr(C)]
#[derive(Debug)]
pub struct MkdirOptions {
    /// `git_pool *` that allocates the cached paths.
    pub cache_pool: *mut c_void,
    /// `git_hashset_str *` of the directories known to exist.
    pub cache_pathset: *mut c_void,
    /// Counters.
    pub perfdata: MkdirPerfdata,
}

/// A set of directories known to exist, which `mkdir` consults and extends
/// (`cache_pool` + `cache_pathset`).
pub trait MkdirCache {
    /// Whether `path` is in the set (C asks only when it has a set).
    fn contains(&mut self, path: &CStr) -> bool;
    /// Add a path: `buf` is the whole buffer, with its NUL, and the path is
    /// the C string at its start (C copies `size + 1` bytes). Does nothing
    /// without both a pool and a set.
    fn add(&mut self, buf: &[u8]) -> Result<()>;
}

/// Options of the Rust `mkdir` functions.
#[derive(Default)]
pub struct MkdirOpts<'a> {
    /// The path cache, if any.
    pub cache: Option<&'a mut dyn MkdirCache>,
    /// Counters.
    pub perfdata: MkdirPerfdata,
}

fn validate_dir(
    path: &CStr,
    st: &mut Stat,
    mode: mode_t,
    flags: u32,
    opts: &mut MkdirOpts<'_>,
) -> Result<()> {
    let p = Bytes(path.to_bytes());
    if flags & GIT_MKDIR_EXCL != 0 {
        let _ = git_error_set!(
            ErrorClass::FILESYSTEM,
            "failed to make directory '{p}': directory exists"
        );
        return Err(ErrorCode::EXISTS);
    }
    let m = st.st_mode;
    if (posix::s_isreg(m) && flags & GIT_MKDIR_REMOVE_FILES != 0)
        || (posix::s_islnk(m) && flags & GIT_MKDIR_REMOVE_SYMLINKS != 0)
    {
        if posix::unlink(path).is_err() {
            let what = if posix::s_islnk(m) { "symlink" } else { "file" };
            let _ = git_error_set!(ErrorClass::OS, "failed to remove {what} '{p}'");
            return Err(ErrorCode::EXISTS);
        }
        opts.perfdata.mkdir_calls += 1;
        if posix::mkdir(path, mode).is_err() {
            let _ = git_error_set!(ErrorClass::OS, "failed to make directory '{p}'");
            return Err(ErrorCode::EXISTS);
        }
    } else if posix::s_islnk(m) {
        opts.perfdata.stat_calls += 1;
        match posix::stat(path) {
            Ok(s) => *st = s,
            Err(_) => {
                let _ = git_error_set!(ErrorClass::OS, "failed to make directory '{p}'");
                return Err(ErrorCode::EXISTS);
            }
        }
    } else if !posix::s_isdir(m) {
        let _ = git_error_set!(
            ErrorClass::FILESYSTEM,
            "failed to make directory '{p}': directory exists"
        );
        return Err(ErrorCode::EXISTS);
    }
    Ok(())
}

fn validate_mode(
    path: &CStr,
    st: &Stat,
    terminal_path: bool,
    mode: mode_t,
    flags: u32,
    opts: &mut MkdirOpts<'_>,
) -> Result<()> {
    if ((terminal_path && flags & GIT_MKDIR_CHMOD != 0) || flags & GIT_MKDIR_CHMOD_PATH != 0)
        && st.st_mode != mode
    {
        opts.perfdata.chmod_calls += 1;
        if posix::chmod(path, mode).is_err() {
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to set permissions on '{}'",
                Bytes(path.to_bytes())
            ));
        }
    }
    Ok(())
}

fn canonicalize(path: &mut GitStr, mut flags: u32) -> Result<()> {
    if path.is_empty() {
        return Err(git_error_set!(
            ErrorClass::OS,
            "attempt to create empty path"
        ));
    }
    let root_len = match fs_path::root(path.as_bytes()) {
        r if r < 0 => 0,
        r => r as usize + 1,
    };
    while path.len() > root_len && path.as_bytes()[path.len() - 1] == b'/' {
        let n = path.len() - 1;
        path.truncate(n);
    }
    if flags & GIT_MKDIR_SKIP_LAST2 != 0 {
        let _ = fs_path::dirname_self(path);
        flags |= GIT_MKDIR_SKIP_LAST;
    }
    if flags & GIT_MKDIR_SKIP_LAST != 0 {
        let _ = fs_path::dirname_self(path);
    }
    if path.len() <= root_len {
        path.clear();
    }
    Ok(())
}

fn zeroed_stat() -> Stat {
    // SAFETY: `struct stat` is plain integers, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// `git_futils_mkdir`: make the directory `path` (and its parents with
/// [`GIT_MKDIR_PATH`]).
pub fn mkdir(path: &[u8], mode: mode_t, flags: u32) -> Result<()> {
    let mut opts = MkdirOpts::default();
    mkdir_with(path, mode, flags, &mut opts)
}

fn mkdir_with(path: &[u8], mode: mode_t, mut flags: u32, opts: &mut MkdirOpts<'_>) -> Result<()> {
    let mut make_path = GitStr::new();
    let mut parent_path = GitStr::new();
    make_path.puts(c_str(path))?;
    canonicalize(&mut make_path, flags)?;
    parent_path.puts(str_c_bytes(&make_path))?;
    if make_path.is_empty() {
        return Ok(());
    }
    let root_len = fs_path::root(make_path.as_bytes());
    let mut relative = 0usize;
    let mut depth = 0usize;
    let mut st = zeroed_stat();
    while !parent_path.is_empty() {
        match posix::lstat(&str_cpath(&parent_path)) {
            Ok(s) => {
                st = s;
                break;
            }
            Err(Errno(e)) if e != libc::ENOENT => {
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "failed to stat '{}'",
                    Bytes(str_c_bytes(&parent_path))
                ));
            }
            Err(_) => {}
        }
        depth += 1;
        let len = fs_path::dirname_self(&mut parent_path)?;
        crate::git_assert!(len != 0, "len");
        let pb = parent_path.as_bytes();
        if (len == 1 && pb[0] == b'.') || (len == 1 && pb[0] == b'/') || len <= root_len {
            relative = 0;
            break;
        }
        relative = len as usize + 1;
        if flags & GIT_MKDIR_PATH == 0 {
            break;
        }
    }
    if depth == 0 {
        let p = str_cpath(&make_path);
        validate_dir(&p, &mut st, mode, flags, opts)?;
        return validate_mode(&p, &st, true, mode, flags, opts);
    }
    flags &= !(GIT_MKDIR_SKIP_LAST2 | GIT_MKDIR_SKIP_LAST);
    let rel = &make_path.as_bytes()[relative.min(make_path.len())..];
    let base = (!parent_path.is_empty()).then(|| str_c_bytes(&parent_path));
    mkdir_relative(rel, base, mode, flags, opts)
}

/// `git_futils_mkdir_r`.
pub fn mkdir_r(path: &[u8], mode: mode_t) -> Result<()> {
    mkdir(path, mode, GIT_MKDIR_PATH)
}

/// `git_futils_mkdir_relative`: make `relative_path` under `base` (whose
/// own directories are never made).
pub fn mkdir_relative(
    relative_path: &[u8],
    base: Option<&[u8]>,
    mode: mode_t,
    flags: u32,
    opts: &mut MkdirOpts<'_>,
) -> Result<()> {
    let mut make_path = GitStr::new();
    let (relative_path, base) = (c_str(relative_path), base.map(c_str));
    let mut root = fs_path::join_unrooted(&mut make_path, relative_path, base)
        .map_err(|_| ErrorCode::GENERIC)?;
    canonicalize(&mut make_path, flags)?;
    if make_path.is_empty() {
        return Ok(());
    }
    if flags & GIT_MKDIR_PATH == 0 {
        root = make_path.rfind(b'/').map_or(-1, |i| i as isize);
    }
    let min_root_len = fs_path::root(make_path.as_bytes()) as isize;
    if root < min_root_len {
        root = min_root_len;
    }
    let size = make_path.len();
    while root >= 0 && make_path.as_bytes().get(root as usize) == Some(&b'/') {
        root += 1;
    }
    if root > size as isize {
        root = size as isize;
    }
    if root < 0 {
        root = 0;
    }
    let at = |s: &GitStr, i: usize| s.as_bytes().get(i).copied().unwrap_or(0);
    let mut tail = root as usize;
    let mut lastch = b'/';
    let mut st = zeroed_stat();
    while at(&make_path, tail) != 0 {
        while at(&make_path, tail) == b'/' {
            tail += 1;
        }
        while at(&make_path, tail) != 0 && at(&make_path, tail) != b'/' {
            tail += 1;
        }
        lastch = at(&make_path, tail);
        if tail < size {
            make_path.as_mut_bytes()[tail] = 0;
        }
        let r = mkdir_component(&make_path, &mut st, lastch, mode, flags, opts);
        if tail < size {
            make_path.as_mut_bytes()[tail] = lastch;
        }
        r?;
    }
    if flags & GIT_MKDIR_VERIFY_DIR != 0 && lastch != 0 {
        opts.perfdata.stat_calls += 1;
        let ok = posix::stat(&str_cpath(&make_path)).is_ok_and(|s| posix::s_isdir(s.st_mode));
        if !ok {
            let _ = git_error_set!(
                ErrorClass::OS,
                "path is not a directory '{}'",
                Bytes(str_c_bytes(&make_path))
            );
            return Err(ErrorCode::NOT_FOUND);
        }
    }
    Ok(())
}

/// One step of `mkdir_relative`'s walk: `make_path` is cut (by a NUL) at
/// the component to make.
fn mkdir_component(
    make_path: &GitStr,
    st: &mut Stat,
    lastch: u8,
    mode: mode_t,
    flags: u32,
    opts: &mut MkdirOpts<'_>,
) -> Result<()> {
    let p = str_cpath(make_path);
    st.st_mode = 0;
    if let Some(cache) = opts.cache.as_deref_mut()
        && cache.contains(&p)
    {
        return Ok(());
    }
    opts.perfdata.stat_calls += 1;
    let mut mkdir_attempted = false;
    loop {
        match posix::lstat(&p) {
            Ok(s) => {
                *st = s;
                validate_dir(&p, st, mode, flags, opts)?;
                break;
            }
            Err(Errno(e)) => {
                if mkdir_attempted || e != libc::ENOENT {
                    return Err(git_error_set!(
                        ErrorClass::OS,
                        "cannot access component in path '{}'",
                        Bytes(p.to_bytes())
                    ));
                }
                errors::clear();
                opts.perfdata.mkdir_calls += 1;
                mkdir_attempted = true;
                match posix::mkdir(&p, mode) {
                    Ok(()) => break,
                    Err(Errno(libc::EEXIST)) => continue,
                    Err(_) => {
                        return Err(git_error_set!(
                            ErrorClass::OS,
                            "failed to make directory '{}'",
                            Bytes(p.to_bytes())
                        ));
                    }
                }
            }
        }
    }
    validate_mode(&p, st, lastch == 0, mode, flags, opts)?;
    if let Some(cache) = opts.cache.as_deref_mut() {
        let mut buf = Vec::new();
        buf.try_reserve_exact(alloc_add(make_path.len(), 1)?)
            .map_err(|_| errors::oom())?;
        buf.extend_from_slice(make_path.as_bytes());
        buf.push(0);
        cache.add(&buf)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// rmdir -r
// ---------------------------------------------------------------------------

struct RmdirData<'a> {
    base: &'a [u8],
    baselen: usize,
    flags: u32,
    depth: c_int,
}

const FUTILS_MAX_DEPTH: c_int = 100;

fn error_cannot_rmdir(path: &[u8], filemsg: Option<&str>) -> ErrorCode {
    let path = Bytes(path);
    match filemsg {
        Some(m) => git_error_set!(ErrorClass::OS, "could not remove directory '{path}': {m}"),
        None => git_error_set!(ErrorClass::OS, "could not remove directory '{path}'"),
    }
}

fn rm_first_parent(path: &mut GitStr, ceiling: &[u8]) -> c_int {
    let mut error = ErrorCode::NOT_FOUND.raw();
    while error == ErrorCode::NOT_FOUND.raw() {
        path.rtruncate_at_char(b'/');
        if path.is_empty() || !str_c_bytes(path).starts_with(c_str(ceiling)) {
            error = 0;
        } else {
            match posix::lstat(&str_cpath(path)) {
                Ok(st) => {
                    if posix::s_isreg(st.st_mode) || posix::s_islnk(st.st_mode) {
                        error = match posix::unlink(&str_cpath(path)) {
                            Ok(()) => 0,
                            Err(_) => -1,
                        };
                    } else if !posix::s_isdir(st.st_mode) {
                        error = -1;
                    }
                }
                Err(Errno(e)) if e != libc::ENOTDIR => error = -1,
                Err(_) => {}
            }
        }
    }
    if error != 0 {
        let _ = error_cannot_rmdir(str_c_bytes(path), Some("cannot remove parent"));
    }
    error
}

fn rmdir_recurs_foreach(data: &mut RmdirData<'_>, path: &mut GitStr) -> c_int {
    if data.depth > FUTILS_MAX_DEPTH {
        return error_cannot_rmdir(str_c_bytes(path), Some("directory nesting too deep")).raw();
    }
    let cpath = str_cpath(path).into_owned();
    let st = match posix::lstat(&cpath) {
        Ok(st) => st,
        Err(Errno(libc::ENOENT)) => return 0,
        Err(Errno(libc::ENOTDIR)) => {
            if data.flags & GIT_RMDIR_REMOVE_BLOCKERS != 0 {
                return rm_first_parent(path, data.base);
            }
            let _ = error_cannot_rmdir(cpath.to_bytes(), Some("parent is not directory"));
            return -1;
        }
        Err(Errno(e)) => return fs_path::set_error(e, cpath.to_bytes(), "rmdir").raw(),
    };
    if posix::s_isdir(st.st_mode) {
        data.depth += 1;
        let r = fs_path::direach(path, 0, &mut |p: &mut GitStr| rmdir_recurs_foreach(data, p));
        data.depth -= 1;
        if let Err(e) = r
            && e.raw() < 0
        {
            return e.raw();
        }
        let error = match r {
            Ok(()) => 0,
            Err(e) => e.raw(),
        };
        if data.depth == 0 && data.flags & GIT_RMDIR_SKIP_ROOT != 0 {
            return error;
        }
        let cpath = str_cpath(path).into_owned();
        return match posix::rmdir(&cpath) {
            Ok(()) => 0,
            Err(Errno(e))
                if data.flags & GIT_RMDIR_SKIP_NONEMPTY != 0
                    && (e == libc::ENOTEMPTY || e == libc::EEXIST || e == libc::EBUSY) =>
            {
                0
            }
            Err(Errno(e)) => fs_path::set_error(e, cpath.to_bytes(), "rmdir").raw(),
        };
    }
    if data.flags & GIT_RMDIR_REMOVE_FILES != 0 {
        return match posix::unlink(&cpath) {
            Ok(()) => 0,
            Err(Errno(e)) => fs_path::set_error(e, cpath.to_bytes(), "remove").raw(),
        };
    }
    if data.flags & GIT_RMDIR_SKIP_NONEMPTY == 0 {
        return error_cannot_rmdir(cpath.to_bytes(), Some("still present")).raw();
    }
    0
}

fn rmdir_empty_parent(data: &RmdirData<'_>, path: &[u8]) -> c_int {
    if path.len() <= data.baselen {
        return ErrorCode::ITER_OVER.raw();
    }
    let cpath = to_cstring(path);
    match posix::rmdir(&cpath) {
        Ok(()) => 0,
        Err(Errno(en)) => {
            if en == libc::ENOENT || en == libc::ENOTDIR {
                0
            } else if data.flags & GIT_RMDIR_SKIP_NONEMPTY == 0 && en == libc::EBUSY {
                fs_path::set_error(en, path, "rmdir").raw()
            } else if en == libc::ENOTEMPTY || en == libc::EEXIST || en == libc::EBUSY {
                ErrorCode::ITER_OVER.raw()
            } else {
                fs_path::set_error(en, path, "rmdir").raw()
            }
        }
    }
}

/// `git_futils_rmdir_r`: remove `path` (under `base`) and what it holds,
/// as `flags` say.
pub fn rmdir_r(path: &[u8], base: Option<&[u8]>, flags: u32) -> Result<()> {
    let mut fullpath = GitStr::new();
    let (path, base) = (c_str(path), base.map(c_str));
    fs_path::join_unrooted(&mut fullpath, path, base).map_err(|_| ErrorCode::GENERIC)?;
    let base_b = c_str(base.unwrap_or(b""));
    let mut data = RmdirData {
        base: base_b,
        baselen: if base.is_some() { base_b.len() } else { 0 },
        flags,
        depth: 0,
    };
    let mut error = rmdir_recurs_foreach(&mut data, &mut fullpath);
    if error == 0 && flags & GIT_RMDIR_EMPTY_PARENTS != 0 {
        let bytes = fullpath.as_bytes().to_vec();
        error = fs_path::walk_up(&bytes, base, |step| match step {
            WalkStep::Prefix(n) => rmdir_empty_parent(&data, c_str(&bytes[..n])),
            WalkStep::Empty => rmdir_empty_parent(&data, b""),
        });
    }
    if error == ErrorCode::ITER_OVER.raw() {
        errors::clear();
        error = 0;
    }
    match ErrorCode::from_raw(error) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Copying
// ---------------------------------------------------------------------------

/// `git_futils_fake_symlink`: write `target` into a new file `path`.
pub fn fake_symlink(target: &[u8], path: &CStr) -> Result<()> {
    let fd = creat_withpath(path, 0o755, 0o644).map_err(|_| ErrorCode::GENERIC)?;
    let r = posix::write(fd, c_str(target));
    // SAFETY: a descriptor this code opened, closed once.
    let _ = unsafe { posix::close(fd) };
    r.map_err(|_| ErrorCode::GENERIC)
}

fn cp_by_fd(ifd: c_int, ofd: c_int, close_fd_when_done: bool) -> Result<()> {
    let mut buffer = Vec::new();
    let r = if buffer.try_reserve_exact(GIT_BUFSIZE_FILEIO).is_err() {
        Err(errors::oom())
    } else {
        buffer.resize(GIT_BUFSIZE_FILEIO, 0);
        let mut error = Ok(());
        let mut read_failed = false;
        loop {
            match posix::read(ifd, &mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if posix::write(ofd, &buffer[..n]).is_err() {
                        error = Err(ErrorCode::GENERIC);
                        break;
                    }
                }
                Err(_) => {
                    read_failed = true;
                    break;
                }
            }
        }
        if read_failed {
            let _ = git_error_set!(ErrorClass::OS, "read error while copying file");
            error = Err(ErrorCode::GENERIC);
        }
        if error.is_err() {
            let _ = git_error_set!(ErrorClass::OS, "write error while copying file");
        }
        error
    };
    if close_fd_when_done {
        // SAFETY: a descriptor this code opened, closed once.
        let _ = unsafe { posix::close(ifd) };
        // SAFETY: a descriptor this code opened, closed once.
        let _ = unsafe { posix::close(ofd) };
    }
    r
}

/// `git_futils_cp`: copy the file `from` to a new file `to` with mode
/// `filemode`.
pub fn cp(from: &CStr, to: &CStr, filemode: mode_t) -> Result<()> {
    let ifd = open_ro(from)?;
    let ofd = match posix::open(to, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, filemode) {
        Ok(fd) => fd,
        Err(_) => {
            // SAFETY: a descriptor this code opened, closed once.
            let _ = unsafe { posix::close(ifd) };
            return Err(fs_path::set_error(
                errors::errno(),
                to.to_bytes(),
                "open for writing",
            ));
        }
    };
    cp_by_fd(ifd, ofd, true)
}

/// `git_futils_touch`: set `path`'s access and modification times to
/// `when` (seconds; now if `None`).
pub fn touch(path: &CStr, when: Option<libc::time_t>) -> Result<()> {
    // SAFETY: `time(NULL)` has no preconditions.
    let t = when.unwrap_or_else(|| unsafe { libc::time(core::ptr::null_mut()) });
    let tv = libc::timeval {
        tv_sec: t,
        tv_usec: 0,
    };
    posix::utimes(path, &[tv, tv])
        .map_err(|Errno(e)| fs_path::set_error(e, path.to_bytes(), "touch"))
}

fn cp_link(from: &CStr, to: &CStr, link_size: usize) -> Result<()> {
    let alloc_size = alloc_add(link_size, 1)?;
    let mut link_data = Vec::new();
    link_data
        .try_reserve_exact(alloc_size)
        .map_err(|_| errors::oom())?;
    link_data.resize(alloc_size, 0);
    match posix::readlink(from, &mut link_data[..link_size]) {
        Ok(n) if n == link_size => {}
        _ => {
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to read symlink data for '{}'",
                Bytes(from.to_bytes())
            ));
        }
    }
    let target = to_cstring(&link_data[..link_size]);
    posix::symlink(&target, to).map_err(|_| {
        git_error_set!(
            ErrorClass::OS,
            "could not symlink '{}' as '{}'",
            Bytes(target.to_bytes()),
            Bytes(to.to_bytes())
        )
    })
}

struct CpRInfo<'a> {
    to_root: &'a [u8],
    to: GitStr,
    from_prefix: usize,
    flags: u32,
    mkdir_flags: u32,
    dirmode: mode_t,
}

fn cp_r_mkdir(info: &mut CpRInfo<'_>, from: &GitStr) -> Result<()> {
    let mut r = Ok(());
    if info.flags & GIT_CPDIR_MKDIR_DONE_FOR_TO_ROOT == 0 {
        let f = if info.flags & GIT_CPDIR_CHMOD_DIRS != 0 {
            GIT_MKDIR_CHMOD
        } else {
            0
        };
        r = mkdir(info.to_root, info.dirmode, f);
        info.flags |= GIT_CPDIR_MKDIR_DONE_FOR_TO_ROOT;
    }
    r?;
    let rel = &from.as_bytes()[info.from_prefix.min(from.len())..];
    mkdir_relative(
        rel,
        Some(info.to_root),
        info.dirmode,
        info.mkdir_flags,
        &mut MkdirOpts::default(),
    )
}

fn cp_r_callback(info: &mut CpRInfo<'_>, from: &mut GitStr) -> Result<()> {
    let fb = str_c_bytes(from);
    if info.flags & GIT_CPDIR_COPY_DOTFILES == 0
        && fb.get(fs_path::basename_offset(Some(from))) == Some(&b'.')
    {
        return Ok(());
    }
    let rel = c_str(&from.as_bytes()[info.from_prefix.min(from.len())..]);
    info.to.joinpath(c_str(info.to_root), rel)?;
    let to = str_cpath(&info.to).into_owned();
    let (exists, to_st) = match fs_path::lstat(&to) {
        Ok(st) => (true, Some(st)),
        Err(ErrorCode::NOT_FOUND) => {
            errors::clear();
            (false, None)
        }
        Err(e) => return Err(e),
    };
    let fromc = str_cpath(from).into_owned();
    let from_st = fs_path::lstat(&fromc)?;
    if posix::s_isdir(from_st.st_mode) {
        let oldmode = info.dirmode;
        if info.flags & GIT_CPDIR_CHMOD_DIRS == 0 {
            info.dirmode = from_st.st_mode;
        }
        let mut r = Ok(());
        if !exists && info.flags & GIT_CPDIR_CREATE_EMPTY_DIRS != 0 {
            r = cp_r_mkdir(info, from);
        }
        if r.is_ok() && (!exists || to_st.is_some_and(|s| posix::s_isdir(s.st_mode))) {
            r = fs_path::direach(
                from,
                0,
                &mut |p: &mut GitStr| match cp_r_callback(info, p) {
                    Ok(()) => 0,
                    Err(e) => e.raw(),
                },
            );
        }
        if oldmode != 0 {
            info.dirmode = oldmode;
        }
        return r;
    }
    if exists {
        if info.flags & GIT_CPDIR_OVERWRITE == 0 {
            return Ok(());
        }
        if posix::unlink(&to).is_err() {
            let _ = git_error_set!(
                ErrorClass::OS,
                "cannot overwrite existing file '{}'",
                Bytes(to.to_bytes())
            );
            return Err(ErrorCode::EXISTS);
        }
    }
    if !posix::s_isreg(from_st.st_mode)
        && (!posix::s_islnk(from_st.st_mode) || info.flags & GIT_CPDIR_COPY_SYMLINKS == 0)
    {
        return Ok(());
    }
    if info.flags & GIT_CPDIR_CREATE_EMPTY_DIRS == 0 {
        cp_r_mkdir(info, from)?;
    }
    if info.flags & GIT_CPDIR_LINK_FILES != 0 {
        posix::link(&fromc, &to).map_err(|_| {
            git_error_set!(
                ErrorClass::OS,
                "failed to link '{}'",
                Bytes(fromc.to_bytes())
            )
        })
    } else if posix::s_islnk(from_st.st_mode) {
        cp_link(&fromc, &to, from_st.st_size as usize)
    } else {
        let mut usemode = from_st.st_mode;
        if info.flags & GIT_CPDIR_SIMPLE_TO_MODE != 0 {
            usemode = perms_for_write(usemode);
        }
        cp(&fromc, &to, usemode)
    }
}

/// `git_futils_cp_r`: copy the tree `from` to `to`, as `flags` say.
pub fn cp_r(from: &[u8], to: &[u8], flags: u32, dirmode: mode_t) -> Result<()> {
    let mut path = GitStr::new();
    path.joinpath(c_str(from), b"")?;
    let mkdir_flags = if flags & GIT_CPDIR_CREATE_EMPTY_DIRS == 0 {
        let mut f = GIT_MKDIR_PATH | GIT_MKDIR_SKIP_LAST;
        if flags & GIT_CPDIR_CHMOD_DIRS != 0 {
            f |= GIT_MKDIR_CHMOD_PATH;
        }
        f
    } else if flags & GIT_CPDIR_CHMOD_DIRS != 0 {
        GIT_MKDIR_CHMOD
    } else {
        0
    };
    let mut info = CpRInfo {
        to_root: c_str(to),
        to: GitStr::new(),
        from_prefix: path.len(),
        flags,
        mkdir_flags,
        dirmode,
    };
    cp_r_callback(&mut info, &mut path)
}

// ---------------------------------------------------------------------------
// File stamps
// ---------------------------------------------------------------------------

/// `struct timespec`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    /// Seconds.
    pub tv_sec: libc::time_t,
    /// Nanoseconds.
    pub tv_nsec: libc::c_long,
}

/// `git_futils_filestamp`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filestamp {
    /// The modification time (`st_mtim`, nanoseconds included: `GIT_NSEC`).
    pub mtime: Timespec,
    /// The size.
    pub size: u64,
    /// The inode number, truncated to `unsigned int`.
    pub ino: c_uint,
}

impl Filestamp {
    /// `git_futils_filestamp_set_from_stat`.
    pub fn from_stat(st: &Stat) -> Filestamp {
        Filestamp {
            mtime: Timespec {
                tv_sec: st.st_mtime,
                tv_nsec: st.st_mtime_nsec,
            },
            size: st.st_size as u64,
            ino: st.st_ino as c_uint,
        }
    }
}

/// `git_futils_filestamp_check`: refresh `stamp` from `path`; 0 if it was
/// up to date, 1 if it changed (or there is no stamp), `GIT_ENOTFOUND`
/// (without an error message) if `path` cannot be stat'ed.
pub fn filestamp_check(stamp: Option<&mut Filestamp>, path: &CStr) -> c_int {
    let Some(stamp) = stamp else { return 1 };
    let Ok(st) = posix::stat(path) else {
        return ErrorCode::NOT_FOUND.raw();
    };
    let new = Filestamp::from_stat(&st);
    if *stamp == new {
        return 0;
    }
    *stamp = new;
    1
}

/// `git_futils_filestamp_set`: copy `source`, or clear.
pub fn filestamp_set(target: &mut Filestamp, source: Option<&Filestamp>) {
    *target = source.copied().unwrap_or_default();
}

// ---------------------------------------------------------------------------
// fsync
// ---------------------------------------------------------------------------

/// `git_futils_fsync_dir`.
pub fn fsync_dir(path: &CStr) -> Result<()> {
    let p = Bytes(path.to_bytes());
    let fd = posix::open(path, libc::O_RDONLY, 0)
        .map_err(|_| git_error_set!(ErrorClass::OS, "failed to open directory '{p}' for fsync"))?;
    let r = posix::fsync(fd)
        .map_err(|_| git_error_set!(ErrorClass::OS, "failed to fsync directory '{p}'"));
    // SAFETY: a descriptor this code opened, closed once.
    let _ = unsafe { posix::close(fd) };
    r
}

/// `git_futils_fsync_parent`: `fsync` the directory that contains `path`.
pub fn fsync_parent(path: &[u8]) -> Result<()> {
    let Some(parent) = fs_path::dirname(Some(path)) else {
        return Err(ErrorCode::GENERIC);
    };
    fsync_dir(parent.as_c_str())
}

/// The C ABI of `futils.c`.
pub mod ffi;

crate::c_layout_test! {
    headers: ["futils.h"],
    types: [
        Filestamp => "git_futils_filestamp" { mtime, size, ino },
        Timespec => "struct timespec" { tv_sec, tv_nsec },
        MkdirPerfdata => "struct git_futils_mkdir_perfdata" { stat_calls, mkdir_calls, chmod_calls },
        MkdirOptions => "struct git_futils_mkdir_options" { cache_pool, cache_pathset, perfdata },
    ],
    asserts: [
        "GIT_MKDIR_EXCL == 1", "GIT_MKDIR_PATH == 2", "GIT_MKDIR_CHMOD == 4",
        "GIT_MKDIR_CHMOD_PATH == 8", "GIT_MKDIR_SKIP_LAST == 16", "GIT_MKDIR_SKIP_LAST2 == 32",
        "GIT_MKDIR_VERIFY_DIR == 64", "GIT_MKDIR_REMOVE_FILES == 128",
        "GIT_MKDIR_REMOVE_SYMLINKS == 256",
        "GIT_RMDIR_REMOVE_FILES == 1", "GIT_RMDIR_SKIP_NONEMPTY == 2",
        "GIT_RMDIR_EMPTY_PARENTS == 4", "GIT_RMDIR_REMOVE_BLOCKERS == 8",
        "GIT_RMDIR_SKIP_ROOT == 16",
        "GIT_CPDIR_CREATE_EMPTY_DIRS == 1", "GIT_CPDIR_COPY_SYMLINKS == 2",
        "GIT_CPDIR_COPY_DOTFILES == 4", "GIT_CPDIR_OVERWRITE == 8", "GIT_CPDIR_CHMOD_DIRS == 16",
        "GIT_CPDIR_SIMPLE_TO_MODE == 32", "GIT_CPDIR_LINK_FILES == 64",
        "O_FSYNC == 04010000", "GIT_BUFSIZE_FILEIO == 65536",
    ],
}

#[cfg(test)]
mod tests {
    //! What runs without a filesystem (and so under Miri); the rest is in
    //! `tests/util_fs.rs` and `rust/difftest/fs_diff.c`.
    use super::*;
    use crate::util::test_support;

    #[test]
    fn canonical_modes() {
        assert_eq!(canonical_mode(0o100644), 0o100644);
        assert_eq!(canonical_mode(0o100755), 0o100755);
        assert_eq!(canonical_mode(0o100600), 0o100644);
        assert_eq!(canonical_mode(0o100700), 0o100755);
        assert_eq!(canonical_mode(0o120777), 0o120000);
        assert_eq!(canonical_mode(0o160000), 0o160000);
        assert_eq!(canonical_mode(0o040755), 0o040000);
        assert_eq!(canonical_mode(0o010644), 0);
        assert_eq!(
            (perms_canonical(0o700), perms_for_write(0o644)),
            (0o755, 0o666)
        );
    }

    #[test]
    fn filestamps() {
        // SAFETY: `struct stat` is plain integers.
        let mut st: Stat = unsafe { core::mem::zeroed() };
        st.st_mtime = 7;
        st.st_mtime_nsec = 9;
        st.st_size = 42;
        st.st_ino = (1u64 << 33) | 5;
        let s = Filestamp::from_stat(&st);
        assert_eq!(
            s,
            Filestamp {
                mtime: Timespec {
                    tv_sec: 7,
                    tv_nsec: 9
                },
                size: 42,
                ino: 5,
            }
        );
        let mut t = Filestamp::default();
        filestamp_set(&mut t, Some(&s));
        assert_eq!(t, s);
        filestamp_set(&mut t, None);
        assert_eq!(t, Filestamp::default());
        assert_eq!(filestamp_check(None, c"/nonexistent"), 1);
    }

    #[test]
    fn mkdir_paths_that_need_no_system_call() {
        let _g = test_support::init();
        assert_eq!(mkdir(b"", 0o755, GIT_MKDIR_PATH), Err(ErrorCode::GENERIC));
        assert!(
            errors::last_message()
                .unwrap()
                .starts_with(b"attempt to create empty path")
        );
        // The root (or a path that SKIP_LAST trims to it): nothing to make.
        assert_eq!(mkdir(b"/", 0o755, GIT_MKDIR_PATH), Ok(()));
        assert_eq!(mkdir(b"///", 0o755, 0), Ok(()));
        assert_eq!(mkdir(b"/x", 0o755, GIT_MKDIR_SKIP_LAST), Ok(()));
        let mut o = MkdirOpts::default();
        assert_eq!(
            mkdir_relative(b"", Some(b"/"), 0o755, GIT_MKDIR_PATH, &mut o),
            Ok(())
        );
        assert_eq!(o.perfdata, MkdirPerfdata::default());
    }
}
