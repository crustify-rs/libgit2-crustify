//! Port of `src/util/fs_path.c` (and the inline helpers of `fs_path.h`):
//! filesystem path manipulation, directory iteration, path validation and
//! ownership checks.
//!
//! Paths are bytes. The pure functions take `&[u8]` and treat it as the C
//! treats a `const char *`: the bytes up to the first NUL (a slice without
//! NUL is the whole string). The hottest ones (`basename`, `dirname`,
//! `dirlen`, `join_unrooted`) take the bytes of a C string, without a NUL,
//! as the exports and the Rust callers have them ([`c_str`] makes them). Functions that reach the filesystem take
//! `&CStr`. Functions on a `git_str *` take [`GitStr`] and, as the C, read
//! its contents as a C string (up to the first NUL).
//!
//! Only the Unix branch of the C is ported (no `GIT_WIN32`, no
//! `GIT_I18N_ICONV`, which the reference configuration does not define:
//! [`does_decompose_unicode`] is `false`).
//!
//! # Types
//!
//! - [`GitFsPathDiriter`] is `git_fs_path_diriter`, mirrored (kind 3: C
//!   embeds it by value, `GIT_FS_PATH_DIRITER_INIT`, in `iterator.c` and the
//!   tests).

use core::ffi::{CStr, c_char, c_int, c_uint};
use core::sync::atomic::{AtomicU32, Ordering};
use std::borrow::Cow;
use std::ffi::CString;

use super::alloc::{self, GitCStr};
use super::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use super::integer::alloc_add;
use super::posix::{self, Stat};
use super::str::GitStr;
use super::vector::GitVector;
use crate::{git_assert, git_error_set};

pub use super::posix::GIT_PATH_MAX;

/// `GIT_PATH_LIST_SEPARATOR` (Unix).
pub const GIT_PATH_LIST_SEPARATOR: u8 = b':';

/// `GIT_FS_PATH_DIR_IGNORE_CASE`.
pub const GIT_FS_PATH_DIR_IGNORE_CASE: c_uint = 1 << 0;
/// `GIT_FS_PATH_DIR_PRECOMPOSE_UNICODE`.
pub const GIT_FS_PATH_DIR_PRECOMPOSE_UNICODE: c_uint = 1 << 1;
/// `GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT`.
pub const GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT: c_uint = 1 << 2;

/// `GIT_FS_PATH_NOTEQUAL`.
pub const GIT_FS_PATH_NOTEQUAL: c_int = 0;
/// `GIT_FS_PATH_EQUAL`.
pub const GIT_FS_PATH_EQUAL: c_int = 1;
/// `GIT_FS_PATH_PREFIX`.
pub const GIT_FS_PATH_PREFIX: c_int = 2;

/// `GIT_FS_PATH_REJECT_EMPTY_COMPONENT`.
pub const GIT_FS_PATH_REJECT_EMPTY_COMPONENT: c_uint = 1 << 0;
/// `GIT_FS_PATH_REJECT_TRAVERSAL`.
pub const GIT_FS_PATH_REJECT_TRAVERSAL: c_uint = 1 << 1;
/// `GIT_FS_PATH_REJECT_SLASH`.
pub const GIT_FS_PATH_REJECT_SLASH: c_uint = 1 << 2;
/// `GIT_FS_PATH_REJECT_BACKSLASH`.
pub const GIT_FS_PATH_REJECT_BACKSLASH: c_uint = 1 << 3;
/// `GIT_FS_PATH_REJECT_TRAILING_DOT`.
pub const GIT_FS_PATH_REJECT_TRAILING_DOT: c_uint = 1 << 4;
/// `GIT_FS_PATH_REJECT_TRAILING_SPACE`.
pub const GIT_FS_PATH_REJECT_TRAILING_SPACE: c_uint = 1 << 5;
/// `GIT_FS_PATH_REJECT_TRAILING_COLON`.
pub const GIT_FS_PATH_REJECT_TRAILING_COLON: c_uint = 1 << 6;
/// `GIT_FS_PATH_REJECT_DOS_PATHS`.
pub const GIT_FS_PATH_REJECT_DOS_PATHS: c_uint = 1 << 7;
/// `GIT_FS_PATH_REJECT_NT_CHARS`.
pub const GIT_FS_PATH_REJECT_NT_CHARS: c_uint = 1 << 8;
/// `GIT_FS_PATH_REJECT_LONG_PATHS`.
pub const GIT_FS_PATH_REJECT_LONG_PATHS: c_uint = 1 << 9;
/// `GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS` (Unix).
pub const GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS: c_uint =
    GIT_FS_PATH_REJECT_EMPTY_COMPONENT | GIT_FS_PATH_REJECT_TRAVERSAL;

/// `GIT_FS_PATH_OWNER_NONE`.
pub const GIT_FS_PATH_OWNER_NONE: c_uint = 0;
/// `GIT_FS_PATH_OWNER_CURRENT_USER`.
pub const GIT_FS_PATH_OWNER_CURRENT_USER: c_uint = 1 << 0;
/// `GIT_FS_PATH_OWNER_ADMINISTRATOR`.
pub const GIT_FS_PATH_OWNER_ADMINISTRATOR: c_uint = 1 << 1;
/// `GIT_FS_PATH_USER_IS_ADMINISTRATOR` (Windows only; ignored).
pub const GIT_FS_PATH_USER_IS_ADMINISTRATOR: c_uint = 1 << 2;
/// `GIT_FS_PATH_OWNER_RUNNING_SUDO`.
pub const GIT_FS_PATH_OWNER_RUNNING_SUDO: c_uint = 1 << 3;
/// `GIT_FS_PATH_OWNER_OTHER`.
pub const GIT_FS_PATH_OWNER_OTHER: c_uint = 1 << 4;

// ---------------------------------------------------------------------------
// C-string views
// ---------------------------------------------------------------------------

/// The byte at `i` of the C string `p`: 0 at and past its end.
#[inline(always)]
fn at(p: &[u8], i: usize) -> u8 {
    match p.get(i) {
        Some(&c) => c,
        None => 0,
    }
}

/// `p` up to its first NUL (what C's `strlen` sees).
#[inline]
pub fn c_str(p: &[u8]) -> &[u8] {
    match memchr::memchr(0, p) {
        Some(n) => &p[..n],
        None => p,
    }
}

/// The contents of `s` as C reads `s->ptr`: up to the first NUL.
#[inline]
pub fn str_c_bytes(s: &GitStr) -> &[u8] {
    c_str(s.as_bytes())
}

/// The contents of `s` as a C string for a system call: borrowed from an
/// owned buffer, copied from a borrowed one.
pub fn str_cpath(s: &GitStr) -> Cow<'_, CStr> {
    match s.as_c_str() {
        Some(c) => Cow::Borrowed(c),
        None => Cow::Owned(to_cstring(s.as_bytes())),
    }
}

/// `p` (up to its first NUL) as an owned C string.
pub fn to_cstring(p: &[u8]) -> CString {
    // No NUL remains in `c_str(p)`.
    CString::new(c_str(p)).unwrap_or_default()
}

/// A C string read byte by byte, as the C loops read it: `at(i)` for each
/// `i` up to the first NUL, without measuring the string first.
pub trait CBytes: Copy {
    /// The byte at `i` (0 at and past the end). Called only for `i` up to
    /// the index of the first NUL.
    fn at(self, i: usize) -> u8;
    /// The bytes `start..end`, all read with [`at`](Self::at) before (so
    /// none is the NUL).
    fn seg<'a>(self, start: usize, end: usize) -> &'a [u8]
    where
        Self: 'a;
}

impl CBytes for &[u8] {
    #[inline(always)]
    fn at(self, i: usize) -> u8 {
        self::at(self, i)
    }

    #[inline(always)]
    fn seg<'a>(self, start: usize, end: usize) -> &'a [u8]
    where
        Self: 'a,
    {
        &self[start..end]
    }
}

/// A NUL-terminated string behind a raw pointer, as an export receives it
/// ([`CBytes`] without a `strlen`).
#[derive(Clone, Copy)]
pub struct RawCStr<'a>(core::ptr::NonNull<u8>, core::marker::PhantomData<&'a [u8]>);

impl<'a> RawCStr<'a> {
    /// The string at `p`.
    ///
    /// # Safety
    ///
    /// `p` points to a NUL-terminated string that stays readable and
    /// unwritten for `'a`.
    #[inline(always)]
    pub unsafe fn new(p: core::ptr::NonNull<c_char>) -> RawCStr<'a> {
        RawCStr(p.cast(), core::marker::PhantomData)
    }
}

impl<'b> CBytes for RawCStr<'b> {
    #[inline(always)]
    fn at(self, i: usize) -> u8 {
        // SAFETY: the string is readable up to its NUL (`new`'s contract),
        // and `at` is asked only for bytes up to the NUL (the trait's).
        unsafe { self.0.as_ptr().wrapping_add(i).read() }
    }

    #[inline(always)]
    fn seg<'a>(self, start: usize, end: usize) -> &'a [u8]
    where
        Self: 'a,
    {
        // SAFETY: `start..end` were read with `at`: readable, before the
        // NUL, and unwritten for `'b: 'a`.
        unsafe { core::slice::from_raw_parts(self.0.as_ptr().wrapping_add(start), end - start) }
    }
}

// ---------------------------------------------------------------------------
// Inline helpers of fs_path.h
// ---------------------------------------------------------------------------

/// `git_fs_path_is_root`.
#[inline]
pub fn is_root(name: &[u8]) -> bool {
    at(name, 0) == b'/' && at(name, 1) == 0
}

/// `git_fs_path_is_dot_or_dotdot`.
#[inline]
pub fn is_dot_or_dotdot(name: &[u8]) -> bool {
    at(name, 0) == b'.' && (at(name, 1) == 0 || (at(name, 1) == b'.' && at(name, 2) == 0))
}

/// `git_fs_path_is_absolute` (Unix).
#[inline]
pub fn is_absolute(p: &[u8]) -> bool {
    at(p, 0) == b'/'
}

/// `git_fs_path_is_dirsep` (Unix).
#[inline]
pub fn is_dirsep(c: u8) -> bool {
    c == b'/'
}

/// `git_fs_path_is_relative`.
#[inline]
pub fn is_relative(p: &[u8]) -> bool {
    at(p, 0) == b'.' && (at(p, 1) == b'/' || (at(p, 1) == b'.' && at(p, 2) == b'/'))
}

/// `git_fs_path_at_end_of_segment`.
#[inline]
pub fn at_end_of_segment(p: &[u8]) -> bool {
    let c = at(p, 0);
    c == 0 || c == b'/'
}

/// `git_fs_path_equal_or_prefixed`: whether `child` is `parent` or lies
/// beneath it, and the length of the common prefix (without a trailing
/// slash of `parent`) when it does.
#[inline]
pub fn equal_or_prefixed(parent: &[u8], child: &[u8]) -> (c_int, Option<isize>) {
    let (mut p, mut c) = (0usize, 0usize);
    let mut lastslash = 0isize;
    while at(parent, p) != 0 && at(child, c) != 0 {
        lastslash = (parent[p] == b'/') as isize;
        if parent[p] != child[c] {
            return (GIT_FS_PATH_NOTEQUAL, None);
        }
        p += 1;
        c += 1;
    }
    if at(parent, p) != 0 {
        return (GIT_FS_PATH_NOTEQUAL, None);
    }
    if at(child, c) == 0 {
        return (GIT_FS_PATH_EQUAL, Some(p as isize));
    }
    if child[c] == b'/' || lastslash != 0 {
        return (GIT_FS_PATH_PREFIX, Some(p as isize - lastslash));
    }
    (GIT_FS_PATH_NOTEQUAL, None)
}

// ---------------------------------------------------------------------------
// basename, dirname, root
// ---------------------------------------------------------------------------

/// A piece of a path that `basename` / `dirname` computes: a static string
/// or a range of the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// `"."` or `"/"`.
    Static(&'static [u8]),
    /// `path[start..start + len]`.
    Range(usize, usize),
}

impl Piece {
    /// The bytes of the piece of `path`.
    pub fn bytes<'a>(&self, path: &'a [u8]) -> &'a [u8] {
        match *self {
            Piece::Static(s) => s,
            Piece::Range(start, len) => &path[start..start + len],
        }
    }

    fn len(&self) -> usize {
        match *self {
            Piece::Static(s) => s.len(),
            Piece::Range(_, len) => len,
        }
    }
}

/// `git_str_set(buf, data)` with the exports' fast path (one copy when
/// `data` fits and lies outside the buffer).
#[inline(always)]
fn set_bytes(buf: &mut GitStr, data: &[u8]) -> Result<()> {
    // SAFETY: `data` is readable for its length; the fast path checks that
    // it lies outside the allocation.
    if unsafe { buf.try_set_fast(data.as_ptr(), data.len()) } {
        return Ok(());
    }
    buf.set(data)
}

/// The basename of the C string `path` (`None`: C's NULL), as
/// `git_fs_path_basename_r` finds it.
#[inline]
pub fn basename_piece(path: Option<&[u8]>) -> Piece {
    let path = path.unwrap_or(b"");
    let Some(&first) = path.first() else {
        return Piece::Static(b".");
    };
    // The last non-slash byte, keeping the first byte (C's `endp > path`).
    let endp = trailing_slashes_end(path);
    if endp == 0 && first == b'/' {
        return Piece::Static(b"/");
    }
    let startp = memchr::memrchr(b'/', &path[..endp]).map_or(0, |i| i + 1);
    Piece::Range(startp, endp - startp + 1)
}

/// The index of the last byte of `path` (not empty) that is not a
/// trailing slash, or 0: C's `while (endp > path && *endp == '/') endp--`.
#[inline(always)]
fn trailing_slashes_end(path: &[u8]) -> usize {
    match path[1..].iter().rposition(|&c| c != b'/') {
        Some(i) => i + 1,
        None => 0,
    }
}

/// `git_fs_path_basename_r`: the basename's length, and the basename in
/// `buffer` if given.
#[inline]
pub fn basename_r(buffer: Option<&mut GitStr>, path: Option<&[u8]>) -> Result<c_int> {
    let p = path.unwrap_or(b"");
    let piece = basename_piece(path);
    let len = piece.len() as c_int;
    if let Some(b) = buffer {
        set_bytes(b, piece.bytes(p))?;
    }
    Ok(len)
}

/// The dirname of the C string `path` (`None`: C's NULL), as
/// `git_fs_path_dirname_r` finds it: a prefix of `path`, or a static
/// string.
#[inline]
pub fn dirname_piece(path: Option<&[u8]>) -> Result<Piece> {
    let path = path.unwrap_or(b"");
    let Some(&first) = path.first() else {
        return Ok(Piece::Static(b"."));
    };
    let endp = trailing_slashes_end(path);
    if endp + 1 > c_int::MAX as usize {
        return Err(git_error_set!(ErrorClass::INVALID, "path too long"));
    }
    // The last slash before `endp`, not the first byte (C's `endp > path`).
    let slash = match memchr::memrchr(b'/', &path[1..=endp]) {
        Some(i) if endp > 0 => i + 1,
        _ => 0,
    };
    if slash == 0 {
        return Ok(Piece::Static(if first == b'/' { b"/" } else { b"." }));
    }
    // Back over the run of slashes, keeping the first byte.
    let endp = trailing_slashes_end(&path[..slash]);
    if endp + 1 > c_int::MAX as usize {
        return Err(git_error_set!(ErrorClass::INVALID, "path too long"));
    }
    Ok(Piece::Range(0, endp + 1))
}

/// `git_fs_path_dirname_r`: the dirname's length, and the dirname in
/// `buffer` if given.
#[inline]
pub fn dirname_r(buffer: Option<&mut GitStr>, path: Option<&[u8]>) -> Result<c_int> {
    let p = path.unwrap_or(b"");
    let piece = dirname_piece(path)?;
    let len = piece.len() as c_int;
    if let Some(b) = buffer {
        set_bytes(b, piece.bytes(p))?;
    }
    Ok(len)
}

/// `git_fs_path_dirname_r(buf, buf->ptr)`: replace `buf` with its dirname.
pub fn dirname_self(buf: &mut GitStr) -> Result<c_int> {
    let piece = dirname_piece(Some(str_c_bytes(buf)))?;
    match piece {
        Piece::Static(s) => buf.set(s)?,
        // C's `git_str_set(buf, buf->ptr, len)`: the bytes are in place.
        Piece::Range(_, len) => buf.truncate(len),
    }
    Ok(piece.len() as c_int)
}

/// `git_fs_path_dirname`: a new string (NULL on failure).
pub fn dirname(path: Option<&[u8]>) -> Option<GitCStr> {
    let mut buf = GitStr::new();
    let _ = dirname_r(Some(&mut buf), path);
    buf.detach()
}

/// `git_fs_path_basename`: a new string (NULL on failure).
pub fn basename(path: Option<&[u8]>) -> Option<GitCStr> {
    let mut buf = GitStr::new();
    let _ = basename_r(Some(&mut buf), path);
    buf.detach()
}

/// `git_fs_path_basename_offset`.
#[inline]
pub fn basename_offset(buffer: Option<&GitStr>) -> usize {
    let Some(b) = buffer else { return 0 };
    if b.is_empty() {
        return 0;
    }
    match b.rfind_next(b'/') {
        Some(slash) if b.as_bytes()[slash] == b'/' => slash + 1,
        _ => 0,
    }
}

/// `dos_drive_prefix_length` over a byte reader (`at(i)` is the C
/// string's byte `i`; it is asked only for bytes up to and including the
/// first NUL).
#[inline(always)]
fn dos_drive_prefix_length(at: impl Fn(usize) -> u8) -> usize {
    let c0 = at(0);
    if c0 & 0x80 == 0 {
        return if c0 != 0 && at(1) == b':' { 2 } else { 0 };
    }
    let mut i = 1;
    while i < 4 && at(i) & 0x80 != 0 {
        i += 1;
    }
    if at(i) == b':' { i + 1 } else { 0 }
}

/// [`root`] over a byte reader (as [`dos_drive_prefix_length`]'s).
#[inline(always)]
pub(crate) fn root_with(at: impl Fn(usize) -> u8) -> c_int {
    let offset = dos_drive_prefix_length(&at);
    if at(offset) == b'/' {
        offset as c_int
    } else {
        -1
    }
}

/// `git_fs_path_root`: the offset of the root slash, or -1 when `path` is
/// not rooted. (A DOS drive prefix such as `C:` is recognized on every
/// platform, as in C.) It reads at most five bytes and never past a NUL, so
/// `path` may extend past its C string.
#[inline]
pub fn root(path: &[u8]) -> c_int {
    root_with(|i| self::at(path, i))
}

#[inline]
fn trim_slashes(path: &mut GitStr) {
    let ceiling = root(path.as_bytes()) + 1;
    if ceiling < 0 {
        return;
    }
    let ceiling = ceiling as usize;
    while path.len() > ceiling && path.as_bytes()[path.len() - 1] == b'/' {
        let n = path.len() - 1;
        path.truncate(n);
    }
}

/// `git_fs_path_to_dir`: append a slash unless there is one (an empty or
/// unallocated string is left alone).
#[inline]
pub fn to_dir(path: &mut GitStr) -> Result<()> {
    let len = path.len();
    if path.capacity() > 0 && len > 0 && path.as_bytes().last().is_some_and(|&c| c != b'/') {
        // `git_str_putc`, in place when the slash and the NUL fit.
        // SAFETY: the NUL is written back right away (`spare[1]`).
        let spare = unsafe { path.spare_with_nul_mut() };
        if spare.len() >= 2 {
            spare[0].write(b'/');
            spare[1].write(0);
            // SAFETY: owned (it has spare room); the first `len` bytes are
            // the contents and byte `len` was just written, followed by the
            // NUL.
            unsafe { path.set_len(len + 1) };
        } else {
            let _ = path.putc(b'/');
        }
    }
    if path.is_oom() {
        Err(ErrorCode::GENERIC)
    } else {
        Ok(())
    }
}

/// `git_fs_path_string_to_dir`: append a slash to the C string in `path`
/// when its length is below `size` (C writes the slash and the NUL up to
/// `path[size]`: callers pass one less than the buffer's size). Nothing is
/// written past `path`'s end.
pub fn string_to_dir(path: &mut [u8], size: usize) {
    let Some(end) = memchr::memchr(0, path) else {
        return;
    };
    if end > 0 && path[end - 1] != b'/' && end < size && end + 1 < path.len() {
        path[end] = b'/';
        path[end + 1] = 0;
    }
}

/// `git_fs_path_dirlen`: the length without trailing slashes (keeping
/// one character).
#[inline]
pub fn dirlen(path: &[u8]) -> usize {
    let mut len = path.len();
    while len > 1 && path[len - 1] == b'/' {
        len -= 1;
    }
    len
}

/// `git_fs_path_join_unrooted`: join `path` to `base` unless it is rooted;
/// the offset of the root in the result.
#[inline]
pub fn join_unrooted(path_out: &mut GitStr, path: &[u8], base: Option<&[u8]>) -> Result<isize> {
    let mut r = root(path) as isize;
    match base {
        Some(base) if r < 0 => {
            joinpath_into(path_out, base, path)?;
            r = base.len() as isize;
        }
        _ => {
            set_bytes(path_out, path)?;
            if r < 0 {
                r = 0;
            } else if let Some(base) = base
                && let (_, Some(n)) = equal_or_prefixed(base, path)
            {
                r = n;
            }
        }
    }
    Ok(r)
}

/// `git_str_joinpath(out, a, b)` for `a` and `b` outside `out`: the same
/// result and growth, written in one pass.
#[inline]
fn joinpath_into(out: &mut GitStr, a: &[u8], b: &[u8]) -> Result<()> {
    if out.is_oom() || (out.capacity() == 0 && !out.is_empty()) {
        // Out of memory, or borrowed: the general path fails as C does.
        return out.joinpath(a, b);
    }
    let mut b = b;
    let mut need_sep = 0;
    if let Some(&last) = a.last() {
        while let [b'/', rest @ ..] = b {
            b = rest;
        }
        need_sep = (last != b'/') as usize;
    }
    let total = alloc_add(alloc_add(a.len(), b.len())?, need_sep)?;
    let alloc_len = alloc_add(total, 1)?;
    if alloc_len > out.capacity() {
        out.grow(alloc_len)?;
    }
    out.clear();
    // SAFETY: the NUL is written at `total` before the string is used.
    let spare = unsafe { out.spare_with_nul_mut() };
    spare[..a.len()].write_copy_of_slice(a);
    if need_sep == 1 {
        spare[a.len()].write(b'/');
    }
    spare[a.len() + need_sep..total].write_copy_of_slice(b);
    spare[total].write(0);
    // SAFETY: owned (grown to `alloc_len > total`); its first `total` bytes
    // were just written, followed by the NUL.
    unsafe { out.set_len(total) };
    Ok(())
}

/// `git_fs_path_squash_slashes`: collapse runs of slashes.
#[inline]
pub fn squash_slashes(path: &mut GitStr) {
    if path.is_empty() {
        return;
    }
    let size = path.len();
    let b = path.as_mut_bytes();
    if b.is_empty() {
        // A borrowed string is not written.
        return;
    }
    let (mut p, mut q, mut squashed) = (0usize, 0usize, 0usize);
    while q < b.len() && b[q] != 0 {
        b[p] = b[q];
        while b[q] == b'/' && at(b, q + 1) == b'/' {
            squashed += 1;
            q += 1;
        }
        p += 1;
        q += 1;
    }
    if p < b.len() {
        b[p] = 0;
    }
    path.truncate(size - squashed);
}

/// `git_fs_path_prettify`: the canonical absolute path of `path` (joined to
/// `base` first if it is relative).
pub fn prettify(path_out: &mut GitStr, path: &CStr, base: Option<&[u8]>) -> Result<()> {
    let mut buf = [0u8; GIT_PATH_MAX];
    let joined;
    let path: &CStr = match base {
        Some(base) if root(path.to_bytes()) < 0 => {
            path_out.joinpath(c_str(base), path.to_bytes())?;
            joined = str_cpath(path_out).into_owned();
            &joined
        }
        _ => path,
    };
    match posix::realpath_buf(path, &mut buf) {
        Ok(n) => path_out.sets(&buf[..n]),
        Err(posix::Errno(e)) => {
            let code = if e == libc::ENOENT || e == libc::ENOTDIR {
                ErrorCode::NOT_FOUND
            } else {
                ErrorCode::GENERIC
            };
            let _ = git_error_set!(
                ErrorClass::OS,
                "failed to resolve path '{}'",
                Bytes(path.to_bytes())
            );
            path_out.clear();
            Err(code)
        }
    }
}

/// `git_fs_path_prettify_dir`: [`prettify`] with a trailing slash.
pub fn prettify_dir(path_out: &mut GitStr, path: &CStr, base: Option<&[u8]>) -> Result<()> {
    prettify(path_out, path, base)?;
    to_dir(path_out)
}

/// `git_fs_path_find_dir`: the canonical directory of `dir` (its dirname),
/// with a trailing slash.
pub fn find_dir(dir: &mut GitStr) -> Result<()> {
    let mut buf = [0u8; GIT_PATH_MAX];
    let real = posix::realpath_buf(&str_cpath(dir), &mut buf);
    if let Ok(n) = real {
        dir.sets(&buf[..n])?;
    }
    dirname_self(dir)?;
    to_dir(dir)
}

// ---------------------------------------------------------------------------
// Relative paths
// ---------------------------------------------------------------------------

/// Make a borrowed string owned, so that it can be written in place.
#[inline]
fn make_owned(path: &mut GitStr) -> Result<()> {
    if path.capacity() == 0 && !path.is_empty() {
        let copy = GitStr::from_bytes(path.as_bytes())?;
        *path = copy;
    }
    Ok(())
}

/// `git_fs_path_resolve_relative`: resolve `.` and `..` components in
/// place, not backing up over the first `ceiling` bytes (0: the root, or a
/// URL's `scheme://`).
#[inline]
pub fn resolve_relative(path: &mut GitStr, ceiling: usize) -> Result<()> {
    if path.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    make_owned(path)?;
    let size = path.len();
    let mut ceiling = ceiling.min(size);
    if ceiling == 0 {
        ceiling = (root(path.as_bytes()) + 1) as usize;
    }
    let b = path.as_mut_bytes();
    if ceiling == 0 {
        let mut next = 0;
        while at(b, next) != 0 && at(b, next).is_ascii_alphabetic() {
            next += 1;
        }
        if at(b, next) == b':' && at(b, next + 1) == b'/' && at(b, next + 2) == b'/' {
            ceiling = next + 3;
        }
    }
    // C walks from the ceiling to the next NUL (the end of the contents,
    // unless a NUL is embedded); `at(b, end)` is that 0.
    let end = ceiling + memchr::memchr(0, &b[ceiling..]).unwrap_or(b.len() - ceiling);
    let b = &mut b[..end];
    let (mut base, mut to, mut from) = (ceiling, ceiling, ceiling);
    while from < end {
        let next = from
            + b[from..]
                .iter()
                .position(|&c| c == b'/')
                .unwrap_or(end - from);
        let at_slash = next < end;
        let mut len = next - from;
        if len == 1 && b[from] == b'.' {
            // a singleton dot: nothing
        } else if len == 2 && b[from] == b'.' && b[from + 1] == b'.' {
            if to == base && ceiling != 0 {
                return Err(git_error_set!(
                    ErrorClass::INVALID,
                    "cannot strip root component off url"
                ));
            }
            if to == base {
                if at_slash {
                    len += 1;
                }
                if to != from {
                    b.copy_within(from..from + len, to);
                }
                to += len;
                base = to;
            } else {
                while to > base && b[to - 1] == b'/' {
                    to -= 1;
                }
                while to > base && b[to - 1] != b'/' {
                    to -= 1;
                }
            }
        } else {
            if at_slash && b[from] != b'/' {
                len += 1;
            }
            if to != from {
                b.copy_within(from..from + len, to);
            }
            to += len;
        }
        from += len;
        while from < end && b[from] == b'/' {
            from += 1;
        }
    }
    path.truncate(to);
    Ok(())
}

/// `git_fs_path_apply_relative`: join `relpath` to `target` and resolve.
///
/// C returns `joinpath(…) || resolve_relative(…)`, i.e. 1 (not a negative
/// code) on failure; the export does the same.
pub fn apply_relative(target: &mut GitStr, relpath: &[u8]) -> Result<()> {
    let joined = match target.as_c_str() {
        Some(_) => {
            let n = str_c_bytes(target).len();
            target.truncate(n);
            target.joinpath_self(c_str(relpath))
        }
        None => {
            let a = str_c_bytes(target).to_vec();
            target.joinpath(&a, c_str(relpath))
        }
    };
    joined?;
    resolve_relative(target, 0)
}

/// `git_fs_path_cmp`: compare two names of `len1` and `len2` bytes with
/// `compare` on their common length, then by the next byte, a directory's
/// end counting as `/`. A name's byte at its length is read as 0 when the
/// slice ends there.
pub fn cmp(
    name1: &[u8],
    len1: usize,
    isdir1: bool,
    name2: &[u8],
    len2: usize,
    isdir2: bool,
    compare: impl FnOnce(&[u8], &[u8], usize) -> c_int,
) -> c_int {
    let len = len1.min(len2);
    let r = compare(name1, name2, len);
    if r != 0 {
        return r;
    }
    let mut c1 = at(name1, len);
    let mut c2 = at(name2, len);
    if c1 == 0 && isdir1 {
        c1 = b'/';
    }
    if c2 == 0 && isdir2 {
        c2 = b'/';
    }
    (c1 > c2) as c_int - (c1 < c2) as c_int
}

/// `git_fs_path_common_dirlen`: the length of the common directory
/// prefix, its slash included.
pub fn common_dirlen(one: &[u8], two: &[u8]) -> usize {
    common_dirlen_with(one, two)
}

/// [`common_dirlen`] over [`CBytes`].
#[inline]
pub fn common_dirlen_with(one: impl CBytes, two: impl CBytes) -> usize {
    // One past the last common slash, 0 for none.
    let mut dirsep = 0;
    let mut i = 0;
    loop {
        let p = one.at(i);
        if p == 0 {
            break;
        }
        let q = two.at(i);
        if q == 0 {
            break;
        }
        if p == b'/' && q == b'/' {
            dirsep = i + 1;
        } else if p != q {
            break;
        }
        i += 1;
    }
    dirsep
}

/// `git_fs_path_make_relative`: make `path` relative to `parent`.
#[inline]
pub fn make_relative(path: &mut GitStr, parent: &[u8]) -> Result<()> {
    let parent = c_str(parent);
    let size = path.len();
    let pb = str_c_bytes(path);
    let qb = parent;
    let (mut p, mut q, mut p_dirsep, mut q_dirsep) = (0usize, 0usize, 0usize, 0usize);
    while at(pb, p) != 0 && at(qb, q) != 0 {
        if pb[p] == b'/' && qb[q] == b'/' {
            p_dirsep = p;
            q_dirsep = q;
        } else if pb[p] != qb[q] {
            break;
        }
        p += 1;
        q += 1;
    }
    if (p_dirsep == 0 || q_dirsep == 0) && (at(pb, p_dirsep) != b'/' || at(qb, q_dirsep) != b'/') {
        let _ = git_error_set!(
            ErrorClass::INVALID,
            "{} is not a parent of {}",
            Bytes(parent),
            Bytes(pb)
        );
        return Err(ErrorCode::NOT_FOUND);
    }
    let (cp, cq) = (at(pb, p), at(qb, q));
    if cp == b'/' && cq == 0 {
        p += 1;
    } else if cp == 0 && cq == b'/' {
        q += 1;
    } else if cp == 0 && cq == 0 {
        path.clear();
        return Ok(());
    } else {
        p = p_dirsep + 1;
        q = q_dirsep + 1;
    }
    let plen = size - p;
    if at(qb, q) == 0 {
        // C's `git_str_set(path, p, plen)`, from within the buffer.
        let rest = path.as_bytes()[p..size].to_vec();
        return path.set(&rest);
    }
    let mut depth = 1usize;
    let mut i = q;
    while let Some(off) = memchr::memchr(b'/', &qb[i..]) {
        let slash = i + off;
        if at(qb, slash + 1) == 0 {
            break;
        }
        depth += 1;
        i = slash + 1;
    }
    let newlen = depth
        .checked_mul(3)
        .and_then(|n| n.checked_add(plen))
        .ok_or_else(errors::oom)?;
    let alloclen = alloc_add(newlen, 1)?;
    let mut v = Vec::new();
    v.try_reserve_exact(newlen).map_err(|_| errors::oom())?;
    for _ in 0..depth {
        v.extend_from_slice(b"../");
    }
    v.extend_from_slice(&path.as_bytes()[p..size]);
    path.try_grow(alloclen, true)?;
    path.set(&v)
}

/// `git_fs_path_has_non_ascii`.
#[inline]
pub fn has_non_ascii(path: &[u8]) -> bool {
    path.iter().any(|&c| c & 0x80 != 0)
}

/// `git_fs_path_does_decompose_unicode`: `false` (no iconv on this
/// platform's configuration).
pub fn does_decompose_unicode(_root: &[u8]) -> bool {
    false
}

// ---------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------

fn fromhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `git__percent_decode`: decode `%XX` escapes of `input` into
/// `decoded_out` (replacing its contents).
pub fn percent_decode(decoded_out: &mut GitStr, input: &[u8]) -> Result<()> {
    let input = c_str(input);
    let len = input.len() as isize;
    decoded_out.clear();
    let mut i = 0isize;
    while i < len {
        let mut c = input[i as usize];
        if c == b'%'
            && i < len - 2
            && let (Some(hi), Some(lo)) = (
                fromhex(input[i as usize + 1]),
                fromhex(input[i as usize + 2]),
            )
        {
            c = (hi << 4) | lo;
            i += 2;
        }
        decoded_out.putc(c)?;
        i += 1;
    }
    Ok(())
}

fn local_file_url_prefixlen(file_url: &[u8]) -> Option<usize> {
    if file_url.starts_with(b"file://") {
        if at(file_url, 7) == b'/' {
            return Some(8);
        } else if file_url[7..].starts_with(b"localhost/") {
            return Some(17);
        }
    }
    None
}

/// `git_fs_path_is_local_file_url`.
pub fn is_local_file_url(file_url: &[u8]) -> bool {
    local_file_url_prefixlen(c_str(file_url)).is_some()
}

/// `git_fs_path_fromurl`: the local path of a `file://` URL.
pub fn fromurl(local_path_out: &mut GitStr, file_url: &[u8]) -> Result<()> {
    let url = c_str(file_url);
    let offset = match local_file_url_prefixlen(url) {
        Some(o) if at(url, o) != 0 && at(url, o) != b'/' => o,
        _ => {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "'{}' is not a valid local file URI",
                Bytes(url)
            ));
        }
    };
    // A *nix absolute path starts with a forward slash.
    let offset = offset - 1;
    local_path_out.clear();
    percent_decode(local_path_out, &url[offset..])
}

/// `git_fs_path_from_url_or_path`.
pub fn from_url_or_path(local_path_out: &mut GitStr, url_or_path: &[u8]) -> Result<()> {
    if is_local_file_url(url_or_path) {
        fromurl(local_path_out, url_or_path)
    } else {
        local_path_out.sets(c_str(url_or_path))
    }
}

// ---------------------------------------------------------------------------
// Walking up, and directory iteration
// ---------------------------------------------------------------------------

/// What [`walk_up`] hands its callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkStep {
    /// The path's first `n` bytes (a directory, with its trailing slash
    /// except for the path itself).
    Prefix(usize),
    /// The empty path (`""`), yielded last for a relative path.
    Empty,
}

/// `git_fs_path_walk_up` over the bytes of `path` (its `size` bytes;
/// `cpath` is what C reads as `path->ptr`, up to the first NUL): call `cb`
/// for the path and each parent directory down to `ceiling`, stopping at
/// the first non-zero return, which is returned.
///
/// C's ensure-an-error-is-set step after a failing callback has no effect
/// since `git_error_last` never returns NULL; it is not reproduced.
pub fn walk_up(
    path: &[u8],
    ceiling: Option<&[u8]>,
    mut cb: impl FnMut(WalkStep) -> c_int,
) -> c_int {
    let cpath = c_str(path);
    let mut stop = 0usize;
    if let Some(ceiling) = ceiling {
        let ceiling = c_str(ceiling);
        stop = if cpath.starts_with(ceiling) {
            ceiling.len()
        } else {
            path.len()
        };
    }
    let mut scan = path.len() as isize;
    if scan == 0 {
        return cb(WalkStep::Empty);
    }
    let mut error = 0;
    let mut iter_size = path.len();
    while scan >= stop as isize {
        error = cb(WalkStep::Prefix(scan as usize));
        if error != 0 {
            break;
        }
        // git_str_rfind_next(&iter, '/')
        let b = &path[..iter_size];
        let mut idx = b.len();
        while idx > 0 && b[idx - 1] == b'/' {
            idx -= 1;
        }
        while idx > 0 && b[idx - 1] != b'/' {
            idx -= 1;
        }
        scan = idx as isize - 1;
        if scan >= 0 {
            scan += 1;
            iter_size = scan as usize;
        }
    }
    if error == 0 && stop == 0 && at(path, 0) != b'/' {
        error = cb(WalkStep::Empty);
    }
    error
}

/// Access to the `git_str` that [`direach`] extends with each entry: the
/// string itself for Rust callers, or a raw pointer for the export, whose C
/// callback receives the same pointer.
pub trait PathCell {
    /// The string, for an access that ends before the callback runs.
    fn path(&mut self) -> &mut GitStr;
}

impl PathCell for GitStr {
    #[inline]
    fn path(&mut self) -> &mut GitStr {
        self
    }
}

/// A directory stream that is closed on drop.
struct Dir(core::ptr::NonNull<libc::DIR>);

impl Dir {
    fn open(path: &CStr) -> Option<Dir> {
        // SAFETY: `path` is NUL-terminated.
        let d = unsafe { libc::opendir(path.as_ptr()) };
        core::ptr::NonNull::new(d).map(Dir)
    }

    /// The next entry's name (copied into `name`), `Ok(false)` at the end,
    /// or the `errno` of a failure.
    fn read(&mut self, name: &mut Vec<u8>) -> posix::SysResult<bool> {
        errors::set_errno(0);
        // SAFETY: the stream is open and only this `Dir` uses it.
        let de = unsafe { libc::readdir(self.0.as_ptr()) };
        if de.is_null() {
            let e = errors::errno();
            return if e == 0 {
                Ok(false)
            } else {
                Err(posix::Errno(e))
            };
        }
        // SAFETY: `readdir` returned an entry, valid until the next call on
        // this stream: a place projection, no read.
        let d_name = unsafe { &raw const (*de).d_name };
        // SAFETY: as above; `d_name` is NUL-terminated.
        let n = unsafe { CStr::from_ptr(d_name.cast::<c_char>()) };
        name.clear();
        name.extend_from_slice(n.to_bytes());
        Ok(true)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        // SAFETY: the stream is open; it is closed once.
        unsafe { libc::closedir(self.0.as_ptr()) };
    }
}

/// `git_fs_path_direach`: call `cb` with `path` extended by each entry of
/// the directory `path` (but `.` and `..`), restoring it after each call;
/// a non-zero return stops the walk and is returned as the error.
///
/// C sets an error only for a failure of its own; as C, the error is
/// cleared before each callback.
pub fn direach<P: PathCell + ?Sized>(
    path: &mut P,
    _flags: u32,
    cb: &mut dyn FnMut(&mut P) -> c_int,
) -> Result<()> {
    to_dir(path.path())?;
    let wd_len = path.path().len();
    let Some(mut dir) = Dir::open(&str_cpath(path.path())) else {
        let _ = git_error_set!(
            ErrorClass::OS,
            "failed to open directory '{}'",
            Bytes(str_c_bytes(path.path()))
        );
        // C tests `errno == ENOENT` here, after `git_error_set` reset it to
        // 0: never true. Kept.
        if errors::errno() == libc::ENOENT {
            return Err(ErrorCode::NOT_FOUND);
        }
        return Err(ErrorCode::GENERIC);
    };
    let mut name = Vec::new();
    // C stops at `readdir`'s NULL whatever `errno` says.
    while let Ok(true) = dir.read(&mut name) {
        if is_dot_or_dotdot(&name) {
            continue;
        }
        path.path().put(&name)?;
        errors::clear();
        let error = cb(path);
        path.path().truncate(wd_len);
        if let Some(e) = ErrorCode::from_raw(error) {
            return Err(e);
        }
    }
    Ok(())
}

/// `git_fs_path_diriter`, mirrored (kind 3).
///
/// Its invariant: `dir` is NULL or a stream this iterator opened and alone
/// uses; `path` holds the directory (`parent_len` bytes) and, after
/// [`next`](Self::next), the entry's name.
#[repr(C)]
#[derive(Debug)]
pub struct GitFsPathDiriter {
    path: GitStr,
    parent_len: usize,
    flags: c_uint,
    dir: *mut libc::DIR,
}

// SAFETY: the directory stream belongs to this iterator alone (its
// invariant), and may be used from any one thread at a time.
unsafe impl Send for GitFsPathDiriter {}

impl Default for GitFsPathDiriter {
    fn default() -> Self {
        Self::new()
    }
}

impl GitFsPathDiriter {
    /// `GIT_FS_PATH_DIRITER_INIT`.
    pub const fn new() -> Self {
        GitFsPathDiriter {
            path: GitStr::new(),
            parent_len: 0,
            flags: 0,
            dir: core::ptr::null_mut(),
        }
    }

    /// `git_fs_path_diriter_init`: open the directory `path`, releasing
    /// what the iterator held (the export overwrites a C iterator without
    /// reading it first, as C's `memset`).
    pub fn init(&mut self, path: &[u8], flags: c_uint) -> Result<()> {
        let path = c_str(path);
        self.free();
        self.parent_len = 0;
        self.flags = 0;
        self.path.puts(path)?;
        trim_slashes(&mut self.path);
        if self.path.is_empty() {
            return Err(git_error_set!(
                ErrorClass::FILESYSTEM,
                "could not open directory '{}'",
                Bytes(path)
            ));
        }
        let Some(dir) = Dir::open(&str_cpath(&self.path)) else {
            self.path.dispose();
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to open directory '{}'",
                Bytes(path)
            ));
        };
        self.dir = dir.0.as_ptr();
        core::mem::forget(dir);
        self.parent_len = self.path.len();
        self.flags = flags;
        Ok(())
    }

    /// `git_fs_path_diriter_next`: advance to the next entry;
    /// `Err(GIT_ITEROVER)` at the end.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<()> {
        git_assert!(!self.dir.is_null(), "diriter->dir");
        let skip_dot = self.flags & GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT == 0;
        let mut name = Vec::new();
        // The stream is borrowed for the reads, not owned.
        let mut dir = core::mem::ManuallyDrop::new(Dir(
            core::ptr::NonNull::new(self.dir).expect("checked above")
        ));
        loop {
            match dir.read(&mut name) {
                Ok(true) if skip_dot && is_dot_or_dotdot(&name) => continue,
                Ok(true) => break,
                Ok(false) => return Err(ErrorCode::ITER_OVER),
                Err(_) => {
                    return Err(git_error_set!(
                        ErrorClass::OS,
                        "could not read directory '{}'",
                        Bytes(str_c_bytes(&self.path))
                    ));
                }
            }
        }
        self.path.truncate(self.parent_len);
        if self.parent_len > 0 && self.path.as_bytes().get(self.parent_len - 1) != Some(&b'/') {
            let _ = self.path.putc(b'/');
        }
        let _ = self.path.put(&name);
        if self.path.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `git_fs_path_diriter_filename`: the current entry's name (C's
    /// offset: one past the parent's length, even when the parent ends with
    /// a slash).
    pub fn filename(&self) -> Result<&[u8]> {
        git_assert!(
            self.path.len() > self.parent_len,
            "diriter->path.size > diriter->parent_len"
        );
        Ok(&self.path.as_bytes()[(self.parent_len + 1).min(self.path.len())..])
    }

    /// `git_fs_path_diriter_fullpath`: the current entry's path.
    pub fn fullpath(&self) -> &[u8] {
        self.path.as_bytes()
    }

    /// `git_fs_path_diriter_stat`: `lstat` of the current entry.
    pub fn stat(&self) -> Result<Stat> {
        lstat(&str_cpath(&self.path))
    }

    /// `git_fs_path_diriter_free`: close the directory and free the path.
    pub fn free(&mut self) {
        if !self.dir.is_null() {
            // The stream this iterator opened (its invariant), closed once.
            drop(Dir(core::ptr::NonNull::new(self.dir).expect("not null")));
            self.dir = core::ptr::null_mut();
        }
        self.path.dispose();
    }
}

impl Drop for GitFsPathDiriter {
    fn drop(&mut self) {
        self.free();
    }
}

/// `git_fs_path_dirload`: append to `contents` a copy (from
/// `git__allocator`, which the caller frees) of the path of each entry of
/// `path`, without its first `prefix_len` bytes.
pub fn dirload(contents: &mut GitVector, path: &[u8], prefix_len: usize, flags: u32) -> Result<()> {
    let mut iter = GitFsPathDiriter::new();
    iter.init(path, flags)?;
    let r = loop {
        match iter.next() {
            Ok(()) => {}
            Err(ErrorCode::ITER_OVER) => break Ok(()),
            Err(e) => break Err(e),
        }
        let name = iter.fullpath();
        if name.len() <= prefix_len {
            break Err(errors::assert_failed("name_len > prefix_len"));
        }
        let dup = alloc::strndup(&name[prefix_len..], name.len() - prefix_len);
        if dup.is_null() {
            break Err(ErrorCode::GENERIC);
        }
        if let Err(e) = contents.insert(dup.cast()) {
            // SAFETY: `dup` is the fresh allocation above, not stored.
            unsafe { alloc::free(dup.cast()) };
            break Err(e);
        }
    };
    iter.free();
    r
}

// ---------------------------------------------------------------------------
// The filesystem
// ---------------------------------------------------------------------------

/// `git_fs_path_exists`.
pub fn exists(path: &CStr) -> bool {
    posix::access(path, libc::F_OK).is_ok()
}

/// `git_fs_path_isdir`.
pub fn isdir(path: &CStr) -> bool {
    posix::stat(path).is_ok_and(|st| posix::s_isdir(st.st_mode))
}

/// `git_fs_path_isfile`.
pub fn isfile(path: &CStr) -> bool {
    posix::stat(path).is_ok_and(|st| posix::s_isreg(st.st_mode))
}

/// `git_fs_path_isexecutable`.
pub fn isexecutable(path: &CStr) -> bool {
    posix::stat(path).is_ok_and(|st| posix::s_isreg(st.st_mode) && st.st_mode & libc::S_IXUSR != 0)
}

/// `git_fs_path_islink`.
pub fn islink(path: &CStr) -> bool {
    posix::lstat(path).is_ok_and(|st| posix::s_islnk(st.st_mode))
}

/// `git_fs_path_is_empty_dir`.
pub fn is_empty_dir(path: &CStr) -> bool {
    if !isdir(path) {
        return false;
    }
    let mut dir = GitStr::new();
    if dir.sets(path.to_bytes()).is_err() {
        errors::clear();
        return false;
    }
    // As C, the callback tests the entry's whole path, which is never `.`
    // or `..`: the first entry stops the walk.
    direach(&mut dir, 0, &mut |p: &mut GitStr| {
        !is_dot_or_dotdot(p.as_bytes()) as c_int
    })
    .is_ok()
}

/// `git_fs_path_set_error`: the error for a failed operation `action` on
/// `path` with `errno_value` (the message ends with `errno`'s, as every
/// `GIT_ERROR_OS` one).
pub fn set_error(errno_value: c_int, path: &[u8], action: &str) -> ErrorCode {
    let path = Bytes(c_str(path));
    match errno_value {
        libc::ENOENT | libc::ENOTDIR => {
            let _ = git_error_set!(ErrorClass::OS, "could not find '{path}' to {action}");
            ErrorCode::NOT_FOUND
        }
        libc::EINVAL | libc::ENAMETOOLONG => {
            let _ = git_error_set!(ErrorClass::OS, "invalid path for filesystem '{path}'");
            ErrorCode::INVALID_SPEC
        }
        libc::EEXIST => {
            let _ = git_error_set!(ErrorClass::OS, "failed {action} - '{path}' already exists");
            ErrorCode::EXISTS
        }
        libc::EACCES => {
            let _ = git_error_set!(ErrorClass::OS, "failed {action} - '{path}' is locked");
            ErrorCode::LOCKED
        }
        _ => git_error_set!(ErrorClass::OS, "could not {action} '{path}'"),
    }
}

/// `git_fs_path_lstat`.
pub fn lstat(path: &CStr) -> Result<Stat> {
    posix::lstat(path).map_err(|posix::Errno(e)| set_error(e, path.to_bytes(), "stat"))
}

fn check_dir_contents(dir: &mut GitStr, sub: &[u8], predicate: fn(&CStr) -> bool) -> bool {
    let dir_size = dir.len();
    let sub = c_str(sub);
    let Some(alloc_size) = dir_size
        .checked_add(sub.len())
        .and_then(|n| n.checked_add(2))
    else {
        let _ = errors::oom();
        return false;
    };
    if dir.try_grow(alloc_size, false).is_err() {
        return false;
    }
    let n = str_c_bytes(dir).len();
    dir.truncate(n);
    if dir.joinpath_self(sub).is_err() {
        return false;
    }
    let result = predicate(&str_cpath(dir));
    dir.truncate(dir_size);
    result
}

/// `git_fs_path_contains`: whether `dir/item` exists.
pub fn contains(dir: &mut GitStr, item: &[u8]) -> bool {
    check_dir_contents(dir, item, exists)
}

/// `git_fs_path_contains_dir`.
pub fn contains_dir(base: &mut GitStr, subdir: &[u8]) -> bool {
    check_dir_contents(base, subdir, isdir)
}

/// `git_fs_path_contains_file`.
pub fn contains_file(base: &mut GitStr, file: &[u8]) -> bool {
    check_dir_contents(base, file, isfile)
}

/// `git_fs_path_normalize_slashes`: append `path` to `out`, then turn
/// backslashes into slashes.
pub fn normalize_slashes(out: &mut GitStr, path: &[u8]) -> Result<()> {
    out.puts(c_str(path))?;
    for c in out.as_mut_bytes() {
        if *c == 0 {
            break;
        }
        if *c == b'\\' {
            *c = b'/';
        }
    }
    Ok(())
}

/// `git_fs_path_supports_symlinks`: whether symlinks can be made in `dir`.
pub fn supports_symlinks(dir: &[u8]) -> bool {
    let mut path = GitStr::new();
    let mut supported = false;
    if let Ok(fd) = super::futils::mktmp(&mut path, dir, 0o666) {
        let p = str_cpath(&path).into_owned();
        if posix::close(fd).is_ok()
            && posix::unlink(&p).is_ok()
            && posix::symlink(c"testing", &p).is_ok()
            && let Ok(st) = posix::lstat(&p)
        {
            supported = posix::s_islnk(st.st_mode);
        }
    }
    if !path.is_empty() {
        let _ = posix::unlink(&str_cpath(&path));
    }
    supported
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[inline(always)]
fn validate_dospath(component: &[u8], dospath: &[u8; 3], trailing_num: bool) -> bool {
    let last = if trailing_num { 4 } else { 3 };
    let len = component.len();
    if len < last || !component[..3].eq_ignore_ascii_case(dospath) {
        return true;
    }
    if trailing_num && !(b'1'..=b'9').contains(&component[3]) {
        return true;
    }
    len > last && component[last] != b'.' && component[last] != b':'
}

const CLASS_PLAIN: u8 = 0;
const CLASS_REJECT: u8 = 1;
const CLASS_SLASH: u8 = 2;
const CLASS_NUL: u8 = 3;

/// The index in [`CHAR_CLASSES`] of the flags that `validate_char` reads.
#[inline(always)]
fn char_table_index(flags: c_uint) -> usize {
    (flags & GIT_FS_PATH_REJECT_BACKSLASH != 0) as usize
        | ((flags & GIT_FS_PATH_REJECT_SLASH != 0) as usize) << 1
        | ((flags & GIT_FS_PATH_REJECT_NT_CHARS != 0) as usize) << 2
}

/// `validate_char` (and the slash and the NUL) as tables, one per
/// combination of the three flags it reads.
static CHAR_CLASSES: [[u8; 256]; 8] = {
    let mut t = [[CLASS_PLAIN; 256]; 8];
    let mut idx = 0;
    while idx < 8 {
        let mut flags = 0;
        if idx & 1 != 0 {
            flags |= GIT_FS_PATH_REJECT_BACKSLASH;
        }
        if idx & 2 != 0 {
            flags |= GIT_FS_PATH_REJECT_SLASH;
        }
        if idx & 4 != 0 {
            flags |= GIT_FS_PATH_REJECT_NT_CHARS;
        }
        let mut c = 1;
        while c < 256 {
            t[idx][c] = if !validate_char(c as u8, flags) {
                CLASS_REJECT
            } else if c == b'/' as usize {
                CLASS_SLASH
            } else {
                CLASS_PLAIN
            };
            c += 1;
        }
        t[idx][0] = CLASS_NUL;
        idx += 1;
    }
    t
};

#[inline]
const fn validate_char(c: u8, flags: c_uint) -> bool {
    if flags & GIT_FS_PATH_REJECT_BACKSLASH != 0 && c == b'\\' {
        return false;
    }
    if flags & GIT_FS_PATH_REJECT_SLASH != 0 && c == b'/' {
        return false;
    }
    if flags & GIT_FS_PATH_REJECT_NT_CHARS != 0 {
        if c < 32 {
            return false;
        }
        if matches!(c, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*') {
            return false;
        }
    }
    true
}

#[inline(always)]
fn validate_component(component: &[u8], flags: c_uint) -> bool {
    let len = component.len();
    if len == 0 {
        return flags & GIT_FS_PATH_REJECT_EMPTY_COMPONENT == 0;
    }
    if flags & GIT_FS_PATH_REJECT_TRAVERSAL != 0 && (component == b"." || component == b"..") {
        return false;
    }
    let last = component[len - 1];
    if flags & GIT_FS_PATH_REJECT_TRAILING_DOT != 0 && last == b'.' {
        return false;
    }
    if flags & GIT_FS_PATH_REJECT_TRAILING_SPACE != 0 && last == b' ' {
        return false;
    }
    if flags & GIT_FS_PATH_REJECT_TRAILING_COLON != 0 && last == b':' {
        return false;
    }
    if flags & GIT_FS_PATH_REJECT_DOS_PATHS != 0
        && (!validate_dospath(component, b"CON", false)
            || !validate_dospath(component, b"PRN", false)
            || !validate_dospath(component, b"AUX", false)
            || !validate_dospath(component, b"NUL", false)
            || !validate_dospath(component, b"COM", true)
            || !validate_dospath(component, b"LPT", true))
    {
        return false;
    }
    true
}

/// A check of each byte of a path ([`ValidateCallbacks`]).
pub type CharCheck<'a> = &'a mut dyn FnMut(u8) -> bool;
/// A check of each component of a path ([`ValidateCallbacks`]).
pub type ComponentCheck<'a> = &'a mut dyn FnMut(&[u8]) -> bool;

/// The callbacks of [`str_is_valid_ext`].
pub struct ValidateCallbacks<'a> {
    /// `validate_char_cb`.
    pub char_cb: Option<CharCheck<'a>>,
    /// `validate_component_cb`.
    pub component_cb: Option<ComponentCheck<'a>>,
}

/// `git_fs_path_str_is_valid_ext`: whether `path` is valid under `flags`.
///
/// `sized` is whether `path` is a `git_str`'s `size` bytes (then a NUL
/// within them makes it invalid); otherwise it is a C string
/// (`git_fs_path_is_valid`, size `SIZE_MAX`) and ends at its first NUL.
/// `GIT_FS_PATH_REJECT_LONG_PATHS` has no effect on Unix.
#[inline]
pub fn str_is_valid_ext(
    path: &[u8],
    sized: bool,
    flags: c_uint,
    cbs: ValidateCallbacks<'_>,
) -> bool {
    let limit = if sized { path.len() } else { usize::MAX };
    str_is_valid_with(path, limit, flags, cbs)
}

/// [`str_is_valid_ext`] over a [`CBytes`]: the bytes up to the first NUL
/// or `limit` (a `git_str`'s `size`; `usize::MAX` for a C string), which
/// must be reached without a NUL for a sized path to be valid.
#[inline]
pub fn str_is_valid_with<P: CBytes>(
    path: P,
    limit: usize,
    flags: c_uint,
    cbs: ValidateCallbacks<'_>,
) -> bool {
    let ValidateCallbacks {
        mut char_cb,
        mut component_cb,
    } = cbs;
    if flags == 0 {
        return true;
    }
    let table = &CHAR_CLASSES[char_table_index(flags)];
    let mut start = 0usize;
    let mut i = 0usize;
    while i < limit {
        let c = path.at(i);
        // One lookup per byte: the NUL, a rejected byte, a slash, or none
        // of these (`validate_char` under `flags`, as a table).
        let class = table[c as usize];
        if class != CLASS_PLAIN {
            if class == CLASS_NUL {
                break;
            }
            if class == CLASS_REJECT {
                return false;
            }
        }
        if let Some(cb) = char_cb.as_mut()
            && !cb(c)
        {
            return false;
        }
        if class == CLASS_SLASH {
            let comp = path.seg(start, i);
            if !validate_component(comp, flags) {
                return false;
            }
            if let Some(cb) = component_cb.as_mut()
                && !cb(comp)
            {
                return false;
            }
            start = i + 1;
        }
        i += 1;
    }
    if limit != usize::MAX && i != limit {
        return false;
    }
    let comp = path.seg(start, i);
    if !validate_component(comp, flags) {
        return false;
    }
    if let Some(cb) = component_cb.as_mut()
        && !cb(comp)
    {
        return false;
    }
    true
}

/// `git_fs_path_is_valid`.
#[inline]
pub fn is_valid(path: &[u8], flags: c_uint) -> bool {
    str_is_valid_ext(
        path,
        false,
        flags,
        ValidateCallbacks {
            char_cb: None,
            component_cb: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

static MOCK_OWNER: AtomicU32 = AtomicU32::new(GIT_FS_PATH_OWNER_NONE);

/// `git_fs_path__set_owner`: make [`owner_is`] answer from `owner` (for
/// tests); `GIT_FS_PATH_OWNER_NONE` restores the real check.
pub fn set_owner(owner: c_uint) {
    MOCK_OWNER.store(owner, Ordering::Relaxed);
}

fn sudo_uid_lookup() -> Option<libc::uid_t> {
    let mut uid_str = GitStr::new();
    super::util::getenv(&mut uid_str, c"SUDO_UID").ok()?;
    let (uid, _) = super::util::strntol64(uid_str.as_bytes(), 10).ok()?;
    if uid == uid as libc::uid_t as i64 {
        Some(uid as libc::uid_t)
    } else {
        None
    }
}

/// `git_fs_path_owner_is`: whether `path` is owned as `owner_type` allows.
pub fn owner_is(path: &CStr, owner_type: c_uint) -> Result<bool> {
    let mock = MOCK_OWNER.load(Ordering::Relaxed);
    if mock != 0 {
        return Ok(mock & owner_type != 0);
    }
    // SAFETY: `geteuid` has no preconditions.
    let euid = unsafe { libc::geteuid() };
    let st = match posix::lstat(path) {
        Ok(st) => st,
        Err(posix::Errno(libc::ENOENT)) => return Err(ErrorCode::NOT_FOUND),
        Err(_) => {
            return Err(git_error_set!(
                ErrorClass::OS,
                "could not stat '{}'",
                Bytes(path.to_bytes())
            ));
        }
    };
    if owner_type & GIT_FS_PATH_OWNER_CURRENT_USER != 0 && st.st_uid == euid {
        return Ok(true);
    }
    if owner_type & GIT_FS_PATH_OWNER_ADMINISTRATOR != 0 && st.st_uid == 0 {
        return Ok(true);
    }
    if owner_type & GIT_FS_PATH_OWNER_RUNNING_SUDO != 0
        && euid == 0
        && sudo_uid_lookup().is_some_and(|uid| st.st_uid == uid)
    {
        return Ok(true);
    }
    Ok(false)
}

/// `git_fs_path_owner_is_current_user`.
pub fn owner_is_current_user(path: &CStr) -> Result<bool> {
    owner_is(path, GIT_FS_PATH_OWNER_CURRENT_USER)
}

/// `git_fs_path_owner_is_system`.
pub fn owner_is_system(path: &CStr) -> Result<bool> {
    owner_is(path, GIT_FS_PATH_OWNER_ADMINISTRATOR)
}

/// `git_fs_path_find_executable`: `executable` itself if it has a slash,
/// else the first executable `dir/executable` of `$PATH` (an empty entry
/// is `.`).
pub fn find_executable(fullpath: &mut GitStr, executable: &[u8]) -> Result<()> {
    let executable = c_str(executable);
    if memchr::memchr(b'/', executable).is_some() {
        if !isexecutable(&to_cstring(executable)) {
            return Err(ErrorCode::NOT_FOUND);
        }
        return fullpath.puts(executable);
    }
    let mut path = GitStr::new();
    if super::util::getenv(&mut path, c"PATH").is_err() {
        return Err(ErrorCode::GENERIC);
    }
    let dirs = str_c_bytes(&path);
    let mut cur = 0usize;
    while cur < dirs.len() {
        let term =
            memchr::memchr(GIT_PATH_LIST_SEPARATOR, &dirs[cur..]).map_or(dirs.len(), |i| cur + i);
        fullpath.clear();
        if term == cur {
            fullpath.putc(b'.')?;
        } else {
            fullpath.put(&dirs[cur..term])?;
        }
        fullpath.putc(b'/')?;
        fullpath.puts(executable)?;
        if isexecutable(&str_cpath(fullpath)) {
            return Ok(());
        }
        cur = term;
        if at(dirs, cur) == GIT_PATH_LIST_SEPARATOR {
            cur += 1;
        }
    }
    fullpath.clear();
    Err(ErrorCode::NOT_FOUND)
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

/// The C ABI of `fs_path.c`.
pub mod ffi;

crate::c_layout_test! {
    headers: ["fs_path.h"],
    types: [
        GitFsPathDiriter => "git_fs_path_diriter" { path, parent_len, flags, dir },
    ],
    asserts: [
        "GIT_FS_PATH_DIR_IGNORE_CASE == 1", "GIT_FS_PATH_DIR_PRECOMPOSE_UNICODE == 2",
        "GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT == 4",
        "GIT_FS_PATH_REJECT_EMPTY_COMPONENT == 1", "GIT_FS_PATH_REJECT_TRAVERSAL == 2",
        "GIT_FS_PATH_REJECT_SLASH == 4", "GIT_FS_PATH_REJECT_BACKSLASH == 8",
        "GIT_FS_PATH_REJECT_TRAILING_DOT == 16", "GIT_FS_PATH_REJECT_TRAILING_SPACE == 32",
        "GIT_FS_PATH_REJECT_TRAILING_COLON == 64", "GIT_FS_PATH_REJECT_DOS_PATHS == 128",
        "GIT_FS_PATH_REJECT_NT_CHARS == 256", "GIT_FS_PATH_REJECT_LONG_PATHS == 512",
        "GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS == 3",
        "GIT_FS_PATH_OWNER_CURRENT_USER == 1", "GIT_FS_PATH_OWNER_ADMINISTRATOR == 2",
        "GIT_FS_PATH_USER_IS_ADMINISTRATOR == 4", "GIT_FS_PATH_OWNER_RUNNING_SUDO == 8",
        "GIT_FS_PATH_OWNER_OTHER == 16", "GIT_PATH_MAX == 4096",
        "GIT_FS_PATH_EQUAL == 1", "GIT_FS_PATH_PREFIX == 2",
    ],
}

#[cfg(test)]
mod tests;
