//! Port of `src/util/regexp.c` for this configuration's PCRE2 backend
//! (`GIT_REGEX_PCRE2`): a safe binding over the system `libpcre2-8`, so the
//! semantics are PCRE2's own.
//!
//! `git_regexp` is a `pcre2_code *` that C that stays holds by value
//! (diff drivers, revparse, config): [`Regexp`] owns one, and the exports
//! hand its pointer to C and take it back (`git_regexp_dispose`).

use core::ffi::{CStr, c_char, c_int};
use core::mem::MaybeUninit;
use core::ops::Range;

use super::errors::{self, ErrorClass, ErrorCode, Result};
use crate::git_assert;
use crate::sys::pcre2::{self, CodeBox, CodeRef, PCRE2_CASELESS, PCRE2_ERROR_NOMATCH, PCRE2_UNSET};

/// `GIT_REGEXP_ICASE`.
pub(crate) const GIT_REGEXP_ICASE: c_int = 1 << 0;

/// Options of [`Regexp::new`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegexpFlags {
    /// Match letters of either case (`GIT_REGEXP_ICASE`, PCRE2's
    /// `PCRE2_CASELESS`).
    pub ignore_case: bool,
}

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
/// `git_regexp_search` on a borrowed pattern, for `n` groups: on success,
/// `set(i, m)` is called for every `i < n` (C's output array may be
/// uninitialized, so the export writes through `MaybeUninit`).
fn search_code(
    code: Option<CodeRef<'_>>,
    s: &[u8],
    n: usize,
    mut set: impl FnMut(usize, RegMatch),
) -> Result<()> {
    git_assert!(n <= u32::MAX as usize, "given_nmatches <= UINT32_MAX");
    let nmatches = n as u32;
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
    if captures == 0 || captures > n {
        captures = n;
    }
    let ovec = pcre2::ovector(&md);
    let off = |o: usize| if o == PCRE2_UNSET { -1 } else { o as isize };
    for i in 0..n {
        set(
            i,
            if i < captures {
                RegMatch {
                    start: off(ovec[2 * i]),
                    end: off(ovec[2 * i + 1]),
                }
            } else {
                RegMatch { start: -1, end: -1 }
            },
        );
    }
    Ok(())
}

impl Regexp {
    /// `git_regexp_compile`: `pattern` up to its first NUL (C's
    /// `PCRE2_ZERO_TERMINATED`). A bad pattern fails with
    /// `GIT_EINVALIDSPEC` and PCRE2's message (class `GIT_ERROR_REGEX`).
    pub fn new(pattern: &[u8], flags: RegexpFlags) -> Result<Regexp> {
        let flags = if flags.ignore_case {
            GIT_REGEXP_ICASE
        } else {
            0
        };
        let end = memchr::memchr(0, pattern).unwrap_or(pattern.len());
        let mut z = Vec::new();
        z.try_reserve_exact(end + 1).map_err(|_| errors::oom())?;
        z.extend_from_slice(&pattern[..end]);
        z.push(0);
        let pattern = CStr::from_bytes_with_nul(&z).map_err(|_| ErrorCode::INVALID_SPEC)?;
        Regexp::compile_opt(Some(pattern), flags)
    }

    /// As [`new`](Self::new), from a C string.
    pub fn compile(pattern: &CStr, flags: RegexpFlags) -> Result<Regexp> {
        let flags = if flags.ignore_case {
            GIT_REGEXP_ICASE
        } else {
            0
        };
        Regexp::compile_opt(Some(pattern), flags)
    }

    /// `git_regexp_compile`, also of a NULL pattern (PCRE2's error).
    pub(crate) fn compile_opt(pattern: Option<&CStr>, flags: c_int) -> Result<Regexp> {
        let options = if flags & GIT_REGEXP_ICASE != 0 {
            PCRE2_CASELESS
        } else {
            0
        };
        match pcre2::compile(pattern, options) {
            Ok(code) => Ok(Regexp(code)),
            Err(error) => {
                // On the stack, as C's.
                let mut buf = [0u8; 1024];
                let _ = errors::set_str(ErrorClass::REGEX, pcre2::error_message(error, &mut buf));
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

    /// `git_regexp_search` for `n` groups: the byte ranges of the whole
    /// match and of the first `n - 1` groups (`None` for the groups that
    /// did not match, and those beyond the pattern's). `GIT_ENOTFOUND` if
    /// `s` does not match.
    pub fn search(&self, s: &[u8], n: usize) -> Result<Vec<Option<Range<usize>>>> {
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| errors::oom())?;
        search_code(Some(self.0.as_ref()), s, n, |_, m| {
            v.push(match (usize::try_from(m.start), usize::try_from(m.end)) {
                (Ok(a), Ok(b)) => Some(a..b),
                _ => None,
            })
        })?;
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
            // SAFETY: the caller passes a NUL-terminated pattern (or NULL,
            // which PCRE2 rejects, as for the C).
            let pattern = unsafe { cstr(pattern) };
            let (code, rc) = match Regexp::compile_opt(pattern, flags) {
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
        // SAFETY: `r` is a readable `git_regexp` (the caller's contract).
        let code = unsafe { r.read() };
        // SAFETY: the caller's contract on the pattern.
        unsafe { pcre2::code_ref(code) }
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
            let code = unsafe { code(r) };
            // SAFETY: the caller's contract.
            let s = unsafe { cstr_bytes(string) };
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
            let code = unsafe { code(r) };
            // SAFETY: the caller's contract.
            let s = unsafe { cstr_bytes(string) };
            let m: &mut [MaybeUninit<RegMatch>] = if nmatches == 0 || matches.is_null() {
                &mut []
            } else {
                // SAFETY: the caller passes `nmatches` writable entries
                // (maybe uninitialized: they are only written).
                unsafe {
                    core::slice::from_raw_parts_mut(
                        matches.cast::<MaybeUninit<RegMatch>>(),
                        nmatches,
                    )
                }
            };
            status(search_code(code, s, m.len(), |i, v| {
                m[i].write(v);
            }))
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
