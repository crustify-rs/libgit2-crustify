//! Port of `src/util/regexp.c` for this configuration's PCRE2 backend
//! (`GIT_REGEX_PCRE2`): a safe binding over the system `libpcre2-8`, so the
//! semantics are PCRE2's own.
//!
//! `git_regexp` is a `pcre2_code *` that C that stays holds by value
//! (diff drivers, revparse, config): [`Regexp`] owns one, and the exports
//! hand its pointer to C and take it back (`git_regexp_dispose`).

use core::ffi::{CStr, c_char, c_int};

use super::errors::{self, ErrorClass, ErrorCode, Result};
use crate::git_assert;
use crate::sys::pcre2::{self, CodeBox, CodeRef, PCRE2_CASELESS, PCRE2_ERROR_NOMATCH, PCRE2_UNSET};

/// `GIT_REGEXP_ICASE`.
pub const GIT_REGEXP_ICASE: c_int = 1 << 0;

/// `git_regmatch`, mirrored: a group's byte offsets, -1 when it did not
/// match.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegMatch {
    /// `start`
    pub start: isize,
    /// `end`
    pub end: isize,
}

/// A compiled regular expression (`git_regexp`).
#[derive(Debug)]
pub struct Regexp(CodeBox);

/// PCRE2's result code as the C's: no match is `GIT_ENOTFOUND`, any other
/// error `GIT_EINVALIDSPEC` (without a message, as the C).
fn match_error(rc: c_int) -> ErrorCode {
    if rc == PCRE2_ERROR_NOMATCH {
        ErrorCode::NOT_FOUND
    } else {
        ErrorCode::INVALID_SPEC
    }
}

/// `git_regexp_match` on a borrowed pattern.
fn match_code(code: Option<CodeRef<'_>>, s: &[u8]) -> Result<()> {
    let Some(mut md) = pcre2::match_data_create(1) else {
        return Err(errors::oom());
    };
    // A NULL pattern is PCRE2_ERROR_NULL to `pcre2_match`.
    let Some(code) = code else {
        return Err(ErrorCode::INVALID_SPEC);
    };
    let rc = pcre2::pcre2_match(code, s, &mut md);
    if rc < 0 { Err(match_error(rc)) } else { Ok(()) }
}

/// `git_regexp_search` on a borrowed pattern.
fn search_code(code: Option<CodeRef<'_>>, s: &[u8], matches: &mut [RegMatch]) -> Result<()> {
    git_assert!(matches.len() <= u32::MAX as usize, "given_nmatches <= UINT32_MAX");
    let nmatches = matches.len() as u32;
    let Some(mut md) = pcre2::match_data_create(nmatches) else {
        return Err(errors::oom());
    };
    let Some(code) = code else {
        return Err(ErrorCode::INVALID_SPEC);
    };
    let rc = pcre2::pcre2_match(code, s, &mut md);
    if rc < 0 {
        return Err(match_error(rc));
    }
    let mut captures = rc as usize;
    if captures == 0 || captures > matches.len() {
        captures = matches.len();
    }
    let ovec = pcre2::ovector(&md);
    let off = |o: usize| if o == PCRE2_UNSET { -1 } else { o as isize };
    for (i, m) in matches.iter_mut().enumerate() {
        if i < captures {
            m.start = off(ovec[2 * i]);
            m.end = off(ovec[2 * i + 1]);
        } else {
            *m = RegMatch { start: -1, end: -1 };
        }
    }
    Ok(())
}

impl Regexp {
    /// `git_regexp_compile`: `pattern` up to its first NUL (C's
    /// `PCRE2_ZERO_TERMINATED`), with `GIT_REGEXP_ICASE` in `flags` for a
    /// caseless match. A bad pattern fails with `GIT_EINVALIDSPEC` and
    /// PCRE2's message (class `GIT_ERROR_REGEX`).
    pub fn new(pattern: &[u8], flags: c_int) -> Result<Regexp> {
        let end = memchr::memchr(0, pattern).unwrap_or(pattern.len());
        let mut z = Vec::new();
        z.try_reserve_exact(end + 1).map_err(|_| errors::oom())?;
        z.extend_from_slice(&pattern[..end]);
        z.push(0);
        let pattern = CStr::from_bytes_with_nul(&z).map_err(|_| ErrorCode::INVALID_SPEC)?;
        Regexp::compile(pattern, flags)
    }

    /// As [`new`](Self::new), from a C string.
    pub fn compile(pattern: &CStr, flags: c_int) -> Result<Regexp> {
        let options = if flags & GIT_REGEXP_ICASE != 0 {
            PCRE2_CASELESS
        } else {
            0
        };
        match pcre2::compile(pattern, options) {
            Ok(code) => Ok(Regexp(code)),
            Err(error) => {
                let msg = pcre2::error_message::<1024>(error);
                let _ = errors::set_str(ErrorClass::REGEX, msg);
                Err(ErrorCode::INVALID_SPEC)
            }
        }
    }

    /// `git_regexp_match`: `Ok` if `s` matches, `GIT_ENOTFOUND` if not,
    /// `GIT_EINVALIDSPEC` if PCRE2 failed.
    pub fn matches(&self, s: &[u8]) -> Result<()> {
        match_code(Some(self.0.as_ref()), s)
    }

    /// Whether `s` matches; PCRE2's errors other than no-match are errors.
    pub fn is_match(&self, s: &[u8]) -> Result<bool> {
        match self.matches(s) {
            Ok(()) => Ok(true),
            Err(e) if e == ErrorCode::NOT_FOUND => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `git_regexp_search`: fill `matches` with the offsets of the whole
    /// match and of the first groups (-1 for the groups that did not
    /// match, and those beyond the pattern's).
    pub fn search_into(&self, s: &[u8], matches: &mut [RegMatch]) -> Result<()> {
        search_code(Some(self.0.as_ref()), s, matches)
    }

    /// [`search_into`](Self::search_into) `n` groups.
    pub fn search(&self, s: &[u8], n: usize) -> Result<Vec<RegMatch>> {
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| errors::oom())?;
        v.resize(n, RegMatch::default());
        self.search_into(s, &mut v)?;
        Ok(v)
    }
}

pub mod ffi {
    use super::*;
    use crate::sys::pcre2::raw::pcre2_code_8;
    use crate::util::export::{barrier, cstr, cstr_bytes, status};

    /// `git_regexp`.
    pub type GitRegexp = *mut pcre2_code_8;

    /// `git_regexp_compile`.
    ///
    /// # Safety
    ///
    /// `r` is writable; `pattern` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_regexp_compile(
        r: *mut GitRegexp,
        pattern: *const c_char,
        flags: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a NUL-terminated pattern.
            let Some(pattern) = (unsafe { cstr(pattern) }) else {
                return ErrorCode::INVALID_SPEC.raw();
            };
            let (code, rc) = match Regexp::compile(pattern, flags) {
                Ok(re) => (re.0.into_c(), 0),
                Err(e) => (core::ptr::null_mut(), e.raw()),
            };
            // SAFETY: `r` is writable (the caller's contract).
            unsafe { r.write(code) };
            rc
        })
    }

    /// `git_regexp_dispose`: free the pattern and set `*r` to NULL.
    ///
    /// # Safety
    ///
    /// `*r` is NULL or a pattern from `git_regexp_compile` that is not in
    /// use.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_regexp_dispose(r: *mut GitRegexp) {
        barrier(|| {
            // SAFETY: `r` is valid (the caller's contract).
            let code = unsafe { r.replace(core::ptr::null_mut()) };
            // SAFETY: `code` is NULL or an owned compiled pattern, freed
            // once: C's copy is cleared.
            drop(unsafe { CodeBox::from_c(code) });
        })
    }

    /// # Safety
    ///
    /// `*r` is NULL or a live pattern from `git_regexp_compile`, not freed
    /// during `'a`.
    unsafe fn code<'a>(r: *const GitRegexp) -> Option<CodeRef<'a>> {
        // SAFETY: the caller's contract.
        unsafe { pcre2::code_ref(r.read()) }
    }

    /// `git_regexp_match`.
    ///
    /// # Safety
    ///
    /// `r` points to a `git_regexp` that is NULL or compiled, and not
    /// disposed during the call; `string` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_regexp_match(r: *const GitRegexp, string: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let (code, s) = unsafe { (code(r), cstr_bytes(string)) };
            status(match_code(code, s))
        })
    }

    /// `git_regexp_search`.
    ///
    /// # Safety
    ///
    /// As `git_regexp_match`; `matches` is writable for `nmatches`
    /// entries (or `nmatches` is 0).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_regexp_search(
        r: *const GitRegexp,
        string: *const c_char,
        nmatches: usize,
        matches: *mut RegMatch,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let (code, s) = unsafe { (code(r), cstr_bytes(string)) };
            let m: &mut [RegMatch] = if nmatches == 0 || matches.is_null() {
                &mut []
            } else {
                // SAFETY: the caller passes `nmatches` writable entries.
                unsafe { core::slice::from_raw_parts_mut(matches, nmatches) }
            };
            status(search_code(code, s, m))
        })
    }
}

crate::c_layout_test! {
    headers: ["regexp.h"],
    types: [
        RegMatch => "git_regmatch" { start, end },
        ffi::GitRegexp => "git_regexp" {},
    ],
    asserts: ["GIT_REGEXP_ICASE == 1"],
}
