//! Port of `src/libgit2/parse.c`: `git_parse_ctx`, the line-oriented cursor
//! that libgit2's text parsers (config, patch, mailmap, grafts, packed refs)
//! share.
//!
//! [`GitParseCtx`] is the mirrored `git_parse_ctx` (kind 3: C embeds it by
//! value, e.g. in `git_config_parser` and `git_patch_parse_ctx`, and reads
//! its fields directly). Its lifetime is the content's: a context made from
//! a `&'a [u8]` reads only that slice, so its methods are safe.
//!
//! As in C, `remain` keeps pointing at the start of the content; the
//! unparsed bytes are `line .. line + remain_len` ([`GitParseCtx::rest`]).
//! Failures are C's -1 with no error message, except where a callee sets
//! one (`advance_digit`'s overflow, `advance_oid`'s parse).
//!
//! ```
//! use libgit2_rs::__test::parse::GitParseCtx;
//! let mut ctx = GitParseCtx::new(b"key = 42\nnext\n");
//! assert!(ctx.contains(b"key"));
//! ctx.advance_expected(b"key")?;
//! ctx.advance_ws()?;
//! ctx.advance_expected(b"=")?;
//! ctx.advance_ws()?;
//! assert_eq!(ctx.advance_digit(10)?, 42);
//! ctx.advance_nl()?;
//! assert_eq!(ctx.line(), b"next\n");
//! assert_eq!(ctx.line_num(), 2);
//! # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
//! ```

use core::marker::PhantomData;

use crate::oid::{Oid, OidType};
use crate::util::ctype_compat::{isdigit, isspace};
use crate::util::errors::{ErrorCode, Result};
use crate::util::util::{linenlen, strntol64};

/// `GIT_PARSE_PEEK_SKIP_WHITESPACE`.
pub const GIT_PARSE_PEEK_SKIP_WHITESPACE: i32 = 1 << 0;

/// The `""` that C's contexts point at when they have no content.
const EMPTY: &[u8; 1] = b"\0";

/// `git_parse_ctx`, mirrored, over content that lives for `'a`.
///
/// Invariant: `content` is valid for `content_len` bytes for `'a` (or
/// `content_len` is 0); `line` is within it (or NULL with every length 0,
/// after [`clear`](GitParseCtx::clear)), with `line + remain_len` the end
/// of the content and `line_len <= remain_len`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitParseCtx<'a> {
    content: *const u8,
    content_len: usize,
    remain: *const u8,
    remain_len: usize,
    line: *const u8,
    line_len: usize,
    line_num: usize,
    _content: PhantomData<&'a [u8]>,
}

// SAFETY: the context only reads shared, immutable content.
unsafe impl Send for GitParseCtx<'_> {}
// SAFETY: as above.
unsafe impl Sync for GitParseCtx<'_> {}

impl Default for GitParseCtx<'_> {
    /// `GIT_PARSE_CTX_INIT`: all zero.
    fn default() -> Self {
        GitParseCtx {
            content: core::ptr::null(),
            content_len: 0,
            remain: core::ptr::null(),
            remain_len: 0,
            line: core::ptr::null(),
            line_len: 0,
            line_num: 0,
            _content: PhantomData,
        }
    }
}

/// `len` bytes at `p` for `'a`.
///
/// # Safety
///
/// When `len > 0`, `p` is valid for `len` bytes for `'a`.
#[inline(always)]
unsafe fn slice<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

impl<'a> GitParseCtx<'a> {
    /// `git_parse_ctx_init`: a context at the first line of `content`.
    pub fn new(content: &'a [u8]) -> GitParseCtx<'a> {
        let mut c = GitParseCtx::default();
        c.init(content);
        c
    }

    /// `git_parse_ctx_init`.
    pub fn init(&mut self, content: &'a [u8]) {
        let (p, len) = if content.is_empty() {
            (EMPTY.as_ptr(), 0)
        } else {
            (content.as_ptr(), content.len())
        };
        self.content = p;
        self.content_len = len;
        self.remain = p;
        self.remain_len = len;
        self.line = p;
        self.line_len = linenlen(content);
        self.line_num = 1;
    }

    /// `git_parse_ctx_clear`: all zero, the content `""`.
    pub fn clear(&mut self) {
        *self = GitParseCtx::default();
        self.content = EMPTY.as_ptr();
    }

    /// The whole content.
    #[inline]
    pub fn content(&self) -> &'a [u8] {
        // SAFETY: the invariant.
        unsafe { slice(self.content, self.content_len) }
    }

    /// The current line, with its `\n` (what is left of it).
    #[inline]
    pub fn line(&self) -> &'a [u8] {
        // SAFETY: the invariant: `line_len <= remain_len` bytes at `line`.
        unsafe { slice(self.line, self.line_len) }
    }

    /// The unparsed bytes: the current line and those after it.
    #[inline]
    pub fn rest(&self) -> &'a [u8] {
        // SAFETY: the invariant: `remain_len` bytes at `line`.
        unsafe { slice(self.line, self.remain_len) }
    }

    /// The offset of the current position in the content.
    #[inline]
    pub fn offset(&self) -> usize {
        self.content_len - self.remain_len
    }

    /// `line_len`.
    #[inline]
    pub fn line_len(&self) -> usize {
        self.line_len
    }

    /// `remain_len`.
    #[inline]
    pub fn remain_len(&self) -> usize {
        self.remain_len
    }

    /// `line_num`: 1 for the first line.
    #[inline]
    pub fn line_num(&self) -> usize {
        self.line_num
    }

    /// `git_parse_ctx_contains`: whether the line starts with `s`.
    #[inline]
    pub fn contains(&self, s: &[u8]) -> bool {
        self.line().starts_with(s)
    }

    /// `git_parse_advance_line`: on to the next line.
    pub fn advance_line(&mut self) {
        self.line = self.line.wrapping_add(self.line_len);
        self.remain_len -= self.line_len;
        self.line_len = linenlen(self.rest());
        self.line_num += 1;
    }

    /// `git_parse_advance_chars`: skip `n` bytes of the line (at most the
    /// rest of it; C does not check).
    pub fn advance_chars(&mut self, n: usize) {
        let n = n.min(self.line_len);
        self.line = self.line.wrapping_add(n);
        self.remain_len -= n;
        self.line_len -= n;
    }

    /// `git_parse_advance_expected`: skip `expected` if the line starts
    /// with it.
    pub fn advance_expected(&mut self, expected: &[u8]) -> Result<()> {
        if !self.contains(expected) {
            return Err(ErrorCode::GENERIC);
        }
        self.advance_chars(expected.len());
        Ok(())
    }

    /// `git_parse_advance_ws`: skip whitespace other than `\n`; fails if
    /// there was none.
    pub fn advance_ws(&mut self) -> Result<()> {
        let n = self
            .line()
            .iter()
            .take_while(|&&c| c != b'\n' && isspace(c))
            .count();
        if n == 0 {
            return Err(ErrorCode::GENERIC);
        }
        self.advance_chars(n);
        Ok(())
    }

    /// `git_parse_advance_nl`: if the line is just `\n`, on to the next.
    pub fn advance_nl(&mut self) -> Result<()> {
        if self.line() != b"\n" {
            return Err(ErrorCode::GENERIC);
        }
        self.advance_line();
        Ok(())
    }

    /// `git_parse_advance_digit`: parse a number in `base` (with
    /// `git__strntol64`) at the start of the line, which must be a digit.
    pub fn advance_digit(&mut self, base: i32) -> Result<i64> {
        let line = self.line();
        if !line.first().is_some_and(|&c| isdigit(c)) {
            return Err(ErrorCode::GENERIC);
        }
        let (v, n) = strntol64(line, base).map_err(|_| ErrorCode::GENERIC)?;
        self.advance_chars(n);
        Ok(v)
    }

    /// `git_parse_advance_oid`: parse a full hex id of type `t` at the
    /// start of the line.
    pub fn advance_oid(&mut self, t: OidType) -> Result<Oid> {
        let hexsize = t.hexsize();
        let line = self.line();
        if line.len() < hexsize {
            return Err(ErrorCode::GENERIC);
        }
        let oid = Oid::from_prefix(&line[..hexsize], t).map_err(|_| ErrorCode::GENERIC)?;
        self.advance_chars(hexsize);
        Ok(oid)
    }

    /// `git_parse_peek`: the line's first byte, after whitespace (`\n`
    /// included) if `skip_ws`; `None` if there is none.
    pub fn peek(&self, skip_ws: bool) -> Option<u8> {
        self.line()
            .iter()
            .copied()
            .find(|&c| !(skip_ws && isspace(c)))
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `parse.c`.
pub mod ffi {
    use core::ffi::{c_char, c_int};

    use super::*;
    use crate::git_assert;
    use crate::oid::{git_oid_t, oid_hexsize};
    use crate::util::export::{barrier, bytes, status};

    /// A C context: its content is C's to keep alive.
    type Ctx = GitParseCtx<'static>;

    /// `git_parse_ctx_init`.
    ///
    /// # Safety
    ///
    /// `ctx` is writable; `content` is NULL or readable for `content_len`
    /// bytes for as long as the context is used.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_ctx_init(
        ctx: *mut Ctx,
        content: *const c_char,
        content_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller keeps the content alive while the context
            // is used: that is the context's `'static` here.
            let content = unsafe { bytes(content, content_len) };
            // SAFETY: writable.
            unsafe { ctx.write(GitParseCtx::new(content)) };
            0
        })
    }

    /// `git_parse_ctx_clear`.
    ///
    /// # Safety
    ///
    /// `ctx` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_ctx_clear(ctx: *mut Ctx) {
        let mut c = GitParseCtx::default();
        c.clear();
        // SAFETY: writable.
        unsafe { ctx.write(c) };
    }

    /// The context, exclusively for the call.
    ///
    /// # Safety
    ///
    /// `ctx` is a valid context (made by `git_parse_ctx_init` and then
    /// only advanced) whose content is alive; nothing else uses it during
    /// the call.
    unsafe fn ctx_mut<'c>(ctx: *mut Ctx) -> &'c mut Ctx {
        // SAFETY: the caller's contract.
        unsafe { &mut *ctx }
    }

    /// `git_parse_advance_line`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_line(ctx: *mut Ctx) {
        // SAFETY: forwarded.
        barrier(|| unsafe { ctx_mut(ctx) }.advance_line())
    }

    /// `git_parse_advance_chars`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_chars(ctx: *mut Ctx, char_cnt: usize) {
        // SAFETY: forwarded.
        barrier(|| unsafe { ctx_mut(ctx) }.advance_chars(char_cnt))
    }

    /// `git_parse_advance_expected`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`]; `expected` is readable for `expected_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_expected(
        ctx: *mut Ctx,
        expected: *const c_char,
        expected_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: forwarded.
            let ctx = unsafe { ctx_mut(ctx) };
            // SAFETY: `expected_len` readable bytes.
            let e = unsafe { bytes(expected, expected_len) };
            status(ctx.advance_expected(e))
        })
    }

    /// `git_parse_advance_ws`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_ws(ctx: *mut Ctx) -> c_int {
        // SAFETY: forwarded.
        barrier(|| status(unsafe { ctx_mut(ctx) }.advance_ws()))
    }

    /// `git_parse_advance_nl`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_nl(ctx: *mut Ctx) -> c_int {
        // SAFETY: forwarded.
        barrier(|| status(unsafe { ctx_mut(ctx) }.advance_nl()))
    }

    /// `git_parse_advance_digit`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`]; `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_digit(
        out: *mut i64,
        ctx: *mut Ctx,
        base: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: forwarded.
            match unsafe { ctx_mut(ctx) }.advance_digit(base) {
                Ok(v) => {
                    // SAFETY: writable.
                    unsafe { out.write(v) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_parse_advance_oid`. As in C, `out` holds what
    /// `git_oid_from_prefix` left there when the digits are invalid.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`]; `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_advance_oid(
        out: *mut Oid,
        ctx: *mut Ctx,
        oid_type: git_oid_t,
    ) -> c_int {
        barrier(|| {
            let hexsize = oid_hexsize(oid_type);
            git_assert!(hexsize != 0, "oid_hexsize", -1);
            // SAFETY: forwarded.
            let ctx = unsafe { ctx_mut(ctx) };
            let line = ctx.line();
            if line.len() < hexsize {
                return -1;
            }
            // SAFETY: `out` is writable and the line has `hexsize` bytes.
            let r = unsafe {
                crate::oid::ffi::git_oid_from_prefix(out, line.as_ptr().cast(), hexsize, oid_type)
            };
            if r < 0 {
                return -1;
            }
            ctx.advance_chars(hexsize);
            0
        })
    }

    /// `git_parse_peek`.
    ///
    /// # Safety
    ///
    /// As [`ctx_mut`]; `out` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_parse_peek(
        out: *mut c_char,
        ctx: *mut Ctx,
        flags: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: forwarded.
            let ctx = unsafe { ctx_mut(ctx) };
            match ctx.peek(flags & GIT_PARSE_PEEK_SKIP_WHITESPACE != 0) {
                Some(c) => {
                    // SAFETY: writable.
                    unsafe { out.write(c as c_char) };
                    0
                }
                None => -1,
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["parse.h"],
    types: [
        GitParseCtx<'static> => "git_parse_ctx" {
            content, content_len, remain, remain_len, line, line_len, line_num
        },
    ],
    asserts: ["GIT_PARSE_PEEK_SKIP_WHITESPACE == 1"],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        let mut c = GitParseCtx::new(b"a\nbb\nccc");
        assert_eq!(c.line(), b"a\n");
        assert_eq!(c.line_num(), 1);
        c.advance_line();
        assert_eq!(c.line(), b"bb\n");
        assert_eq!(c.rest(), b"bb\nccc");
        c.advance_chars(1);
        assert_eq!(c.line(), b"b\n");
        c.advance_chars(100);
        assert_eq!(c.line(), b"");
        assert!(c.advance_nl().is_err());
        c.advance_line();
        assert_eq!(c.line(), b"ccc");
        assert_eq!(c.line_num(), 3);
        c.advance_line();
        assert_eq!(c.line(), b"");
        assert_eq!(c.rest(), b"");
        assert_eq!(c.offset(), 8);
        assert_eq!(c.content(), b"a\nbb\nccc");
    }

    #[test]
    fn tokens() {
        let _g = crate::util::test_support::init();
        let mut c = GitParseCtx::new(b" \t x 0x1f\n");
        assert_eq!(c.peek(false), Some(b' '));
        assert_eq!(c.peek(true), Some(b'x'));
        c.advance_ws().unwrap();
        assert!(c.advance_ws().is_err());
        assert!(c.advance_expected(b"y").is_err());
        c.advance_expected(b"x ").unwrap();
        assert_eq!(c.advance_digit(16).unwrap(), 0x1f);
        assert!(c.advance_digit(10).is_err());
        c.advance_nl().unwrap();
        assert_eq!(c.peek(true), None);
        let mut c = GitParseCtx::new(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 rest");
        assert_eq!(c.advance_oid(OidType::Sha1).unwrap(), Oid::EMPTY_BLOB_SHA1);
        assert_eq!(c.line(), b" rest");
        assert!(c.advance_oid(OidType::Sha1).is_err());
    }

    #[test]
    fn empty_and_clear() {
        let mut c = GitParseCtx::new(b"");
        assert_eq!(c.content(), b"");
        assert_eq!(c.line_num(), 1);
        assert!(!c.content.is_null());
        c.clear();
        assert_eq!(c.line(), b"");
        assert_eq!(c.line_num(), 0);
        assert!(c.advance_ws().is_err());
        assert_eq!(c.peek(true), None);
    }
}
