//! Port of `src/libgit2/signature.c`: `git_signature` (an author, committer
//! or tagger: name, e-mail and time), its parser for object headers, and its
//! serialisation.
//!
//! # Types
//!
//! - [`GitSignature`] / [`GitTime`] are the public `git_signature` /
//!   `git_time` (kind 3, mirrored). C allocates them (`commit.c` does
//!   `git__malloc(sizeof(git_signature))` and parses into it), embeds them
//!   (`git_time` in `git_signature`, `git_signature` on the stack), and frees
//!   them with [`ffi::git_signature_free`]; `name` and `email` come from
//!   `git__allocator`.
//! - [`Signature`] is the safe owner: a `GitSignature` whose `name` and
//!   `email` are NUL-terminated `git__allocator` strings it owns.
//!   `#[repr(transparent)]`, so a `&Signature` is a valid
//!   `const git_signature *` (a tag's tagger is handed to C that way).
//! - [`Time`] is the safe API's `git_time`.
//!
//! # Parsing
//!
//! [`parse`] is `git_signature__parse` on a byte slice: it returns the
//! signature and the number of bytes consumed (the C's new `*buffer_out`,
//! which may be one past the end when there is no `ender`). It runs in safe
//! Rust and is checked against the C by `rust/difftest/obj_diff.c`.

use crate::util::cabi::Out;
use core::ffi::{CStr, c_char, c_int};
use core::fmt;
use core::ptr;

use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::util as gutil;
use crate::{git_assert_arg, git_error_set};

/// `git_time_t`.
#[allow(non_camel_case_types)]
pub type git_time_t = i64;

/// `git_time`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GitTime {
    /// Seconds since the epoch.
    pub time: git_time_t,
    /// The offset from UTC in minutes.
    pub offset: c_int,
    /// `'+'` or `'-'` (0 when the parser found no valid timezone).
    pub sign: c_char,
}

/// `git_signature`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitSignature {
    /// The name (NUL-terminated, `git__allocator`).
    pub name: *mut c_char,
    /// The e-mail (NUL-terminated, `git__allocator`).
    pub email: *mut c_char,
    /// The time.
    pub when: GitTime,
}

/// `git_signature`.
#[allow(non_camel_case_types)]
pub type git_signature = GitSignature;

impl GitSignature {
    /// All zero (C's `memset(sig, 0, sizeof(git_signature))`).
    pub const ZERO: GitSignature = GitSignature {
        name: ptr::null_mut(),
        email: ptr::null_mut(),
        when: GitTime {
            time: 0,
            offset: 0,
            sign: 0,
        },
    };
}

// ---------------------------------------------------------------------------
// The safe owner
// ---------------------------------------------------------------------------

/// A time with its timezone offset (`git_time`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Time {
    raw: GitTime,
}

impl Time {
    /// A time `seconds` after the epoch at `offset_minutes` from UTC; the
    /// sign is `-` for a negative offset, `+` otherwise (as
    /// `git_signature_new`).
    pub const fn new(seconds: i64, offset_minutes: i32) -> Time {
        Time {
            raw: GitTime {
                time: seconds,
                offset: offset_minutes,
                sign: if offset_minutes < 0 {
                    b'-' as c_char
                } else {
                    b'+' as c_char
                },
            },
        }
    }

    /// Seconds since the epoch.
    pub const fn seconds(&self) -> i64 {
        self.raw.time
    }

    /// The offset from UTC in minutes.
    pub const fn offset_minutes(&self) -> i32 {
        self.raw.offset
    }

    /// The sign character (`b'+'`, `b'-'`, or 0 when a parsed timezone was
    /// out of range).
    pub const fn sign(&self) -> u8 {
        self.raw.sign as u8
    }

    /// The C value.
    pub(crate) const fn raw(&self) -> GitTime {
        self.raw
    }
}

impl Time {
    /// From C's value.
    pub(crate) const fn from_raw(raw: GitTime) -> Time {
        Time { raw }
    }
}

/// A borrowed signature: the name, e-mail and time of an author, committer
/// or tagger, as an object stores them (what [`Commit::author`],
/// [`Tag::tagger`] and [`ReflogEntry::committer`] return; [`Signature`]
/// is the owned form).
///
/// [`Commit::author`]: crate::Commit::author
/// [`Tag::tagger`]: crate::Tag::tagger
/// [`ReflogEntry::committer`]: crate::ReflogEntry::committer
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignatureRef<'a> {
    name: &'a [u8],
    email: &'a [u8],
    when: Time,
}

impl<'a> SignatureRef<'a> {
    /// A view of `name`, `email` and `when`, taken as they are (no
    /// trimming or checks: [`Signature::new`] does those).
    pub const fn new(name: &'a [u8], email: &'a [u8], when: Time) -> SignatureRef<'a> {
        SignatureRef { name, email, when }
    }

    /// The view of a C signature.
    ///
    /// # Safety
    ///
    /// `s`'s strings are NULL (read as empty) or NUL-terminated, and live
    /// and unwritten for `'a`.
    pub(crate) unsafe fn from_raw(s: &'a GitSignature) -> SignatureRef<'a> {
        let b = |p: *const c_char| -> &'a [u8] {
            if p.is_null() {
                b""
            } else {
                // SAFETY: the caller's contract.
                unsafe { CStr::from_ptr(p) }.to_bytes()
            }
        };
        SignatureRef {
            name: b(s.name),
            email: b(s.email),
            when: Time { raw: s.when },
        }
    }

    /// The name (bytes, usually UTF-8).
    pub const fn name(&self) -> &'a [u8] {
        self.name
    }

    /// The name, if it is UTF-8.
    pub fn name_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.name).ok()
    }

    /// The e-mail address.
    pub const fn email(&self) -> &'a [u8] {
        self.email
    }

    /// The e-mail address, if it is UTF-8.
    pub fn email_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.email).ok()
    }

    /// The time.
    pub const fn when(&self) -> Time {
        self.when
    }

    /// An owned copy (`git_signature_dup`), checked as
    /// [`Signature::new`] checks.
    pub fn to_signature(&self) -> core::result::Result<Signature, Error> {
        crate::libgit2::ensure_init()?;
        Ok(dup_parts(self.name, self.email, self.when.raw)?)
    }
}

impl<'a> From<&'a Signature> for SignatureRef<'a> {
    fn from(s: &'a Signature) -> SignatureRef<'a> {
        s.as_ref()
    }
}

impl fmt::Debug for SignatureRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignatureRef")
            .field("name", &Bytes(self.name))
            .field("email", &Bytes(self.email))
            .field("when", &self.when)
            .finish()
    }
}

impl fmt::Display for SignatureRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", Bytes(self.name), Bytes(self.email))
    }
}

/// An owned signature (`git_signature`), freed on drop.
///
/// Invariant: `name` and `email` are NUL-terminated `git__allocator`
/// strings owned by the value (never NULL), written by nothing else.
#[repr(transparent)]
pub struct Signature(GitSignature);

// SAFETY: the strings belong to the value alone; `git__allocator` frees
// from any thread.
unsafe impl Send for Signature {}
// SAFETY: `&Signature` only reads.
unsafe impl Sync for Signature {}

impl Drop for Signature {
    fn drop(&mut self) {
        // SAFETY: by the invariant, the value's own allocations, freed once.
        unsafe {
            alloc::free(self.0.name.cast());
        }
        // SAFETY: as above.
        unsafe { alloc::free(self.0.email.cast()) };
    }
}

impl Signature {
    /// Build from owned strings.
    fn from_parts(name: GitCStr, email: GitCStr, when: GitTime) -> Signature {
        Signature(GitSignature {
            name: name.into_raw(),
            email: email.into_raw(),
            when,
        })
    }

    /// `git_signature_new`: a signature for `name` and `email` (trimmed of
    /// leading and trailing "crud": spaces, control characters and
    /// `.,:;<>"\'`) at `time`. Fails if either contains an angle bracket or
    /// is empty once trimmed, or contains a NUL byte.
    pub fn new(
        name: impl AsRef<[u8]>,
        email: impl AsRef<[u8]>,
        time: Time,
    ) -> core::result::Result<Signature, Error> {
        crate::libgit2::ensure_init()?;
        let (name, email) = (name.as_ref(), email.as_ref());
        let (Some(name), Some(email)) = (no_nul(name), no_nul(email)) else {
            return Err(Error::from(signature_error(
                "Neither `name` nor `email` should contain NUL bytes.",
            )));
        };
        Ok(new_sig(name, email, time.raw.time, time.raw.offset)?)
    }

    /// `git_signature_now`: [`new`](Self::new) at the current time and
    /// local timezone.
    pub fn now(
        name: impl AsRef<[u8]>,
        email: impl AsRef<[u8]>,
    ) -> core::result::Result<Signature, Error> {
        let (now, offset) = current_time();
        Signature::new(name, email, Time::new(now, offset))
    }

    /// `git_signature_from_buffer`: parse `name <email> time tz` (the text
    /// up to the first NUL byte).
    pub fn from_buffer(buf: impl AsRef<[u8]>) -> core::result::Result<Signature, Error> {
        crate::libgit2::ensure_init()?;
        let buf = buf.as_ref();
        let buf = &buf[..memchr::memchr(0, buf).unwrap_or(buf.len())];
        Ok(parse(buf, None, 0)?.0)
    }

    /// `git_signature_default`: `user.name` and `user.email` of the
    /// repository's configuration, now.
    pub fn default_for(repo: &crate::object::Repository) -> core::result::Result<Signature, Error> {
        // SAFETY: a live repository.
        Ok(unsafe { default_for(crate::object::repo_ptr(repo), || ()) }?)
    }

    /// `git_signature_default_from_env`: the author and committer
    /// (`GIT_AUTHOR_NAME`/`_EMAIL`/`_DATE`, `GIT_COMMITTER_*`, else the
    /// configuration and `$EMAIL`).
    pub fn default_from_env(
        repo: &crate::object::Repository,
    ) -> core::result::Result<(Signature, Signature), Error> {
        // SAFETY: a live repository.
        let (a, c) = unsafe { default_from_env(crate::object::repo_ptr(repo), true, true) }?;
        match (a, c) {
            (Some(a), Some(c)) => Ok((a, c)),
            _ => Err(Error::from(ErrorCode::GENERIC)),
        }
    }

    /// The name.
    pub fn name(&self) -> &[u8] {
        // SAFETY: by the invariant, a live NUL-terminated string.
        unsafe { CStr::from_ptr(self.0.name) }.to_bytes()
    }

    /// The e-mail.
    pub fn email(&self) -> &[u8] {
        // SAFETY: as above.
        unsafe { CStr::from_ptr(self.0.email) }.to_bytes()
    }

    /// The time.
    pub fn when(&self) -> Time {
        Time { raw: self.0.when }
    }

    /// The borrowed view.
    pub fn as_ref(&self) -> SignatureRef<'_> {
        SignatureRef {
            name: self.name(),
            email: self.email(),
            when: self.when(),
        }
    }

    /// The name, if it is UTF-8.
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(self.name()).ok()
    }

    /// The e-mail address, if it is UTF-8.
    pub fn email_str(&self) -> Option<&str> {
        core::str::from_utf8(self.email()).ok()
    }

    /// `git_signature_dup`: a copy, or the out-of-memory error ([`Clone`]
    /// panics on that instead).
    pub fn try_clone(&self) -> core::result::Result<Signature, Error> {
        Ok(self.try_clone_raw()?)
    }

    /// `git_signature_dup` with the internal error.
    pub(crate) fn try_clone_raw(&self) -> Result<Signature> {
        dup_parts(self.name(), self.email(), self.0.when)
    }

    /// A copy of the C signature at `sig` (`git_signature_dup`; a NULL
    /// name or e-mail copies as empty).
    ///
    /// # Safety
    ///
    /// `sig` is a valid signature for the call.
    pub(crate) unsafe fn dup_raw(sig: *const GitSignature) -> Result<Signature> {
        // SAFETY: per the contract.
        let s = unsafe { &*sig };
        let str_of = |p: *const c_char| -> &[u8] {
            if p.is_null() {
                return b"";
            }
            // SAFETY: the signature's NUL-terminated strings, valid for the
            // call.
            unsafe { CStr::from_ptr(p) }.to_bytes()
        };
        dup_parts(str_of(s.name), str_of(s.email), s.when)
    }

    /// An empty signature (empty name and e-mail, time 0).
    pub(crate) fn empty() -> Result<Signature> {
        dup_parts(b"", b"", GitTime::default())
    }

    /// The C view (`const git_signature *`), valid while `self` is.
    pub(crate) fn as_raw(&self) -> *const GitSignature {
        &raw const self.0
    }

    /// The serialised form, `<header>name <email> time ±hhmm\n`
    /// (`git_signature__writebuf`).
    pub fn to_header(&self, header: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        format_into(&mut v, header, self.name(), self.email(), self.0.when);
        v
    }

    /// Move into a `git__allocator` block that C frees with
    /// `git_signature_free`.
    pub(crate) fn into_raw(self) -> Result<*mut GitSignature> {
        let p = alloc::malloc(size_of::<GitSignature>()).cast::<GitSignature>();
        if p.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        let me = core::mem::ManuallyDrop::new(self);
        // SAFETY: a fresh allocation of a `GitSignature` (8-aligned, as
        // `git__allocator`'s memory is); the strings move into it.
        unsafe {
            p.write(GitSignature {
                name: me.0.name,
                email: me.0.email,
                when: me.0.when,
            })
        };
        Ok(p)
    }
}

impl Clone for Signature {
    /// `git_signature_dup`. Panics if out of memory.
    fn clone(&self) -> Signature {
        self.try_clone_raw()
            .expect("out of memory duplicating a signature")
    }
}

/// `git_signature__equal`.
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        // SAFETY: the invariant: both strings of each are NUL-terminated
        // and live (one pass each, as C's `strcmp`).
        let names = unsafe { c_str_eq(self.0.name, other.0.name) };
        // SAFETY: as above.
        names && unsafe { c_str_eq(self.0.email, other.0.email) } && self.0.when == other.0.when
    }
}

impl Eq for Signature {}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signature")
            .field("name", &Bytes(self.name()))
            .field("email", &Bytes(self.email()))
            .field("when", &self.0.when)
            .finish()
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", Bytes(self.name()), Bytes(self.email()))
    }
}

// ---------------------------------------------------------------------------
// The C's logic, on bytes
// ---------------------------------------------------------------------------

fn no_nul(s: &[u8]) -> Option<&[u8]> {
    if memchr::memchr(0, s).is_some() {
        None
    } else {
        Some(s)
    }
}

/// `signature_parse_error`: `GIT_EINVALID`.
fn signature_parse_error(msg: &str) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::INVALID, "failed to parse signature - {}", msg);
    ErrorCode::INVALID
}

/// `signature_error`: -1.
fn signature_error(msg: &str) -> ErrorCode {
    git_error_set!(ErrorClass::INVALID, "failed to parse signature - {}", msg)
}

fn contains_angle_brackets(s: &[u8]) -> bool {
    memchr::memchr2(b'<', b'>', s).is_some()
}

/// `is_crud`.
#[inline]
fn is_crud(c: u8) -> bool {
    c <= 32 || matches!(c, b',' | b':' | b';' | b'<' | b'>' | b'"' | b'\\' | b'\'')
}

/// The trimmed range of `extract_trimmed`.
#[inline]
fn trimmed(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if !is_crud(*first) {
            break;
        }
        s = rest;
    }
    while let [rest @ .., last] = s {
        if !is_crud(*last) {
            break;
        }
        s = rest;
    }
    s
}

/// `git__substrdup` into an owner (the out-of-memory error set on failure).
fn dup(s: &[u8]) -> Result<GitCStr> {
    let p = alloc::substrdup(s);
    // SAFETY: NULL or a fresh NUL-terminated `git__allocator` string.
    unsafe { GitCStr::from_raw(p) }.ok_or(ErrorCode::GENERIC)
}

fn dup_parts(name: &[u8], email: &[u8], when: GitTime) -> Result<Signature> {
    let name = dup(name)?;
    let email = dup(email)?;
    Ok(Signature::from_parts(name, email, when))
}

/// `git_signature_new` on strings without NUL bytes.
pub(crate) fn new_sig(
    name: &[u8],
    email: &[u8],
    time: git_time_t,
    offset: c_int,
) -> Result<Signature> {
    if contains_angle_brackets(name) || contains_angle_brackets(email) {
        return Err(signature_error(
            "Neither `name` nor `email` should contain angle brackets chars.",
        ));
    }
    let (name_t, email_t) = (trimmed(name), trimmed(email));
    let name = dup(name_t)?;
    let email = dup(email_t)?;
    if name_t.is_empty() || email_t.is_empty() {
        return Err(signature_error(
            "Signature cannot have an empty name or email",
        ));
    }
    Ok(Signature::from_parts(
        name,
        email,
        GitTime {
            time,
            offset,
            sign: if offset < 0 {
                b'-' as c_char
            } else {
                b'+' as c_char
            },
        },
    ))
}

/// `current_time`: now, and the local timezone's offset in minutes (as C:
/// `mktime` of the UTC broken-down time with `tm_isdst = -1`).
pub(crate) fn current_time() -> (i64, c_int) {
    // SAFETY: `time(NULL)` has no precondition.
    let now = unsafe { libc::time(ptr::null_mut()) };
    // SAFETY: `tm` is plain data; zero is a valid value.
    let mut tm: libc::tm = unsafe { core::mem::zeroed() };
    // SAFETY: valid pointers to a `time_t` and a `tm`.
    let utc = unsafe { libc::gmtime_r(&raw const now, &raw mut tm) };
    if utc.is_null() {
        // C would dereference NULL here (rust/DEFECTS.md): an offset of 0.
        return (now, 0);
    }
    tm.tm_isdst = -1;
    // SAFETY: a valid `tm`.
    let local = unsafe { libc::mktime(&raw mut tm) };
    // `(time_t)difftime(now, local) / 60`
    // SAFETY: plain arithmetic in libm.
    let diff = unsafe { libc::difftime(now, local) } as i64;
    (now, (diff / 60) as c_int)
}

/// `git_signature__parse`: parse a signature at the start of `buf`.
///
/// With `ender` (not 0) the signature ends at its first occurrence, which
/// must exist ("no newline given"); with `header`, the text must start with
/// it. Returns the signature and the bytes consumed (up to and including
/// the end; `buf.len() + 1` when `ender` is 0, as C's pointer). Errors are
/// `GIT_EINVALID` with C's messages, or -1 when out of memory.
///
/// A timestamp that is not a number fails; a timezone that is not one
/// (or out of range) leaves the offset 0 (and, out of range, the sign 0).
pub(crate) fn parse(buf: &[u8], header: Option<&[u8]>, ender: u8) -> Result<(Signature, usize)> {
    let (f, used) = parse_fields(buf, header, ender)?;
    Ok((dup_parts(f.name, f.email, f.when)?, used))
}

/// A parsed signature, borrowed from the parsed bytes: the trimmed name
/// and e-mail, and the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigFields<'a> {
    /// The name, trimmed.
    pub name: &'a [u8],
    /// The e-mail, trimmed.
    pub email: &'a [u8],
    /// The time.
    pub when: GitTime,
}

/// [`parse`] without the copies: the fields borrow `buf`.
pub(crate) fn parse_fields<'a>(
    buf: &'a [u8],
    header: Option<&[u8]>,
    ender: u8,
) -> Result<(SigFields<'a>, usize)> {
    let mut end = buf.len();
    if ender != 0 {
        match memchr::memchr(ender, buf) {
            Some(i) => end = i,
            None => return Err(signature_parse_error("no newline given")),
        }
    }
    let mut start = 0;
    if let Some(h) = header {
        if h.len() >= end || &buf[..h.len()] != h {
            return Err(signature_parse_error(
                "expected prefix doesn't match actual",
            ));
        }
        start = h.len();
    }
    let s = &buf[start..end];
    let lt = memchr::memrchr(b'<', s);
    let gt = memchr::memrchr(b'>', s);
    let (Some(lt), Some(gt)) = (lt, gt) else {
        return Err(signature_parse_error("malformed e-mail"));
    };
    if gt <= lt {
        return Err(signature_parse_error("malformed e-mail"));
    }
    let name = trimmed(&s[..lt]);
    let email = trimmed(&s[lt + 1..gt]);
    let mut when = GitTime::default();

    // Is there a time at the end of the signature? (`email_end + 2 <
    // buffer_end`)
    if gt + 2 < s.len() {
        let t = &s[gt + 2..];
        let (time, consumed) = match gutil::strntol64(t, 10) {
            Ok(v) => v,
            Err(_) => return Err(signature_parse_error("invalid Unix timestamp")),
        };
        when.time = time;
        // A timezone? (`time_end + 1 < buffer_end`)
        if consumed + 1 < t.len() {
            let tz = &t[consumed + 1..];
            let sign = tz[0];
            let offset = if sign != b'-' && sign != b'+' {
                0
            } else {
                // A malformed timezone is taken as 0; strntol32's error
                // stays set, as in C.
                gutil::strntol32(&tz[1..], 10).map_or(0, |(v, _)| v)
            };
            let hours = offset / 100;
            let mins = offset % 100;
            // Only a timezone that does not overflow is stored.
            if hours <= 14 && mins <= 59 {
                // (`hours` may be very negative: C's `int` arithmetic
                // overflows there; rust/DEFECTS.md.)
                let mut off = hours.wrapping_mul(60).wrapping_add(mins);
                if sign == b'-' {
                    off = off.wrapping_neg();
                }
                when.offset = off;
                when.sign = sign as c_char;
            }
        }
    }
    Ok((SigFields { name, email, when }, end + 1))
}

/// The bytes `git_signature__writebuf` appends.
pub(crate) fn format_into(
    out: &mut Vec<u8>,
    header: &[u8],
    name: &[u8],
    email: &[u8],
    when: GitTime,
) {
    let mut tail = [0u8; TAIL_MAX];
    let n = format_tail(&mut tail, when);
    for p in [header, name, b" <", email, b"> ", &tail[..n]] {
        out.extend_from_slice(p);
    }
}

/// The longest `"%u %c%02d%02d\n"` (u32 digits, sign, two `int`s).
const TAIL_MAX: usize = 10 + 2 + 2 * 11 + 1;

/// Append `v`'s decimal digits at `out[n..]` (`%u`; with `width` 2 and a
/// value below 10, a leading 0 as `%02d`); the new length.
fn put_decimal(out: &mut [u8; TAIL_MAX], mut n: usize, v: u32, width: usize) -> usize {
    let mut digits = [0u8; 10];
    let mut d = digits.len();
    let mut m = v;
    loop {
        d -= 1;
        digits[d] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    while digits.len() - d < width {
        d -= 1;
        digits[d] = b'0';
    }
    let k = digits.len() - d;
    out[n..n + k].copy_from_slice(&digits[d..]);
    n += k;
    n
}

/// `"%u %c%02d%02d\n"` of the time: the timestamp truncated to 32 bits,
/// the sign, hours and minutes.
fn format_tail(out: &mut [u8; TAIL_MAX], when: GitTime) -> usize {
    let sign = if when.offset < 0 || when.sign == b'-' as c_char {
        b'-'
    } else {
        b'+'
    };
    // C's `if (offset < 0) offset = -offset`, then `/ 60`, `% 60`. For
    // INT_MIN the negation overflows (undefined behaviour); the C compiled
    // as the reference does uses the magnitude, 2^31, and so does this
    // (rust/DEFECTS.md). Both are then non-negative.
    let magnitude = when.offset.unsigned_abs();
    let mut n = put_decimal(out, 0, when.time as u32, 0);
    out[n] = b' ';
    out[n + 1] = sign;
    n = put_decimal(out, n + 2, magnitude / 60, 2);
    n = put_decimal(out, n, magnitude % 60, 2);
    out[n] = b'\n';
    n + 1
}

/// The length of `"%s%s <%s> %u %c%02d%02d\n"`, for `git_str_printf`'s
/// first reservation.
const WRITEBUF_FORMAT_LEN: usize = 24;

/// `git_signature__writebuf`: append `<header>name <email> time ±hhmm\n`.
/// Fails silently (the string becomes out of memory), as C.
pub(crate) fn writebuf(
    buf: &mut GitStr,
    header: Option<&[u8]>,
    name: &[u8],
    email: &[u8],
    when: GitTime,
) {
    let mut tail = [0u8; TAIL_MAX];
    let n = format_tail(&mut tail, when);
    let pieces: [&[u8]; 6] = [header.unwrap_or(b""), name, b" <", email, b"> ", &tail[..n]];
    let _ = crate::object::printf_pieces(buf, WRITEBUF_FORMAT_LEN, &pieces);
}

/// `strcmp(a, b) == 0` in one pass, reading each string only up to its
/// NUL or the first difference.
///
/// # Safety
///
/// `a` and `b` are NUL-terminated strings.
#[inline]
unsafe fn c_str_eq(a: *const c_char, b: *const c_char) -> bool {
    let mut i = 0;
    loop {
        // `i` is at most the index of either string's NUL: the earlier
        // bytes were equal and not NUL.
        let (pa, pb) = (a.wrapping_add(i), b.wrapping_add(i));
        // SAFETY: `pa` is within `a`, up to its NUL (above).
        let x = unsafe { pa.read() };
        // SAFETY: as above for `b`.
        let y = unsafe { pb.read() };
        if x != y {
            return false;
        }
        if x == 0 {
            return true;
        }
        i += 1;
    }
}

/// glibc's `%s` of NULL.
const NULL_STR: &[u8] = b"(null)";

/// A C string pointer's bytes, glibc's `(null)` for NULL.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string valid for the returned lifetime.
unsafe fn c_bytes<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        NULL_STR
    } else {
        // SAFETY: the caller's contract.
        unsafe { CStr::from_ptr(p) }.to_bytes()
    }
}

/// `git_signature_default`: `user.name` and `user.email` from the
/// repository's configuration, now. `found` runs once both strings are read,
/// before they are validated (where C's `git_signature_new` sets
/// `*sig_out = NULL`).
///
/// # Safety
///
/// `repo` is a live repository.
pub(crate) unsafe fn default_for(
    repo: *mut crate::object::deps::git_repository,
    found: impl FnOnce(),
) -> Result<Signature> {
    // SAFETY: the caller's contract.
    let cfg = unsafe { config_snapshot(repo)? };
    let r = (|| {
        let name = cfg.get_string(c"user.name")?;
        let email = cfg.get_string(c"user.email")?;
        let (now, offset) = current_time();
        found();
        new_sig(name.to_bytes(), email.to_bytes(), now, offset)
    })();
    drop(cfg);
    r
}

/// A configuration snapshot, freed on drop.
struct ConfigSnapshot(*mut crate::object::deps::git_config);

impl Drop for ConfigSnapshot {
    fn drop(&mut self) {
        // SAFETY: the snapshot this value owns, freed once.
        unsafe { crate::object::deps::git_config_free(self.0) }
    }
}

impl ConfigSnapshot {
    /// `git_config_get_string`: a string owned by the snapshot.
    fn get_string(&self, name: &CStr) -> Result<&CStr> {
        let mut out = ptr::null();
        // SAFETY: `self.0` is the live snapshot this value owns until drop;
        // a config is only read through `&` (interior mutability).
        let config = unsafe { &*self.0 };
        // SAFETY: a writable out, a live config, a NUL-terminated name.
        let r = unsafe {
            crate::object::deps::git_config_get_string(
                Out::from(&mut out),
                Some(config),
                name.into(),
            )
        };
        ErrorCode::check(r)?;
        // SAFETY: on success `out` is a non-NULL string of an entry of the
        // read-only snapshot, which lives and is not written while `self`
        // (which this borrows) lives.
        Ok(unsafe { CStr::from_ptr(out) })
    }
}

/// `git_repository_config_snapshot`.
///
/// # Safety
///
/// `repo` is a live repository.
unsafe fn config_snapshot(
    repo: *mut crate::object::deps::git_repository,
) -> Result<ConfigSnapshot> {
    let mut cfg = ptr::null_mut();
    // SAFETY: a live repository (the caller's contract); `cfg` is writable.
    let r = unsafe { crate::object::deps::git_repository_config_snapshot(&raw mut cfg, repo) };
    ErrorCode::check(r)?;
    Ok(ConfigSnapshot(cfg))
}

/// `user_from_env`: a signature from the environment variables, else the
/// configuration (and `$EMAIL` for the e-mail), at `$<date_env_var>` or the
/// default time.
///
/// # Safety
///
/// `repo` is a live repository.
unsafe fn user_from_env(
    repo: *mut crate::object::deps::git_repository,
    name_env_var: &CStr,
    email_env_var: &CStr,
    date_env_var: &CStr,
    default_time: i64,
    default_offset: c_int,
) -> Result<Signature> {
    // SAFETY: the caller's contract.
    let cfg = unsafe { config_snapshot(repo)? };
    let mut name_env = GitStr::new();
    let mut email_env = GitStr::new();
    let mut date_env = GitStr::new();

    let name: &[u8] = if gutil::getenv(&mut name_env, name_env_var).is_ok() {
        name_env.as_bytes()
    } else {
        cfg.get_string(c"user.name")?.to_bytes()
    };

    let email: &[u8] = if gutil::getenv(&mut email_env, email_env_var).is_ok() {
        email_env.as_bytes()
    } else {
        match cfg.get_string(c"user.email") {
            Ok(p) => p.to_bytes(),
            Err(e) if e == ErrorCode::NOT_FOUND => {
                let saved = errors::save();
                if gutil::getenv(&mut email_env, c"EMAIL").is_err() {
                    errors::restore(saved);
                    return Err(ErrorCode::NOT_FOUND);
                }
                drop(saved);
                email_env.as_bytes()
            }
            Err(e) => return Err(e),
        }
    };

    let (timestamp, offset) = if gutil::getenv(&mut date_env, date_env_var).is_ok() {
        let (t, o) = crate::util::date::offset_parse(date_env.as_bytes())?;
        (t, o.unwrap_or(0))
    } else {
        (default_time, default_offset)
    };

    // `git_signature_new` on C strings: they end at a NUL.
    let name = &name[..memchr::memchr(0, name).unwrap_or(name.len())];
    let email = &email[..memchr::memchr(0, email).unwrap_or(email.len())];
    new_sig(name, email, timestamp, offset)
}

/// `git_signature_default_from_env`: the author and committer signatures
/// (`GIT_AUTHOR_*`, `GIT_COMMITTER_*`, else the configuration), for the
/// ones asked.
///
/// # Safety
///
/// `repo` is a live repository.
pub(crate) unsafe fn default_from_env(
    repo: *mut crate::object::deps::git_repository,
    author: bool,
    committer: bool,
) -> Result<(Option<Signature>, Option<Signature>)> {
    let (now, offset) = current_time();
    let a = if author {
        // SAFETY: the caller's contract.
        Some(unsafe {
            user_from_env(
                repo,
                c"GIT_AUTHOR_NAME",
                c"GIT_AUTHOR_EMAIL",
                c"GIT_AUTHOR_DATE",
                now,
                offset,
            )?
        })
    } else {
        None
    };
    let c = if committer {
        // SAFETY: the caller's contract.
        Some(unsafe {
            user_from_env(
                repo,
                c"GIT_COMMITTER_NAME",
                c"GIT_COMMITTER_EMAIL",
                c"GIT_COMMITTER_DATE",
                now,
                offset,
            )?
        })
    } else {
        None
    };
    Ok((a, c))
}

/// `git_signature_free` of a block C or Rust made.
///
/// # Safety
///
/// `sig` is NULL or a `git__allocator` block holding a `git_signature`
/// whose strings are NULL or `git__allocator` strings, none used again.
pub(crate) unsafe fn free_raw(sig: *mut GitSignature) {
    if sig.is_null() {
        return;
    }
    // Only the two pointers are read: C may have set nothing else.
    // SAFETY: the caller's contract: a live block (no reference formed).
    let pn = unsafe { &raw const (*sig).name };
    // SAFETY: as above.
    let pe = unsafe { &raw const (*sig).email };
    // SAFETY: a field the caller set (NULL or a string).
    let name = unsafe { pn.read() };
    // SAFETY: as above.
    let email = unsafe { pe.read() };
    // SAFETY: the strings and the block, each freed once.
    unsafe {
        alloc::free(name.cast());
    }
    // SAFETY: as above.
    unsafe { alloc::free(email.cast()) };
    // SAFETY: as above.
    unsafe { alloc::free(sig.cast()) };
}

/// A `git__allocator` `git_signature` holding copies of `f`'s strings, as
/// `git_signature__parse` fills one (freed with [`free_raw`]).
pub fn alloc_raw(f: &SigFields<'_>) -> Result<*mut GitSignature> {
    dup_parts(f.name, f.email, f.when)?
        .into_raw()
        .map_err(|_| crate::util::errors::oom())
}

/// The exports of `signature.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::CStrArg;
    use crate::util::export::{barrier, status};
    use crate::util::pool::GitPool;

    /// Write `sig` into C's out-parameter as a `git__allocator` block.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    unsafe fn give(out: *mut *mut GitSignature, sig: Signature) -> c_int {
        match sig.into_raw() {
            Ok(p) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(p) };
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_signature_free` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `sig` is NULL or a signature libgit2 allocated (or a `git__malloc`
    /// block whose `name` and `email` are NULL or `git__allocator`
    /// strings), not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_free(sig: *mut GitSignature) {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe { free_raw(sig) }
        })
    }

    /// `git_signature_new`.
    ///
    /// # Safety
    ///
    /// `sig_out` is writable; `name` and `email` are NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_new(
        sig_out: *mut *mut GitSignature,
        name: CStrArg<'_>,
        email: CStrArg<'_>,
        time: git_time_t,
        offset: c_int,
    ) -> c_int {
        barrier(|| {
            let name = git_arg!(name.get(), "name", -1);
            let email = git_arg!(email.get(), "email", -1);
            // SAFETY: the caller's contract.
            unsafe { sig_out.write(ptr::null_mut()) };
            let name = name.to_bytes();
            let email = email.to_bytes();
            match new_sig(name, email, time, offset) {
                // SAFETY: writable, as above.
                Ok(s) => unsafe { give(sig_out, s) },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_signature_now`.
    ///
    /// # Safety
    ///
    /// As [`git_signature_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_now(
        sig_out: *mut *mut GitSignature,
        name: CStrArg<'_>,
        email: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let (now, offset) = current_time();
            // SAFETY: forwarded.
            unsafe { git_signature_new(sig_out, name, email, now, offset) }
        })
    }

    /// `git_signature_dup` (`*dest` untouched when `source` is NULL).
    ///
    /// # Safety
    ///
    /// `dest` is writable; `source` is NULL or a valid signature whose
    /// strings are NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_dup(
        mut dest: Out<'_, *mut GitSignature>,
        source: *const GitSignature,
    ) -> c_int {
        barrier(|| {
            if source.is_null() {
                return 0;
            }
            // SAFETY: a valid signature, read by value.
            let s = unsafe { source.read() };
            // C's `git__strdup` of a NULL string crashes; a NULL string is
            // copied as NULL (rust/DEFECTS.md).
            let p = alloc::calloc(1, size_of::<GitSignature>()).cast::<GitSignature>();
            if p.is_null() {
                return -1;
            }
            let copy = |c: *mut c_char| -> core::result::Result<*mut c_char, ()> {
                if c.is_null() {
                    return Ok(ptr::null_mut());
                }
                // SAFETY: a NUL-terminated string (the caller's contract).
                let r = alloc::strdup(unsafe { CStr::from_ptr(c) });
                if r.is_null() { Err(()) } else { Ok(r) }
            };
            let Ok(name) = copy(s.name) else {
                // SAFETY: the block just allocated, unused (C leaks it).
                unsafe { alloc::free(p.cast()) };
                return -1;
            };
            let Ok(email) = copy(s.email) else {
                // SAFETY: as above, and the name copied into it.
                unsafe {
                    alloc::free(name.cast());
                }
                // SAFETY: as above.
                unsafe { alloc::free(p.cast()) };
                return -1;
            };
            // SAFETY: a fresh block for a `GitSignature`.
            unsafe {
                p.write(GitSignature {
                    name,
                    email,
                    when: s.when,
                })
            };
            dest.write(p);
            0
        })
    }

    /// `git_signature__pdup`: a copy in `pool` (`*dest` untouched when
    /// `source` is NULL).
    ///
    /// # Safety
    ///
    /// `dest` is writable, `pool` a valid pool of 1-byte items not
    /// otherwise accessed during the call, and `source` NULL or a valid
    /// signature with NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature__pdup(
        mut dest: Out<'_, *mut GitSignature>,
        source: *const GitSignature,
        pool: *mut GitPool,
    ) -> c_int {
        barrier(|| {
            if source.is_null() {
                return 0;
            }
            // SAFETY: a valid pool, used only here during the call.
            let pool = unsafe { &mut *pool };
            // SAFETY: a valid signature, read by value.
            let s = unsafe { source.read() };
            let r = (|| -> Result<*mut GitSignature> {
                let sig = pool
                    .mallocz(size_of::<GitSignature>())?
                    .cast::<GitSignature>();
                // SAFETY: NUL-terminated strings (the caller's contract).
                let name = pool.strdup(unsafe { CStr::from_ptr(s.name) })?;
                // SAFETY: as above.
                let email = pool.strdup(unsafe { CStr::from_ptr(s.email) })?;
                // SAFETY: the pool's zeroed, 8-aligned block for one
                // `GitSignature`.
                unsafe {
                    sig.as_ptr().write(GitSignature {
                        name: name.as_ptr(),
                        email: email.as_ptr(),
                        when: s.when,
                    })
                };
                Ok(sig.as_ptr())
            })();
            match r {
                Ok(p) => {
                    dest.write(p);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_signature_default`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `repo` is a live repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_default(
        out: *mut *mut GitSignature,
        repo: *mut crate::object::deps::git_repository,
    ) -> c_int {
        barrier(|| {
            // C's `git_signature_new` sets `*out = NULL` once the identity
            // is read, before validating it.
            // SAFETY: `out` is writable (the caller's contract).
            let found = || unsafe { out.write(ptr::null_mut()) };
            // SAFETY: the caller's contract.
            match unsafe { default_for(repo, found) } {
                // SAFETY: writable.
                Ok(s) => unsafe { give(out, s) },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_signature_default_from_env`.
    ///
    /// # Safety
    ///
    /// `author_out` and `committer_out` are NULL or writable; `repo` is a
    /// live repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_default_from_env(
        author_out: *mut *mut GitSignature,
        committer_out: *mut *mut GitSignature,
        repo: *mut crate::object::deps::git_repository,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !author_out.is_null() || !committer_out.is_null(),
                "author_out || committer_out",
                -1
            );
            git_assert_arg!(!repo.is_null(), "repo", -1);
            // SAFETY: the caller's contract.
            let r =
                unsafe { default_from_env(repo, !author_out.is_null(), !committer_out.is_null()) };
            let (a, c) = match r {
                Ok(v) => v,
                Err(e) => return e.raw(),
            };
            let a = match a.map(Signature::into_raw).transpose() {
                Ok(a) => a,
                Err(e) => return e.raw(),
            };
            let c = match c.map(Signature::into_raw).transpose() {
                Ok(c) => c,
                Err(e) => {
                    // SAFETY: the block just made, not handed out.
                    unsafe { free_raw(a.unwrap_or(ptr::null_mut())) };
                    return e.raw();
                }
            };
            if let Some(a) = a {
                // SAFETY: non-NULL, so writable.
                unsafe { author_out.write(a) };
            }
            if let Some(c) = c {
                // SAFETY: as above.
                unsafe { committer_out.write(c) };
            }
            0
        })
    }

    /// `git_signature__parse`: parse a signature at `*buffer_out` (up to
    /// `buffer_end`, and to the first `ender` when it is not 0) into `sig`,
    /// and advance `*buffer_out` past it.
    ///
    /// # Safety
    ///
    /// `sig` is writable (its old contents are overwritten, not freed);
    /// `*buffer_out` .. `buffer_end` is a readable range; `header` is NULL
    /// or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature__parse(
        mut sig: Out<'_, GitSignature>,
        buffer_out: *mut *const c_char,
        buffer_end: *const c_char,
        header: CStrArg<'_>,
        ender: c_char,
    ) -> c_int {
        barrier(|| {
            sig.write(GitSignature::ZERO);
            // SAFETY: readable.
            let start = unsafe { *buffer_out };
            // An end before the start (C's `memchr` with a huge length, UB)
            // is an empty range: rust/DEFECTS.md.
            let len = (buffer_end as usize).saturating_sub(start as usize);
            // SAFETY: the caller's readable range.
            let buf = unsafe { crate::util::export::bytes(start, len) };
            let header = header.get().map(CStr::to_bytes);
            match parse(buf, header, ender as u8) {
                Ok((s, consumed)) => {
                    let s = core::mem::ManuallyDrop::new(s);
                    // The strings move into it.
                    sig.write(GitSignature {
                        name: s.0.name,
                        email: s.0.email,
                        when: s.0.when,
                    });
                    // SAFETY: writable; C's `buffer_end + 1` (not
                    // dereferenced).
                    unsafe { buffer_out.write(start.wrapping_add(consumed)) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_signature_from_buffer`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `buf` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature_from_buffer(
        out: *mut *mut GitSignature,
        buf: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let buf = git_arg!(buf.get(), "buf", -1);
            // SAFETY: writable.
            unsafe { out.write(ptr::null_mut()) };
            let b = buf.to_bytes();
            match parse(b, None, 0) {
                // SAFETY: writable.
                Ok((s, _)) => unsafe { give(out, s) },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_signature__writebuf`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str`; `header` is NULL or NUL-terminated; `sig`
    /// is a valid signature (NULL strings print as `(null)`, as glibc's
    /// `printf`), none of them in `buf`'s buffer.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature__writebuf(
        buf: *mut GitStr,
        header: CStrArg<'_>,
        sig: *const GitSignature,
    ) {
        barrier(|| {
            // SAFETY: a valid signature, read by value.
            let s = unsafe { sig.read() };
            // SAFETY: NULL or a NUL-terminated string, valid for the call.
            let name = unsafe { c_bytes(s.name) };
            // SAFETY: as above.
            let email = unsafe { c_bytes(s.email) };
            let header = header.get().map(CStr::to_bytes);
            // The text is formatted first, so `buf` may alias nothing that
            // matters; `&mut` covers the `git_str` struct only.
            // SAFETY: a valid `git_str`, not otherwise accessed.
            writebuf(unsafe { &mut *buf }, header, name, email, s.when);
        })
    }

    /// `git_signature__equal` (NULL arguments: `true`, as C's
    /// `GIT_ASSERT_ARG` returns -1 from a `bool` function).
    ///
    /// # Safety
    ///
    /// `one` and `two` are NULL or valid signatures with NULL or
    /// NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_signature__equal(
        one: *const GitSignature,
        two: *const GitSignature,
    ) -> bool {
        barrier(|| {
            git_assert_arg!(!one.is_null(), "one", true);
            git_assert_arg!(!two.is_null(), "two", true);
            // SAFETY: a valid signature, read by value.
            let a = unsafe { one.read() };
            // SAFETY: as above.
            let b = unsafe { two.read() };
            // `git__strcmp` (C crashes on NULL: here NULL equals only NULL).
            let same = |x: *mut c_char, y: *mut c_char| {
                if x.is_null() || y.is_null() {
                    return x.is_null() && y.is_null();
                }
                // SAFETY: NUL-terminated strings (the caller's contract).
                unsafe { c_str_eq(x, y) }
            };
            same(a.name, b.name)
                && same(a.email, b.email)
                && a.when.time == b.when.time
                && a.when.offset == b.when.offset
                && a.when.sign == b.when.sign
        })
    }

    /// Unused: keeps `status` imported for symmetry with other modules.
    #[allow(dead_code)]
    fn _status(r: Result<()>) -> c_int {
        status(r)
    }
}

crate::c_layout_test! {
    headers: ["git2/signature.h"],
    types: [
        GitSignature => "git_signature" { name, email, when },
        GitTime => "git_time" { time, offset, sign },
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::cabi::CStrArg;
    use crate::util::test_support;

    /// A parse's name, e-mail, time and length, or its code and message.
    type Parsed = core::result::Result<(Vec<u8>, Vec<u8>, GitTime, usize), (i32, Vec<u8>)>;

    fn p(s: &[u8], h: Option<&[u8]>, e: u8) -> Parsed {
        match parse(s, h, e) {
            Ok((sig, n)) => Ok((sig.name().to_vec(), sig.email().to_vec(), sig.0.when, n)),
            Err(c) => Err((c.raw(), errors::last_message().unwrap_or_default())),
        }
    }

    #[test]
    fn parses_like_c() {
        let _g = test_support::init();
        let (n, e, w, c) = p(
            b"author Vicent Marti <tanoku@gmail.com> 12345 +0200\nrest",
            Some(b"author "),
            b'\n',
        )
        .unwrap();
        assert_eq!(n, b"Vicent Marti");
        assert_eq!(e, b"tanoku@gmail.com");
        assert_eq!(
            w,
            GitTime {
                time: 12345,
                offset: 120,
                sign: b'+' as c_char
            }
        );
        assert_eq!(c, 51);
        let (_, _, w, _) = p(b"a <b> 1 -0130\n", None, b'\n').unwrap();
        assert_eq!((w.offset, w.sign), (-90, b'-' as c_char));
        // Out-of-range timezone: 0 and no sign.
        let (_, _, w, _) = p(b"a <b> 1 +1500\n", None, b'\n').unwrap();
        assert_eq!((w.offset, w.sign), (0, 0));
        // Malformed timezone: 0, with the sign.
        let (_, _, w, _) = p(b"a <b> 1 +xx\n", None, b'\n').unwrap();
        assert_eq!((w.offset, w.sign), (0, b'+' as c_char));
        // No time.
        let (n, e, w, c) = p(b" <> ", None, 0).unwrap();
        assert_eq!(
            (n.as_slice(), e.as_slice(), w, c),
            (&b""[..], &b""[..], GitTime::default(), 5)
        );
        assert_eq!(p(b"a b>", None, 0).unwrap_err().0, -21);
        assert_eq!(
            p(b"a <b> x", None, 0).unwrap_err(),
            (
                -21,
                b"failed to parse signature - invalid Unix timestamp".to_vec()
            )
        );
        assert_eq!(
            p(b"a <b> 1", None, b'\n').unwrap_err(),
            (
                -21,
                b"failed to parse signature - no newline given".to_vec()
            )
        );
        assert_eq!(
            p(b"author \n", Some(b"author "), b'\n').unwrap_err(),
            (
                -21,
                b"failed to parse signature - expected prefix doesn't match actual".to_vec()
            )
        );
        assert_eq!(p(b"a >b< 1", None, 0).unwrap_err().0, -21);
    }

    #[test]
    fn c_strings_compare_like_strcmp() {
        let eq = |a: &CStr, b: &CStr| {
            // SAFETY: NUL-terminated strings.
            unsafe { c_str_eq(a.as_ptr(), b.as_ptr()) }
        };
        assert!(eq(c"", c""));
        assert!(eq(c"abc", c"abc"));
        assert!(!eq(c"ab", c"abc"));
        assert!(!eq(c"abc", c"ab"));
        assert!(!eq(c"abd", c"abc"));
        assert!(!eq(c"\xff", c"\xfe"));
    }

    #[test]
    fn new_trims_and_validates() {
        let _g = test_support::init();
        let s = Signature::new(b"  ,Name; ", b"\t<x@y>", Time::new(5, -60));
        assert!(s.is_err());
        let s = Signature::new(b"  ,Name; ", b"\tx@y'", Time::new(5, -60)).unwrap();
        assert_eq!(s.name(), b"Name");
        assert_eq!(s.email(), b"x@y");
        assert_eq!(s.when().sign(), b'-');
        assert!(Signature::new(b" ", b"x", Time::new(0, 0)).is_err());
        assert_eq!(s.clone(), s);
        assert_eq!(s.to_header(b"tagger "), b"tagger Name <x@y> 5 -0100\n");
    }

    #[test]
    fn writebuf_formats_like_printf() {
        let _g = test_support::init();
        let mut v = Vec::new();
        format_into(
            &mut v,
            b"",
            b"n",
            b"e",
            GitTime {
                time: -1,
                offset: i32::MIN,
                sign: b'+' as c_char,
            },
        );
        assert_eq!(v, b"n <e> 4294967295 -3579139408\n");
        let mut s = GitStr::new();
        writebuf(
            &mut s,
            Some(b"h "),
            b"\xff",
            b"e",
            GitTime {
                time: 1,
                offset: 5,
                sign: 0,
            },
        );
        assert_eq!(s.as_bytes(), b"h \xff <e> 1 +0005\n");
    }

    #[test]
    fn ffi_roundtrip() {
        let _g = test_support::init();
        let mut out = ptr::null_mut();
        // SAFETY: valid arguments.
        let r = unsafe { ffi::git_signature_new(&raw mut out, c" a ".into(), c"b".into(), 3, 0) };
        assert_eq!(r, 0);
        let mut d = ptr::null_mut();
        // SAFETY: a valid signature.
        assert_eq!(unsafe { ffi::git_signature_dup(Out::from(&mut d), out) }, 0);
        // SAFETY: two valid signatures.
        assert!(unsafe { ffi::git_signature__equal(out, d) });
        // SAFETY: NULL arguments are allowed (and answer true).
        assert!(unsafe { ffi::git_signature__equal(ptr::null(), d) });
        // SAFETY: the block made above, freed once.
        unsafe { ffi::git_signature_free(out) };
        // SAFETY: as above.
        unsafe { ffi::git_signature_free(d) };
        let mut sig = GitSignature::ZERO;
        let text = b"committer a <b> 1 +0100\nx";
        let mut cur = text.as_ptr().cast::<c_char>();
        // SAFETY: a readable range and writable outputs.
        let r = unsafe {
            ffi::git_signature__parse(
                Out::from(&mut sig),
                &raw mut cur,
                text.as_ptr().wrapping_add(text.len()).cast(),
                c"committer ".into(),
                b'\n' as c_char,
            )
        };
        assert_eq!(r, 0);
        assert_eq!(cur as usize - text.as_ptr() as usize, 24);
        let mut b = GitStr::new();
        // SAFETY: valid arguments.
        unsafe { ffi::git_signature__writebuf(&raw mut b, CStrArg::NULL, &raw const sig) };
        assert_eq!(b.as_bytes(), b"a <b> 1 +0100\n");
        let raw = alloc::malloc(size_of::<GitSignature>()).cast::<GitSignature>();
        // SAFETY: a fresh block; the parsed signature's strings move in.
        unsafe { raw.write(sig) };
        // SAFETY: as C frees a parsed signature.
        unsafe { ffi::git_signature_free(raw) };
    }
}
