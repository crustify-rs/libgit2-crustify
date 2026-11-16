//! Port of `src/util/wildmatch.c`: shell-style pattern matching with `?`,
//! `\`, `[]`, `*` and `**` (git's `wildmatch`), 8-bit clean.
//!
//! The pattern and the text are byte strings that end at their first NUL
//! (as C's do) or at their end: [`wildmatch`] reads a byte past the end of
//! either as NUL.
//!
//! Character classes (`[:alpha:]`, …) are the C locale's, restricted to
//! ASCII (the C's `ISASCII(c) && isalpha(c)`, …).

use core::cell::Cell;
use core::ffi::{c_char, c_int, c_uint};
use core::marker::PhantomData;

/// `WM_CASEFOLD`
pub const WM_CASEFOLD: c_uint = 1;
/// `WM_PATHNAME`
pub const WM_PATHNAME: c_uint = 2;

/// `WM_NOMATCH`
pub const WM_NOMATCH: c_int = 1;
/// `WM_MATCH`
pub const WM_MATCH: c_int = 0;
/// `WM_ABORT_ALL`
pub const WM_ABORT_ALL: c_int = -1;
/// `WM_ABORT_TO_STARSTAR`
pub const WM_ABORT_TO_STARSTAR: c_int = -2;

const NEGATE_CLASS: u8 = b'!';
const NEGATE_CLASS2: u8 = b'^';

/// `is_glob_special`: `*`, `?`, `[`, `\`.
#[inline]
fn is_glob_special(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'[' | b'\\')
}

/// A string `dowild` reads: byte `i`, NUL at and past the end.
trait Src {
    fn at(&self, i: usize) -> u8;

    /// `strchr(s + i, c)` for a non-NUL `c`, with `i` at most the end: the
    /// index of `c` at or after `i`.
    #[inline]
    fn find_from(&self, mut i: usize, c: u8) -> Option<usize> {
        loop {
            match self.at(i) {
                0 => return None,
                x if x == c => return Some(i),
                _ => i += 1,
            }
        }
    }

    /// C's scan after `*literal`: the first index at or after `t` that
    /// holds `lit` (or, when `fold`, `up`), the end, or (without
    /// `match_slash`) a slash.
    #[inline]
    fn scan(&self, mut t: usize, lit: u8, up: u8, fold: bool, match_slash: bool) -> usize {
        loop {
            let c = self.at(t);
            if c == 0 || (!match_slash && c == b'/') || c == lit || (fold && c == up) {
                return t;
            }
            t += 1;
        }
    }
}

/// A byte string without NUL, which reads NUL past its end.
#[derive(Clone, Copy)]
struct Z<'a>(&'a [u8]);

impl Src for Z<'_> {
    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.0.get(i).copied().unwrap_or(0)
    }

    #[inline]
    fn find_from(&self, i: usize, c: u8) -> Option<usize> {
        let s = self.0.get(i..)?;
        memchr::memchr(c, s).map(|j| i + j)
    }

    #[inline]
    fn scan(&self, t: usize, lit: u8, up: u8, fold: bool, match_slash: bool) -> usize {
        let rest = self.0.get(t..).unwrap_or(&[]);
        let found = match (fold && up != lit, match_slash) {
            (false, true) => memchr::memchr(lit, rest),
            (false, false) => memchr::memchr2(lit, b'/', rest),
            (true, true) => memchr::memchr2(lit, up, rest),
            (true, false) => memchr::memchr3(lit, up, b'/', rest),
        };
        t + found.unwrap_or(rest.len())
    }
}

/// A C string read lazily, as the C reads it: never past its NUL, and
/// only as far as the match needs (no `strlen` up front).
struct CStrLazy<'a> {
    p: *const u8,
    /// Bytes `0..known` are known to be non-NUL.
    known: Cell<usize>,
    /// Where the NUL is, once found.
    nul: Cell<usize>,
    _s: PhantomData<&'a [u8]>,
}

impl CStrLazy<'_> {
    /// # Safety
    ///
    /// `p` is a NUL-terminated string, readable and unwritten for `'a`.
    unsafe fn new(p: *const c_char) -> Self {
        CStrLazy {
            p: p.cast(),
            known: Cell::new(0),
            nul: Cell::new(usize::MAX),
            _s: PhantomData,
        }
    }

    /// Byte `j`, which is at most the NUL's index.
    #[inline(always)]
    fn read(&self, j: usize) -> u8 {
        // SAFETY: `j <=` the NUL's index (every byte before `j` is
        // non-NUL: `known`), so `p + j` lies in the string `new` vouches
        // for, and the read is of a byte of it.
        #[allow(clippy::multiple_unsafe_ops_per_block)]
        unsafe {
            self.p.add(j).read()
        }
    }
}

impl Src for CStrLazy<'_> {
    #[inline]
    fn at(&self, i: usize) -> u8 {
        let known = self.known.get();
        if i < known {
            return self.read(i);
        }
        if i >= self.nul.get() {
            return 0;
        }
        let mut j = known;
        loop {
            let c = self.read(j);
            if c == 0 {
                self.known.set(j);
                self.nul.set(j);
                return 0;
            }
            j += 1;
            if j > i {
                self.known.set(j);
                return c;
            }
        }
    }
}

/// Whether the `len` bytes at `s` of `pat` are `name`.
fn name_is<P: Src>(pat: &P, s: usize, len: usize, name: &[u8]) -> bool {
    len == name.len() && name.iter().enumerate().all(|(k, &c)| pat.at(s + k) == c)
}

// The ctype tests of the C, as in the C locale, for ASCII only.
fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}
fn is_graph(c: u8) -> bool {
    c.is_ascii_graphic()
}
fn is_print(c: u8) -> bool {
    c == b' ' || c.is_ascii_graphic()
}
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `dowild`: match the pattern from `p` against the text from `t`.
fn dowild<P: Src, T: Src>(pat: &P, mut p: usize, text: &T, mut t: usize, flags: c_uint) -> c_int {
    let casefold = flags & WM_CASEFOLD != 0;
    let pathname = flags & WM_PATHNAME != 0;
    let pattern_start = p;

    loop {
        let mut p_ch = pat.at(p);
        if p_ch == 0 {
            break;
        }
        let mut t_ch = text.at(t);
        if t_ch == 0 && p_ch != b'*' {
            return WM_ABORT_ALL;
        }
        if casefold && t_ch.is_ascii_uppercase() {
            t_ch = t_ch.to_ascii_lowercase();
        }
        if casefold && p_ch.is_ascii_uppercase() {
            p_ch = p_ch.to_ascii_lowercase();
        }
        match p_ch {
            b'?' => {
                // Anything but '/'.
                if pathname && t_ch == b'/' {
                    return WM_NOMATCH;
                }
            }
            b'*' => {
                let match_slash;
                p += 1;
                if pat.at(p) == b'*' {
                    // `prev_p = p - 2`: the byte before the first '*', if any.
                    let prev = (p >= pattern_start + 2).then(|| pat.at(p - 2));
                    p += 1;
                    while pat.at(p) == b'*' {
                        p += 1;
                    }
                    if !pathname {
                        // Without WM_PATHNAME, '*' == '**'.
                        match_slash = true;
                    } else if prev.is_none_or(|c| c == b'/')
                        && (pat.at(p) == 0
                            || pat.at(p) == b'/'
                            || (pat.at(p) == b'\\' && pat.at(p + 1) == b'/'))
                    {
                        // Assuming we already match 'foo/' and are at
                        // <star star slash>, assume it matches nothing and
                        // match the rest of the pattern with the remaining
                        // text: foo/**/bar matches foo/bar and foo/a/bar.
                        if pat.at(p) == b'/' && dowild(pat, p + 1, text, t, flags) == WM_MATCH {
                            return WM_MATCH;
                        }
                        match_slash = true;
                    } else {
                        match_slash = false;
                    }
                } else {
                    // Without WM_PATHNAME, '*' == '**'.
                    match_slash = !pathname;
                }
                if pat.at(p) == 0 {
                    // A trailing "**" matches everything; a trailing "*"
                    // only if no slash follows.
                    if !match_slash && text.find_from(t, b'/').is_some() {
                        return WM_NOMATCH;
                    }
                    return WM_MATCH;
                } else if !match_slash && pat.at(p) == b'/' {
                    // One asterisk followed by a slash with WM_PATHNAME
                    // matches the next directory.
                    let Some(slash) = text.find_from(t, b'/') else {
                        return WM_NOMATCH;
                    };
                    // The slash is consumed by the loop's step.
                    t = slash;
                    p += 1;
                    t += 1;
                    continue;
                }
                loop {
                    if t_ch == 0 {
                        break;
                    }
                    // Advance faster when the asterisk is followed by a
                    // literal: the text before the literal must belong to
                    // the '*'. Without match_slash, not past a slash.
                    if !is_glob_special(pat.at(p)) {
                        let mut lit = pat.at(p);
                        if casefold && lit.is_ascii_uppercase() {
                            lit = lit.to_ascii_lowercase();
                        }
                        // C's scan for `lit`, stopping at the end and
                        // (without match_slash) at a slash.
                        t = text.scan(t, lit, lit.to_ascii_uppercase(), casefold, match_slash);
                        t_ch = text.at(t);
                        if casefold && t_ch.is_ascii_uppercase() {
                            t_ch = t_ch.to_ascii_lowercase();
                        }
                        if t_ch != lit {
                            return WM_NOMATCH;
                        }
                    }
                    let matched = dowild(pat, p, text, t, flags);
                    if matched != WM_NOMATCH {
                        if !match_slash || matched != WM_ABORT_TO_STARSTAR {
                            return matched;
                        }
                    } else if !match_slash && t_ch == b'/' {
                        return WM_ABORT_TO_STARSTAR;
                    }
                    t += 1;
                    t_ch = text.at(t);
                }
                return WM_ABORT_ALL;
            }
            b'[' => {
                p += 1;
                p_ch = pat.at(p);
                if p_ch == NEGATE_CLASS2 {
                    p_ch = NEGATE_CLASS;
                }
                let negated = p_ch == NEGATE_CLASS;
                if negated {
                    // Inverted character class.
                    p += 1;
                    p_ch = pat.at(p);
                }
                let mut prev_ch: u8 = 0;
                let mut matched = false;
                loop {
                    if p_ch == 0 {
                        return WM_ABORT_ALL;
                    }
                    if p_ch == b'\\' {
                        p += 1;
                        p_ch = pat.at(p);
                        if p_ch == 0 {
                            return WM_ABORT_ALL;
                        }
                        if t_ch == p_ch {
                            matched = true;
                        }
                    } else if p_ch == b'-'
                        && prev_ch != 0
                        && pat.at(p + 1) != 0
                        && pat.at(p + 1) != b']'
                    {
                        p += 1;
                        p_ch = pat.at(p);
                        if p_ch == b'\\' {
                            p += 1;
                            p_ch = pat.at(p);
                            if p_ch == 0 {
                                return WM_ABORT_ALL;
                            }
                        }
                        if t_ch <= p_ch && t_ch >= prev_ch {
                            matched = true;
                        } else if casefold && t_ch.is_ascii_lowercase() {
                            let up = t_ch.to_ascii_uppercase();
                            if up <= p_ch && up >= prev_ch {
                                matched = true;
                            }
                        }
                        p_ch = 0; // so that prev_ch becomes 0
                    } else if p_ch == b'[' && pat.at(p + 1) == b':' {
                        p += 2;
                        let s = p;
                        loop {
                            p_ch = pat.at(p);
                            if p_ch == 0 || p_ch == b']' {
                                break;
                            }
                            p += 1;
                        }
                        if p_ch == 0 {
                            return WM_ABORT_ALL;
                        }
                        // `i = p - s - 1`: the class name's length.
                        if p < s + 1 || pat.at(p - 1) != b':' {
                            // No ":]": treat as a normal set.
                            p = s - 2;
                            p_ch = b'[';
                            if t_ch == p_ch {
                                matched = true;
                            }
                        } else {
                            let len = p - 1 - s;
                            let is = |name: &[u8]| name_is(pat, s, len, name);
                            let hit = if is(b"alnum") {
                                t_ch.is_ascii_alphanumeric()
                            } else if is(b"alpha") {
                                t_ch.is_ascii_alphabetic()
                            } else if is(b"blank") {
                                is_blank(t_ch)
                            } else if is(b"cntrl") {
                                t_ch.is_ascii_control()
                            } else if is(b"digit") {
                                t_ch.is_ascii_digit()
                            } else if is(b"graph") {
                                is_graph(t_ch)
                            } else if is(b"lower") {
                                t_ch.is_ascii_lowercase()
                            } else if is(b"print") {
                                is_print(t_ch)
                            } else if is(b"punct") {
                                t_ch.is_ascii_punctuation()
                            } else if is(b"space") {
                                is_space(t_ch)
                            } else if is(b"upper") {
                                t_ch.is_ascii_uppercase() || (casefold && t_ch.is_ascii_lowercase())
                            } else if is(b"xdigit") {
                                t_ch.is_ascii_hexdigit()
                            } else {
                                // A malformed [:class:].
                                return WM_ABORT_ALL;
                            };
                            if hit {
                                matched = true;
                            }
                            p_ch = 0; // so that prev_ch becomes 0
                        }
                    } else if t_ch == p_ch {
                        matched = true;
                    }
                    prev_ch = p_ch;
                    p += 1;
                    p_ch = pat.at(p);
                    if p_ch == b']' {
                        break;
                    }
                }
                if matched == negated || (pathname && t_ch == b'/') {
                    return WM_NOMATCH;
                }
            }
            _ => {
                // '\\': literal match with the next character (a NUL there
                // fails the comparison below).
                if p_ch == b'\\' {
                    p += 1;
                    p_ch = pat.at(p);
                }
                if t_ch != p_ch {
                    return WM_NOMATCH;
                }
            }
        }
        t += 1;
        p += 1;
    }

    if text.at(t) != 0 {
        WM_NOMATCH
    } else {
        WM_MATCH
    }
}

/// `wildmatch` on byte strings (each ending at its first NUL or its end),
/// with C's flags and result: [`WM_MATCH`], [`WM_NOMATCH`],
/// [`WM_ABORT_ALL`], or (as in C, with `WM_PATHNAME`)
/// [`WM_ABORT_TO_STARSTAR`].
pub(crate) fn wildmatch_raw(pattern: &[u8], text: &[u8], flags: c_uint) -> c_int {
    let to_nul = |s: &[u8]| -> usize { memchr::memchr(0, s).unwrap_or(s.len()) };
    let (pattern, text) = (&pattern[..to_nul(pattern)], &text[..to_nul(text)]);
    dowild(&Z(pattern), 0, &Z(text), 0, flags)
}

/// [`wildmatch`]'s options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WildmatchFlags {
    /// Letters match either case (`WM_CASEFOLD`).
    pub casefold: bool,
    /// `/` is matched only by a literal `/` or by `**` (`WM_PATHNAME`).
    pub pathname: bool,
}

/// [`wildmatch`]'s result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildMatch {
    /// The text matches (`WM_MATCH`).
    Match,
    /// It does not (`WM_NOMATCH`).
    NoMatch,
    /// It does not, and no longer text could (`WM_ABORT_ALL`), or the
    /// pattern is malformed.
    AbortAll,
    /// It does not, and only a `**` could have matched more
    /// (`WM_ABORT_TO_STARSTAR`, with `pathname`).
    AbortToStarstar,
}

impl WildMatch {
    /// Whether the text matched.
    pub fn is_match(self) -> bool {
        self == WildMatch::Match
    }
}

/// Match `pattern` against `text` (each ending at its first NUL or its
/// end), as git's `wildmatch`.
pub fn wildmatch(pattern: &[u8], text: &[u8], flags: WildmatchFlags) -> WildMatch {
    let f =
        if flags.casefold { WM_CASEFOLD } else { 0 } | if flags.pathname { WM_PATHNAME } else { 0 };
    match wildmatch_raw(pattern, text, f) {
        WM_MATCH => WildMatch::Match,
        WM_NOMATCH => WildMatch::NoMatch,
        WM_ABORT_TO_STARSTAR => WildMatch::AbortToStarstar,
        _ => WildMatch::AbortAll,
    }
}

pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// `wildmatch`.
    ///
    /// # Safety
    ///
    /// `pattern` and `text` are NUL-terminated strings, unwritten during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn wildmatch(
        pattern: *const c_char,
        text: *const c_char,
        flags: c_uint,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let p = unsafe { CStrLazy::new(pattern) };
            // SAFETY: the caller's contract.
            let t = unsafe { CStrLazy::new(text) };
            dowild(&p, 0, &t, 0, flags)
        })
    }
}

crate::c_layout_test! {
    headers: ["wildmatch.h"],
    types: [],
    asserts: [
        "WM_CASEFOLD == 1 && WM_PATHNAME == 2",
        "WM_NOMATCH == 1 && WM_MATCH == 0 && WM_ABORT_ALL == -1 && WM_ABORT_TO_STARSTAR == -2",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, t: &str, f: c_uint) -> c_int {
        wildmatch_raw(p.as_bytes(), t.as_bytes(), f)
    }

    #[test]
    fn basics() {
        assert_eq!(m("foo", "foo", 0), WM_MATCH);
        assert_eq!(m("foo", "bar", 0), WM_NOMATCH);
        assert_eq!(m("", "", 0), WM_MATCH);
        assert_eq!(m("???", "foo", 0), WM_MATCH);
        assert_eq!(m("*f", "foo", 0), WM_NOMATCH);
        assert_eq!(m("*", "foo/bar", WM_PATHNAME), WM_NOMATCH);
        assert_eq!(m("foo/**/bar", "foo/bar", WM_PATHNAME), WM_MATCH);
        assert_eq!(m("foo/**/bar", "foo/a/b/bar", WM_PATHNAME), WM_MATCH);
        assert_eq!(m("foo/*/bar", "foo/a/bar", WM_PATHNAME), WM_MATCH);
        assert_eq!(m("[a-c]x", "bx", 0), WM_MATCH);
        assert_eq!(m("[!a-c]x", "bx", 0), WM_NOMATCH);
        assert_eq!(m("[[:alpha:]]", "Q", 0), WM_MATCH);
        assert_eq!(m("[[:bogus:]]", "Q", 0), WM_ABORT_ALL);
        assert_eq!(m("[[:alpha]", "[", 0), WM_MATCH);
        assert_eq!(m("FOO", "foo", WM_CASEFOLD), WM_MATCH);
        assert_eq!(m("[A-Z]", "q", WM_CASEFOLD), WM_MATCH);
        assert_eq!(m("\\", "", 0), WM_ABORT_ALL);
        assert_eq!(m("a\\", "a", 0), WM_ABORT_ALL);
        assert_eq!(m("[", "[", 0), WM_ABORT_ALL);
        // A NUL ends the strings, as in C.
        assert_eq!(wildmatch_raw(b"ab\0zz", b"ab\0yy", 0), WM_MATCH);
        assert_eq!(m("*?", "a/", WM_PATHNAME), WM_ABORT_TO_STARSTAR);
    }
}
