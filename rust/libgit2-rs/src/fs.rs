//! The safe Rust API of the filesystem layer (`fs_path.c`, `futils.c`,
//! `filebuf.c`): path manipulation, reading and writing whole files,
//! directories, lock files.
//!
//! Paths are bytes (`&[u8]`; on Unix, `std::os::unix::ffi::OsStrExt`
//! converts a `Path`). A path with a NUL byte is refused (an `INVALID`
//! error, or `false`), as `std` does, rather than cut at the NUL as the C
//! would read it; `basename` and `root` read up to the NUL. Failures are
//! [`Error`]s with libgit2's code, class and message.
//!
//! ```
//! # let _g = libgit2_rs::util::test_support::init();
//! use libgit2_rs::fs;
//! assert_eq!(fs::dirname(b"a/b/c").unwrap(), b"a/b");
//! assert_eq!(fs::basename(b"a/b/c/"), b"c");
//! assert_eq!(fs::resolve_relative(b"a/./b/../c").unwrap(), b"a/c");
//! assert!(!fs::is_valid(b"a/../b", fs::REJECT_TRAVERSAL));
//! ```

#![forbid(unsafe_code)]

use crate::util::errors::Error;
use crate::util::fs_path::{self, GitFsPathDiriter, WalkStep};
use crate::util::futils;
use crate::util::posix::mode_t;
use crate::util::str::GitStr;

pub use crate::util::filebuf::Filebuf;
pub use crate::util::filebuf::{
    GIT_FILEBUF_APPEND as FILEBUF_APPEND,
    GIT_FILEBUF_CREATE_LEADING_DIRS as FILEBUF_CREATE_LEADING_DIRS,
    GIT_FILEBUF_DEFLATE_SHIFT as FILEBUF_DEFLATE_SHIFT,
    GIT_FILEBUF_DO_NOT_BUFFER as FILEBUF_DO_NOT_BUFFER, GIT_FILEBUF_FSYNC as FILEBUF_FSYNC,
    GIT_FILEBUF_HASH_SHA1 as FILEBUF_HASH_SHA1, GIT_FILEBUF_HASH_SHA256 as FILEBUF_HASH_SHA256,
    GIT_FILEBUF_TEMPORARY as FILEBUF_TEMPORARY,
};
pub use crate::util::fs_path::{
    GIT_FS_PATH_REJECT_BACKSLASH as REJECT_BACKSLASH,
    GIT_FS_PATH_REJECT_DOS_PATHS as REJECT_DOS_PATHS,
    GIT_FS_PATH_REJECT_EMPTY_COMPONENT as REJECT_EMPTY_COMPONENT,
    GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS as REJECT_FILESYSTEM_DEFAULTS,
    GIT_FS_PATH_REJECT_NT_CHARS as REJECT_NT_CHARS, GIT_FS_PATH_REJECT_SLASH as REJECT_SLASH,
    GIT_FS_PATH_REJECT_TRAILING_COLON as REJECT_TRAILING_COLON,
    GIT_FS_PATH_REJECT_TRAILING_DOT as REJECT_TRAILING_DOT,
    GIT_FS_PATH_REJECT_TRAILING_SPACE as REJECT_TRAILING_SPACE,
    GIT_FS_PATH_REJECT_TRAVERSAL as REJECT_TRAVERSAL,
};
pub use crate::util::futils::{
    Filestamp, GIT_CPDIR_CHMOD_DIRS as CPDIR_CHMOD_DIRS,
    GIT_CPDIR_COPY_DOTFILES as CPDIR_COPY_DOTFILES, GIT_CPDIR_COPY_SYMLINKS as CPDIR_COPY_SYMLINKS,
    GIT_CPDIR_CREATE_EMPTY_DIRS as CPDIR_CREATE_EMPTY_DIRS,
    GIT_CPDIR_LINK_FILES as CPDIR_LINK_FILES, GIT_CPDIR_OVERWRITE as CPDIR_OVERWRITE,
    GIT_CPDIR_SIMPLE_TO_MODE as CPDIR_SIMPLE_TO_MODE, GIT_MKDIR_CHMOD as MKDIR_CHMOD,
    GIT_MKDIR_CHMOD_PATH as MKDIR_CHMOD_PATH, GIT_MKDIR_EXCL as MKDIR_EXCL,
    GIT_MKDIR_PATH as MKDIR_PATH, GIT_MKDIR_REMOVE_FILES as MKDIR_REMOVE_FILES,
    GIT_MKDIR_REMOVE_SYMLINKS as MKDIR_REMOVE_SYMLINKS, GIT_MKDIR_SKIP_LAST as MKDIR_SKIP_LAST,
    GIT_MKDIR_SKIP_LAST2 as MKDIR_SKIP_LAST2, GIT_MKDIR_VERIFY_DIR as MKDIR_VERIFY_DIR,
    GIT_RMDIR_EMPTY_HIERARCHY as RMDIR_EMPTY_HIERARCHY,
    GIT_RMDIR_EMPTY_PARENTS as RMDIR_EMPTY_PARENTS,
    GIT_RMDIR_REMOVE_BLOCKERS as RMDIR_REMOVE_BLOCKERS,
    GIT_RMDIR_REMOVE_FILES as RMDIR_REMOVE_FILES, GIT_RMDIR_SKIP_NONEMPTY as RMDIR_SKIP_NONEMPTY,
    GIT_RMDIR_SKIP_ROOT as RMDIR_SKIP_ROOT, MkdirCache, MkdirPerfdata,
};

/// Whether `path` has a NUL byte: the safe API refuses such paths (C would
/// read them up to the NUL), as `std` does.
fn has_nul(path: &[u8]) -> bool {
    memchr::memchr(0, path).is_some()
}

/// The error for a path with a NUL byte (`GIT_ERROR_INVALID`).
fn nul_error() -> Error {
    crate::util::errors::set_str(
        crate::util::errors::ErrorClass::INVALID,
        "path contains a NUL byte",
    )
    .into()
}

/// The result type of this module.
pub type Result<T> = core::result::Result<T, Error>;

fn bytes(s: &GitStr) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn with_str(f: impl FnOnce(&mut GitStr) -> crate::util::errors::Result<()>) -> Result<Vec<u8>> {
    let mut s = GitStr::new();
    f(&mut s)?;
    Ok(bytes(&s))
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// The directory part of `path` (`"."` without a slash, `"/"` for the
/// root), as POSIX `dirname`.
pub fn dirname(path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    with_str(|s| fs_path::dirname_r(Some(s), Some(fs_path::c_str(path))).map(|_| ()))
}

/// The last component of `path` without trailing slashes (`"."` for an
/// empty path, `"/"` for the root), as POSIX `basename`.
pub fn basename(path: &[u8]) -> Vec<u8> {
    let path = fs_path::c_str(path);
    fs_path::basename_piece(Some(path)).bytes(path).to_vec()
}

/// The offset of `path`'s root slash (after a DOS drive such as `C:`), or
/// `None` for a relative path.
pub fn root(path: &[u8]) -> Option<usize> {
    usize::try_from(fs_path::root(path)).ok()
}

/// `path` with a trailing slash (unless it has one or is empty).
pub fn to_dir(path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    with_str(|s| {
        s.puts(path)?;
        fs_path::to_dir(s)
    })
}

/// `path` joined to `base` unless it is rooted, and the offset of the root
/// in the result.
pub fn join_unrooted(path: &[u8], base: Option<&[u8]>) -> Result<(Vec<u8>, usize)> {
    if has_nul(path) {
        return Err(nul_error());
    }
    if base.is_some_and(has_nul) {
        return Err(nul_error());
    }
    let mut s = GitStr::new();
    let r = fs_path::join_unrooted(&mut s, fs_path::c_str(path), base.map(fs_path::c_str))?;
    Ok((bytes(&s), r.max(0) as usize))
}

/// `path` with runs of slashes collapsed.
pub fn squash_slashes(path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    with_str(|s| {
        s.puts(path)?;
        fs_path::squash_slashes(s);
        Ok(())
    })
}

/// `path` with its `.` and `..` components resolved, without backing up
/// over the root (or a URL's `scheme://`).
pub fn resolve_relative(path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    with_str(|s| {
        s.puts(path)?;
        fs_path::resolve_relative(s, 0)
    })
}

/// `relpath` applied to `base` (joined, then resolved).
pub fn apply_relative(base: &[u8], relpath: &[u8]) -> Result<Vec<u8>> {
    if has_nul(base) {
        return Err(nul_error());
    }
    if has_nul(relpath) {
        return Err(nul_error());
    }
    with_str(|s| {
        s.puts(base)?;
        fs_path::apply_relative(s, relpath)
    })
}

/// `path` relative to `parent` (`../` for each extra level of `parent`).
pub fn make_relative(path: &[u8], parent: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    if has_nul(parent) {
        return Err(nul_error());
    }
    with_str(|s| {
        s.puts(path)?;
        fs_path::make_relative(s, parent)
    })
}

/// The length of the directory prefix that `one` and `two` share, its
/// slash included.
pub fn common_dirlen(one: &[u8], two: &[u8]) -> usize {
    fs_path::common_dirlen(one, two)
}

/// The canonical absolute path of `path` (joined to `base` first when it
/// is relative); the path must exist.
pub fn prettify(path: &[u8], base: Option<&[u8]>) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    if base.is_some_and(has_nul) {
        return Err(nul_error());
    }
    let p = fs_path::to_cstring(path);
    with_str(|s| fs_path::prettify(s, &p, base))
}

/// [`prettify`] with a trailing slash.
pub fn prettify_dir(path: &[u8], base: Option<&[u8]>) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    if base.is_some_and(has_nul) {
        return Err(nul_error());
    }
    let p = fs_path::to_cstring(path);
    with_str(|s| fs_path::prettify_dir(s, &p, base))
}

/// `%XX` escapes decoded (invalid ones are kept as they are).
pub fn percent_decode(input: &[u8]) -> Result<Vec<u8>> {
    if has_nul(input) {
        return Err(nul_error());
    }
    with_str(|s| fs_path::percent_decode(s, input))
}

/// Whether `url` is a local `file://` URL.
pub fn is_local_file_url(url: &[u8]) -> bool {
    if has_nul(url) {
        return false;
    }
    fs_path::is_local_file_url(url)
}

/// The local path of a `file://` URL.
pub fn from_url(url: &[u8]) -> Result<Vec<u8>> {
    if has_nul(url) {
        return Err(nul_error());
    }
    with_str(|s| fs_path::fromurl(s, url))
}

/// The local path of a `file://` URL, or the path itself.
pub fn from_url_or_path(url_or_path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(url_or_path) {
        return Err(nul_error());
    }
    with_str(|s| fs_path::from_url_or_path(s, url_or_path))
}

/// Whether `path` is valid under `flags` (the `REJECT_*` constants).
pub fn is_valid(path: &[u8], flags: u32) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::is_valid(path, flags)
}

/// Whether `path` is valid under `flags`, with extra checks of each byte
/// and of each component.
pub fn is_valid_with(
    path: &[u8],
    flags: u32,
    mut char_ok: impl FnMut(u8) -> bool,
    mut component_ok: impl FnMut(&[u8]) -> bool,
) -> bool {
    fs_path::str_is_valid_ext(
        path,
        true,
        flags,
        fs_path::ValidateCallbacks {
            char_cb: Some(&mut char_ok),
            component_cb: Some(&mut component_ok),
        },
    )
}

/// Call `f` with `path` and each parent directory (with its trailing
/// slash) up to `ceiling`, then with `""` for a relative path without a
/// ceiling; stop at the first `Err`.
pub fn walk_up<E>(
    path: &[u8],
    ceiling: Option<&[u8]>,
    mut f: impl FnMut(&[u8]) -> core::result::Result<(), E>,
) -> core::result::Result<(), E> {
    let mut err = None;
    let r = fs_path::walk_up(path, ceiling, |step| {
        let p = match step {
            WalkStep::Prefix(n) => &path[..n],
            WalkStep::Empty => &b""[..],
        };
        match f(p) {
            Ok(()) => 0,
            Err(e) => {
                err = Some(e);
                1
            }
        }
    });
    match err {
        Some(e) if r != 0 => Err(e),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// The filesystem
// ---------------------------------------------------------------------------

/// Whether `path` exists.
pub fn exists(path: &[u8]) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::exists(&fs_path::to_cstring(path))
}

/// Whether `path` is a directory (following symlinks).
pub fn is_dir(path: &[u8]) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::isdir(&fs_path::to_cstring(path))
}

/// Whether `path` is a regular file (following symlinks).
pub fn is_file(path: &[u8]) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::isfile(&fs_path::to_cstring(path))
}

/// Whether `path` is a symlink.
pub fn is_link(path: &[u8]) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::islink(&fs_path::to_cstring(path))
}

/// Whether `path` is an empty directory.
pub fn is_empty_dir(path: &[u8]) -> bool {
    if has_nul(path) {
        return false;
    }
    fs_path::is_empty_dir(&fs_path::to_cstring(path))
}

/// Whether `path` is owned by the current user.
pub fn owner_is_current_user(path: &[u8]) -> Result<bool> {
    if has_nul(path) {
        return Err(nul_error());
    }
    Ok(fs_path::owner_is_current_user(&fs_path::to_cstring(path))?)
}

/// The path of `executable` (searched in `$PATH` unless it has a slash).
pub fn find_executable(executable: &[u8]) -> Result<Vec<u8>> {
    if has_nul(executable) {
        return Err(nul_error());
    }
    with_str(|s| fs_path::find_executable(s, executable))
}

/// The contents of the file `path`.
pub fn read(path: &[u8]) -> Result<Vec<u8>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    let p = fs_path::to_cstring(path);
    with_str(|s| futils::readbuffer(s, &p))
}

/// The contents of `path` if their SHA-256 differs from `checksum`, which
/// is then updated; `None` when unchanged.
pub fn read_if_changed(path: &[u8], checksum: &mut [u8; 32]) -> Result<Option<Vec<u8>>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    let p = fs_path::to_cstring(path);
    let mut s = GitStr::new();
    let mut updated = 0;
    futils::readbuffer_updated(&mut s, &p, Some(checksum), Some(&mut updated))?;
    Ok((updated != 0).then(|| bytes(&s)))
}

/// Write `data` to `path` (created or truncated, with `mode`; 0 for
/// 0666), `fsync`ing it and its directory if `fsync`.
pub fn write(path: &[u8], data: &[u8], mode: mode_t, fsync: bool) -> Result<()> {
    if has_nul(path) {
        return Err(nul_error());
    }
    let flags =
        libc::O_CREAT | libc::O_TRUNC | libc::O_WRONLY | if fsync { futils::O_FSYNC } else { 0 };
    Ok(futils::writebuffer(
        data,
        &fs_path::to_cstring(path),
        flags,
        mode,
    )?)
}

/// Make the directory `path` as `flags` (`MKDIR_*`) say.
pub fn mkdir(path: &[u8], mode: mode_t, flags: u32) -> Result<()> {
    if has_nul(path) {
        return Err(nul_error());
    }
    Ok(futils::mkdir(path, mode, flags)?)
}

/// Make `relative` under `base`, with a path cache and counters.
pub fn mkdir_relative(
    relative: &[u8],
    base: Option<&[u8]>,
    mode: mode_t,
    flags: u32,
    cache: Option<&mut dyn MkdirCache>,
) -> Result<MkdirPerfdata> {
    if has_nul(relative) {
        return Err(nul_error());
    }
    if base.is_some_and(has_nul) {
        return Err(nul_error());
    }
    let mut opts = futils::MkdirOpts {
        cache,
        perfdata: MkdirPerfdata::default(),
    };
    futils::mkdir_relative(relative, base, mode, flags, &mut opts)?;
    Ok(opts.perfdata)
}

/// Make `path` and its parents.
pub fn mkdir_p(path: &[u8], mode: mode_t) -> Result<()> {
    if has_nul(path) {
        return Err(nul_error());
    }
    Ok(futils::mkdir_r(path, mode)?)
}

/// Remove `path` (under `base`) as `flags` (`RMDIR_*`) say.
pub fn rmdir_r(path: &[u8], base: Option<&[u8]>, flags: u32) -> Result<()> {
    if has_nul(path) {
        return Err(nul_error());
    }
    if base.is_some_and(has_nul) {
        return Err(nul_error());
    }
    Ok(futils::rmdir_r(path, base, flags)?)
}

/// Copy the file `from` to a new file `to`.
pub fn copy(from: &[u8], to: &[u8], mode: mode_t) -> Result<()> {
    if has_nul(from) {
        return Err(nul_error());
    }
    if has_nul(to) {
        return Err(nul_error());
    }
    Ok(futils::cp(
        &fs_path::to_cstring(from),
        &fs_path::to_cstring(to),
        mode,
    )?)
}

/// Copy the tree `from` to `to` as `flags` (`CPDIR_*`) say.
pub fn copy_tree(from: &[u8], to: &[u8], flags: u32, dirmode: mode_t) -> Result<()> {
    if has_nul(from) {
        return Err(nul_error());
    }
    if has_nul(to) {
        return Err(nul_error());
    }
    Ok(futils::cp_r(from, to, flags, dirmode)?)
}

/// Rename `from` to `to`, making `to`'s directories.
pub fn rename_with_path(from: &[u8], to: &[u8], dirmode: mode_t) -> Result<()> {
    if has_nul(from) {
        return Err(nul_error());
    }
    if has_nul(to) {
        return Err(nul_error());
    }
    Ok(futils::mv_withpath(
        &fs_path::to_cstring(from),
        &fs_path::to_cstring(to),
        dirmode,
    )?)
}

/// The paths of the entries of the directory `path`, each without its
/// first `prefix_len` bytes, in the directory's order.
pub fn read_dir(path: &[u8], prefix_len: usize) -> Result<Vec<Vec<u8>>> {
    if has_nul(path) {
        return Err(nul_error());
    }
    let mut out = Vec::new();
    for e in DirIter::new(path, 0)? {
        let e = e?;
        if e.path.len() <= prefix_len {
            return Err(crate::util::errors::assert_failed("name_len > prefix_len").into());
        }
        out.push(e.path[prefix_len..].to_vec());
    }
    Ok(out)
}

impl Filestamp {
    /// Refresh the stamp from `path`: `Ok(true)` if the file changed,
    /// `Ok(false)` if not, an error if it cannot be stat'ed.
    pub fn check(&mut self, path: &[u8]) -> Result<bool> {
        if has_nul(path) {
            return Err(nul_error());
        }
        let p = fs_path::to_cstring(path);
        match futils::filestamp_check(Some(self), &p) {
            0 => Ok(false),
            1 => Ok(true),
            // C sets no error here: say why, rather than report whatever
            // error the thread last had.
            _ => Err(fs_path::set_error(crate::util::errors::errno(), path, "stat").into()),
        }
    }
}

/// An entry of a [`DirIter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The directory's path, a slash, and the name.
    pub path: Vec<u8>,
    /// The offset of the name in `path`.
    pub name_offset: usize,
}

impl DirEntry {
    /// The entry's name.
    pub fn name(&self) -> &[u8] {
        &self.path[self.name_offset..]
    }

    /// `lstat` of the entry: its mode.
    pub fn mode(&self) -> Result<mode_t> {
        Ok(fs_path::lstat(&fs_path::to_cstring(&self.path))?.st_mode)
    }
}

/// The entries of a directory (`git_fs_path_diriter`), without `.` and
/// `..` unless asked.
#[derive(Debug)]
pub struct DirIter {
    inner: GitFsPathDiriter,
    done: bool,
}

impl DirIter {
    /// Open the directory `path` (`flags`: `GIT_FS_PATH_DIR_*`).
    pub fn new(path: &[u8], flags: u32) -> Result<DirIter> {
        if has_nul(path) {
            return Err(nul_error());
        }
        let mut inner = GitFsPathDiriter::new();
        inner.init(path, flags)?;
        Ok(DirIter { inner, done: false })
    }
}

impl Iterator for DirIter {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.inner.next() {
            Ok(()) => {
                let path = self.inner.fullpath().to_vec();
                let name_len = self.inner.filename().map_or(0, <[u8]>::len);
                Some(Ok(DirEntry {
                    name_offset: path.len() - name_len,
                    path,
                }))
            }
            Err(crate::util::errors::ErrorCode::ITER_OVER) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e.into()))
            }
        }
    }
}
