//! Port of `src/util/util.c`, and Rust equivalents of the helpers of
//! `src/util/util.h`.
//!
//! The Rust functions take byte slices where C takes NUL-terminated
//! strings: the end of a slice is where C's string ends, and a C string
//! cannot contain a NUL, so on NUL-free input the results are the C's.

use core::cmp::Ordering;
use core::ffi::{CStr, c_char, c_int, c_void};

use super::ctype_compat::{isspace, tolower};
use super::errors::{ErrorClass, ErrorCode, Result};
use super::str::GitStr;
use crate::git_error_set;

/// The byte at `i`, or 0 past the end (C's terminating NUL).
#[inline(always)]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Number parsing
// ---------------------------------------------------------------------------

/// How a `strntol` parse ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    /// The number.
    Value(i64),
    /// The digits overflowed an `int64_t`.
    Overflow,
    /// No digits.
    NotANumber,
}

/// Parse an integer at the start of `s` as `git__strntol64` does, without
/// setting an error: the outcome and the number of bytes consumed (which C
/// returns through `endptr` for a value or an overflow).
///
/// Leading whitespace and a sign are skipped; base 0 means 16 after `0x`,
/// 8 after `0`, 10 otherwise.
pub fn parse_i64(s: &[u8], base: c_int) -> (ParseOutcome, usize) {
    let mut p = 0usize;
    let len = s.len();
    let mut neg = false;
    let mut n: i64 = 0;
    let mut ndig = 0usize;
    let mut ovfl = false;
    let mut base = base;

    'ret: {
        while p < len && isspace(s[p]) {
            p += 1;
        }
        if p == len {
            break 'ret;
        }
        if s[p] == b'-' || s[p] == b'+' {
            neg = s[p] == b'-';
            p += 1;
        }
        if p == len {
            break 'ret;
        }
        let rem = len - p;
        if base == 0 {
            base = if s[p] != b'0' {
                10
            } else if rem > 2 && (s[p + 1] == b'x' || s[p + 1] == b'X') {
                16
            } else {
                8
            };
        }
        if !(0..=36).contains(&base) {
            break 'ret;
        }
        if base == 16 && rem > 2 && s[p] == b'0' && (s[p + 1] == b'x' || s[p + 1] == b'X') {
            p += 2;
        }
        let b = i64::from(base);
        while p < len {
            let c = s[p];
            let v = match c {
                b'0'..=b'9' => i64::from(c - b'0'),
                b'a'..=b'z' => i64::from(c - b'a') + 10,
                b'A'..=b'Z' => i64::from(c - b'A') + 10,
                _ => b,
            };
            if v >= b {
                break;
            }
            let v = if neg { -v } else { v };
            match n.checked_mul(b).and_then(|nn| nn.checked_add(v)) {
                Some(next) => n = next,
                None => ovfl = true,
            }
            p += 1;
            ndig += 1;
        }
    }

    if ndig == 0 {
        (ParseOutcome::NotANumber, p)
    } else if ovfl {
        (ParseOutcome::Overflow, p)
    } else {
        (ParseOutcome::Value(n), p)
    }
}

/// `git__strntol64`: the number at the start of `s` and the bytes consumed.
/// Errors as C: `failed to convert string to long: not a number` /
/// `: overflow error`.
pub fn strntol64(s: &[u8], base: c_int) -> Result<(i64, usize)> {
    match parse_i64(s, base) {
        (ParseOutcome::Value(n), end) => Ok((n, end)),
        (ParseOutcome::NotANumber, _) => Err(not_a_number()),
        (ParseOutcome::Overflow, _) => Err(overflow()),
    }
}

fn not_a_number() -> ErrorCode {
    git_error_set!(
        ErrorClass::INVALID,
        "failed to convert string to long: not a number"
    )
}

fn overflow() -> ErrorCode {
    git_error_set!(
        ErrorClass::INVALID,
        "failed to convert string to long: overflow error"
    )
}

/// `git__strntol32`: as [`strntol64`], and `failed to convert: '<text>' is
/// too large` when the value does not fit an `int32_t`.
pub fn strntol32(s: &[u8], base: c_int) -> Result<(i32, usize)> {
    let (n, end) = strntol64(s, base)?;
    match i32::try_from(n) {
        Ok(v) => Ok((v, end)),
        Err(_) => Err(git_error_set!(
            ErrorClass::INVALID,
            "failed to convert: '{}' is too large",
            super::errors::Bytes(&s[..end])
        )),
    }
}

// ---------------------------------------------------------------------------
// Comparisons
// ---------------------------------------------------------------------------

/// `git__strcasecmp` (ASCII case folding).
pub fn strcasecmp(a: &[u8], b: &[u8]) -> c_int {
    let mut i = 0;
    while i < a.len() && i < b.len() && tolower(a[i]) == tolower(b[i]) {
        i += 1;
    }
    c_int::from(tolower(at(a, i))) - c_int::from(tolower(at(b, i)))
}

/// `git__strcmp`: `strcmp` on byte strings (unsigned bytes).
pub fn strcmp(a: &[u8], b: &[u8]) -> c_int {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => c_int::from(a[i]) - c_int::from(b[i]),
        None => c_int::from(at(a, b.len().min(a.len()))) - c_int::from(at(b, a.len().min(b.len()))),
    }
}

/// `git__strcasesort_cmp`: case-insensitive, with case as the tie-break.
pub fn strcasesort_cmp(a: &[u8], b: &[u8]) -> c_int {
    let mut cmp = 0;
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a[i] != b[i] {
            if tolower(a[i]) != tolower(b[i]) {
                break;
            }
            if cmp == 0 {
                cmp = c_int::from(a[i]) - c_int::from(b[i]);
            }
        }
        i += 1;
    }
    if i < a.len() || i < b.len() {
        return c_int::from(tolower(at(a, i))) - c_int::from(tolower(at(b, i)));
    }
    cmp
}

/// `git__strncasecmp`. As in C, `sz == 0` compares the whole strings (the
/// C's loop decrements `sz` past 0; see `rust/DEFECTS.md`).
pub fn strncasecmp(a: &[u8], b: &[u8], sz: usize) -> c_int {
    let mut sz = sz;
    let mut i = 0;
    loop {
        let al = c_int::from(tolower(at(a, i)));
        let bl = c_int::from(tolower(at(b, i)));
        i += 1;
        sz = sz.wrapping_sub(1);
        if sz == 0 || al == 0 || al != bl {
            return al - bl;
        }
    }
}

/// `git__strntolower`.
pub fn strntolower(s: &mut [u8]) {
    s.make_ascii_lowercase();
}

fn prefixcmp_impl(s: &[u8], s_n: usize, prefix: &[u8], icase: bool) -> c_int {
    let mut i = 0;
    while i < s_n {
        let mut sc = c_int::from(at(s, i));
        let mut pc = c_int::from(at(prefix, i));
        i += 1;
        if icase {
            sc = c_int::from(tolower(sc as u8));
            pc = c_int::from(tolower(pc as u8));
        }
        if pc == 0 {
            return 0;
        }
        if sc != pc {
            return sc - pc;
        }
    }
    // C: `0 - *prefix`, with `char` signed on the port's platforms.
    -c_int::from(at(prefix, i) as i8)
}

/// `git__prefixcmp`: 0 if `s` starts with `prefix`, else the difference of
/// the first differing bytes.
pub fn prefixcmp(s: &[u8], prefix: &[u8]) -> c_int {
    match s.iter().zip(prefix).position(|(a, b)| a != b) {
        Some(i) => c_int::from(s[i]) - c_int::from(prefix[i]),
        None if prefix.len() <= s.len() => 0,
        None => -c_int::from(prefix[s.len()]),
    }
}

/// `git__prefixncmp`: [`prefixcmp`] on the first `s_n` bytes of `s`.
pub fn prefixncmp(s: &[u8], s_n: usize, prefix: &[u8]) -> c_int {
    prefixcmp_impl(s, s_n, prefix, false)
}

/// `git__prefixcmp_icase`.
pub fn prefixcmp_icase(s: &[u8], prefix: &[u8]) -> c_int {
    prefixcmp_impl(s, usize::MAX, prefix, true)
}

/// `git__prefixncmp_icase`.
pub fn prefixncmp_icase(s: &[u8], s_n: usize, prefix: &[u8]) -> c_int {
    prefixcmp_impl(s, s_n, prefix, true)
}

/// `git__suffixcmp`: 0 if `s` ends with `suffix`, -1 if `s` is shorter,
/// else the comparison of the tail.
pub fn suffixcmp(s: &[u8], suffix: &[u8]) -> c_int {
    if s.len() < suffix.len() {
        return -1;
    }
    strcmp(&s[s.len() - suffix.len()..], suffix)
}

/// `git__suffixcmp_icase`.
pub fn suffixcmp_icase(s: &[u8], suffix: &[u8]) -> c_int {
    if s.len() < suffix.len() {
        return -1;
    }
    strcasecmp(&s[s.len() - suffix.len()..], suffix)
}

/// `git__strlcmp`: compare `a` with the `b_len`-byte string `b`.
pub fn strlcmp(a: &[u8], b: &[u8]) -> c_int {
    let n = b.len();
    let cmp = strncmp(a, b, n);
    if cmp != 0 { cmp } else { c_int::from(at(a, n)) }
}

/// `strncmp` on byte strings.
pub fn strncmp(a: &[u8], b: &[u8], n: usize) -> c_int {
    for i in 0..n {
        let (x, y) = (at(a, i), at(b, i));
        if x != y {
            return c_int::from(x) - c_int::from(y);
        }
        if x == 0 {
            return 0;
        }
    }
    0
}

/// `git__parse_bool`: `true`/`yes`/`on` (or no value) and
/// `false`/`no`/`off`/empty, case-insensitively; `None` otherwise (C
/// returns -1 and sets no error).
pub fn parse_bool(value: Option<&[u8]>) -> Option<bool> {
    let Some(v) = value else { return Some(true) };
    let eq = |w: &[u8]| v.eq_ignore_ascii_case(w);
    if eq(b"true") || eq(b"yes") || eq(b"on") {
        Some(true)
    } else if eq(b"false") || eq(b"no") || eq(b"off") || v.is_empty() {
        Some(false)
    } else {
        None
    }
}

/// `git__unescape`: remove backslashes in place, keeping the character
/// after each (a trailing backslash stays). Returns the new length.
pub fn unescape(s: &mut [u8]) -> usize {
    let mut pos = 0;
    let mut scan = 0;
    while scan < s.len() {
        if s[scan] == b'\\' && scan + 1 < s.len() {
            scan += 1;
        }
        s[pos] = s[scan];
        pos += 1;
        scan += 1;
    }
    pos
}

/// `git__linenlen`: the length of the first line of `buf`, its `\n`
/// included.
pub fn linenlen(buf: &[u8]) -> usize {
    match memchr::memchr(b'\n', buf) {
        Some(i) => i + 1,
        None => buf.len(),
    }
}

/// `git__next_line`: skip to after the next run of line ends.
pub fn next_line(s: &[u8]) -> &[u8] {
    let mut i = memchr::memchr(b'\n', s).unwrap_or(s.len());
    while i < s.len() && (s[i] == b'\n' || s[i] == b'\r') {
        i += 1;
    }
    &s[i..]
}

/// `git__memmem`: the first occurrence of `needle` in `haystack`; `None`
/// for an empty needle or haystack, as in C.
pub fn memmem(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() || haystack.is_empty() || needle.is_empty() {
        return None;
    }
    memchr::memmem::find(haystack, needle)
}

/// `git__memrchr`.
pub fn memrchr(s: &[u8], c: u8) -> Option<usize> {
    memchr::memrchr(c, s)
}

// ---------------------------------------------------------------------------
// Hex, sizes, misc
// ---------------------------------------------------------------------------

/// `git__fromhex`: the value of a hex digit, or -1.
#[inline(always)]
pub const fn fromhex(h: u8) -> i32 {
    match h {
        b'0'..=b'9' => (h - b'0') as i32,
        b'a'..=b'f' => (h - b'a') as i32 + 10,
        b'A'..=b'F' => (h - b'A') as i32 + 10,
        _ => -1,
    }
}

/// `git__ishex`: whether every byte is a hex digit.
pub fn ishex(s: &[u8]) -> bool {
    s.iter().all(|&c| fromhex(c) >= 0)
}

/// `git__size_t_bitmask`.
pub const fn size_t_bitmask(v: usize) -> usize {
    let mut v = v.wrapping_sub(1);
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v
}

/// `git__size_t_powerof2`.
pub const fn size_t_powerof2(v: usize) -> usize {
    size_t_bitmask(v).wrapping_add(1)
}

/// `git__isspace_nonlf`.
pub const fn isspace_nonlf(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | 0x0c | b'\r' | 0x0b)
}

/// `git__iswildcard`.
pub const fn iswildcard(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'[')
}

/// `git__signum`.
pub const fn signum(v: c_int) -> c_int {
    (v > 0) as c_int - (v < 0) as c_int
}

/// `git__memzero`: zero `data` so that the stores are not elided.
pub fn memzero(data: &mut [u8]) {
    for b in data.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
}

/// `git_time_monotonic`: milliseconds of `CLOCK_MONOTONIC`.
pub fn time_monotonic() -> u64 {
    let mut tp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `tp` is valid for writes.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut tp) } == 0 {
        // C computes `tv_sec * 1000 + tv_nsec / 1.0E6` in floating point.
        return ((tp.tv_sec as f64) * 1000.0 + (tp.tv_nsec as f64) / 1.0e6) as u64;
    }
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    // SAFETY: `tv` is valid for writes; a NULL timezone is allowed.
    unsafe { libc::gettimeofday(&raw mut tv, core::ptr::null_mut()) };
    (tv.tv_sec as u64) * 1000 + (tv.tv_usec as u64) / 1000
}

/// `git__hash`: libgit2's variant of MurmurHash3 (32-bit).
pub fn hash(key: &[u8], seed: u32) -> u32 {
    // `len` is an `int` in C.
    let len = key.len() as i32 as u32;
    let nblocks = key.len() / 4;
    let mut h1: u32 = 0x971e_137b ^ seed;
    let mut c1: u32 = 0x9554_3787;
    let mut c2: u32 = 0x2ad7_eb25;
    let mut block = |k1: u32, h1: &mut u32| {
        let mut k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(11);
        k1 = k1.wrapping_mul(c2);
        *h1 ^= k1;
        *h1 = h1.wrapping_mul(3).wrapping_add(0x52dc_e729);
        c1 = c1.wrapping_mul(5).wrapping_add(0x7b7d_159c);
        c2 = c2.wrapping_mul(5).wrapping_add(0x6bce_6396);
    };
    for chunk in key.as_chunks::<4>().0 {
        block(u32::from_ne_bytes(*chunk), &mut h1);
    }
    let tail = &key[nblocks * 4..];
    let mut k1: u32 = 0;
    match key.len() & 3 {
        3 => {
            k1 ^= u32::from(tail[2]) << 16;
            k1 ^= u32::from(tail[1]) << 8;
            k1 ^= u32::from(tail[0]);
            block(k1, &mut h1);
        }
        2 => {
            k1 ^= u32::from(tail[1]) << 8;
            k1 ^= u32::from(tail[0]);
            block(k1, &mut h1);
        }
        1 => {
            k1 ^= u32::from(tail[0]);
            block(k1, &mut h1);
        }
        _ => {}
    }
    h1 ^= len;
    h1 ^= h1 >> 16;
    h1 = h1.wrapping_mul(0x85eb_ca6b);
    h1 ^= h1 >> 13;
    h1 = h1.wrapping_mul(0xc2b2_ae35);
    h1 ^= h1 >> 16;
    h1
}

/// The text `git__hexdump` prints for `buf`.
pub fn hexdump_string(buf: &[u8]) -> String {
    use core::fmt::Write;
    const W: usize = 16;
    let mut out = String::new();
    let printable = |c: u8| {
        if (32..=126).contains(&c) {
            c as char
        } else {
            '.'
        }
    };
    let lines = buf.len() / W;
    let last = buf.len() % W;
    for i in 0..lines {
        let line = &buf[i * W..(i + 1) * W];
        let _ = write!(out, "{:08x}  ", i * W);
        for (j, &c) in line.iter().enumerate() {
            let _ = write!(out, "{c:02x} ");
            if j == W / 2 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        out.extend(line.iter().map(|&c| printable(c)));
        out.push_str("|\n");
    }
    if last > 0 {
        let line = &buf[lines * W..];
        let _ = write!(out, "{:08x}  ", lines * W);
        for (j, &c) in line.iter().enumerate() {
            let _ = write!(out, "{c:02x} ");
            if j == W / 2 {
                out.push(' ');
            }
        }
        if last < W / 2 {
            out.push(' ');
        }
        for _ in 0..(W - last) {
            out.push_str("   ");
        }
        out.push_str(" |");
        out.extend(line.iter().map(|&c| printable(c)));
        out.push_str("|\n");
    }
    out.push('\n');
    out
}

/// `git__bsearch`: binary search for the element `cmp` (called with each
/// probed element, returning the key's order against it) reports equal.
/// `Ok(position)` when found, `Err(insertion position)` otherwise.
pub fn bsearch<T>(
    array: &[T],
    mut cmp: impl FnMut(&T) -> c_int,
) -> core::result::Result<usize, usize> {
    let mut base = 0usize;
    let mut lim = array.len();
    while lim != 0 {
        let part = base + (lim >> 1);
        let c = cmp(&array[part]);
        if c == 0 {
            return Ok(part);
        }
        if c > 0 {
            base = part + 1;
            lim -= 1;
        }
        lim >>= 1;
    }
    Err(base)
}

/// `git__getenv`: set `out` to the environment variable `name`;
/// `GIT_ENOTFOUND` (with `out` cleared and no error set) when it is unset.
pub fn getenv(out: &mut GitStr, name: &CStr) -> Result<()> {
    out.clear();
    // SAFETY: `name` is NUL-terminated. `getenv`'s result is read before
    // any call that could change the environment; the port, like the C,
    // relies on the environment not being changed concurrently.
    let val = unsafe { libc::getenv(name.as_ptr()) };
    if val.is_null() {
        return Err(ErrorCode::NOT_FOUND);
    }
    // SAFETY: a non-null `getenv` result is a NUL-terminated string.
    out.puts(unsafe { CStr::from_ptr(val) }.to_bytes())
}

/// `git__online_cpus`.
pub fn online_cpus() -> c_int {
    // SAFETY: `sysconf` has no preconditions.
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n > 0 { n as c_int } else { 1 }
}

/// A C `strcmp`-style comparison as an [`Ordering`].
#[inline]
pub fn ordering(c: c_int) -> Ordering {
    c.cmp(&0)
}

/// `git__strtok` on a byte buffer: skip separators from `*pos`, and return
/// the token's range, writing a NUL after it (if it is not at the end) and
/// moving `*pos` past it; `None` (and `*pos` unchanged) when only
/// separators remain. The buffer ends at its first NUL or its end.
pub fn strtok(buf: &mut [u8], pos: &mut usize, sep: &[u8]) -> Option<core::ops::Range<usize>> {
    let end_of = |b: &[u8], i: usize| i >= b.len() || b[i] == 0;
    let mut p = *pos;
    while !end_of(buf, p) && sep.contains(&buf[p]) {
        p += 1;
    }
    if end_of(buf, p) {
        return None;
    }
    let start = p;
    let mut e = start + 1;
    while !end_of(buf, e) && !sep.contains(&buf[e]) {
        e += 1;
    }
    let tok = start..e;
    if !end_of(buf, e) {
        buf[e] = 0;
        e += 1;
    }
    *pos = e;
    Some(tok)
}

/// `git__strsep` on a byte buffer: the token from `*pos` up to the next
/// separator, which is replaced with a NUL, moving `*pos` past it; `None`
/// (and `*pos` unchanged) when no separator follows.
pub fn strsep(buf: &mut [u8], pos: &mut usize, sep: &[u8]) -> Option<core::ops::Range<usize>> {
    let start = *pos;
    let mut p = start;
    while p < buf.len() && buf[p] != 0 && !sep.contains(&buf[p]) {
        p += 1;
    }
    if p < buf.len() && buf[p] != 0 {
        buf[p] = 0;
        *pos = p + 1;
        return Some(start..p);
    }
    None
}

/// The exports of `util.c`.
pub mod ffi {
    use super::*;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, bytes, cstr_bytes, status};

    /// The bytes of `s`, bounded to what a comparison with a `bound`-byte
    /// string can read: C stops at the first difference, so at most
    /// `bound + 1` bytes of `s` matter.
    ///
    /// # Safety
    ///
    /// `s` is NUL-terminated.
    unsafe fn bounded<'a>(s: *const c_char, bound: usize) -> &'a [u8] {
        // SAFETY: `strnlen` stops at the NUL of the caller's string.
        let n = unsafe { libc::strnlen(s, bound.saturating_add(1)) };
        // SAFETY: the first `n` bytes precede the NUL or the bound.
        unsafe { bytes(s, n) }
    }

    /// `git__strntol64`.
    ///
    /// # Safety
    ///
    /// `nptr` is readable for `nptr_len` bytes; `result` is writable;
    /// `endptr` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strntol64(
        mut result: Out<'_, i64>,
        nptr: *const c_char,
        nptr_len: usize,
        mut endptr: Out<'_, *const c_char>,
        base: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes `nptr_len` readable bytes.
            let s = unsafe { bytes(nptr, nptr_len) };
            let (outcome, end) = parse_i64(s, base);
            let endp = nptr.wrapping_add(end);
            match outcome {
                ParseOutcome::NotANumber => status(Err(not_a_number())),
                ParseOutcome::Overflow => {
                    endptr.write(endp);
                    status(Err(overflow()))
                }
                ParseOutcome::Value(n) => {
                    endptr.write(endp);
                    result.write(n);
                    0
                }
            }
        })
    }

    /// `git__strntol32`.
    ///
    /// # Safety
    ///
    /// As `git__strntol64`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strntol32(
        mut result: Out<'_, i32>,
        nptr: *const c_char,
        nptr_len: usize,
        mut endptr: Out<'_, *const c_char>,
        base: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes `nptr_len` readable bytes.
            let s = unsafe { bytes(nptr, nptr_len) };
            match strntol32(s, base) {
                Ok((v, end)) => {
                    result.write(v);
                    endptr.write(nptr.wrapping_add(end));
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    macro_rules! cmp2 {
        ($(#[$m:meta])* $name:ident, $f:path) => {
            $(#[$m])*
            ///
            /// # Safety
            ///
            /// Both arguments are NUL-terminated strings.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(a: *const c_char, b: *const c_char) -> c_int {
                // SAFETY: the caller passes NUL-terminated strings.
                barrier(|| $f(unsafe { cstr_bytes(a) }, unsafe { cstr_bytes(b) }))
            }
        };
    }

    cmp2!(
        /// `git__strcasecmp`.
        git__strcasecmp,
        strcasecmp
    );
    cmp2!(
        /// `git__strcasesort_cmp`.
        git__strcasesort_cmp,
        strcasesort_cmp
    );
    cmp2!(
        /// `git__suffixcmp`.
        git__suffixcmp,
        suffixcmp
    );
    cmp2!(
        /// `git__suffixcmp_icase`.
        git__suffixcmp_icase,
        suffixcmp_icase
    );

    /// `git__strncasecmp`.
    ///
    /// # Safety
    ///
    /// Both are NUL-terminated, or readable for `sz` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strncasecmp(
        a: *const c_char,
        b: *const c_char,
        sz: usize,
    ) -> c_int {
        barrier(|| {
            // C reads at most `sz` bytes of each (the loop stops when
            // `--sz` reaches 0, before index `sz`); `sz == 0` compares the
            // whole strings (see `strncasecmp`).
            let bound = if sz == 0 { usize::MAX } else { sz };
            // SAFETY: the caller's strings end at a NUL or are readable for
            // `sz` bytes; `strnlen` reads no further than that.
            let la = unsafe { libc::strnlen(a, bound) };
            // SAFETY: as for `a`.
            let lb = unsafe { libc::strnlen(b, bound) };
            // SAFETY: the `la` bytes just measured are readable.
            let a = unsafe { bytes(a, la) };
            // SAFETY: as for `a`.
            let b = unsafe { bytes(b, lb) };
            strncasecmp(a, b, sz)
        })
    }

    /// `git__strcmp_cb`.
    ///
    /// # Safety
    ///
    /// Both arguments are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strcmp_cb(a: *const c_void, b: *const c_void) -> c_int {
        // SAFETY: the caller passes NUL-terminated strings.
        barrier(|| unsafe { libc::strcmp(a.cast(), b.cast()) })
    }

    /// `git__strcasecmp_cb`.
    ///
    /// # Safety
    ///
    /// Both arguments are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strcasecmp_cb(a: *const c_void, b: *const c_void) -> c_int {
        // SAFETY: forwarded.
        unsafe { git__strcasecmp(a.cast(), b.cast()) }
    }

    /// `git__prefixcmp`.
    ///
    /// # Safety
    ///
    /// Both arguments are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__prefixcmp(s: *const c_char, prefix: CStrArg<'_>) -> c_int {
        barrier(|| {
            let p = prefix.to_bytes();
            // SAFETY: `s` is NUL-terminated; only its first `strlen(prefix)
            // + 1` bytes can be compared.
            let s = unsafe { bounded(s, p.len()) };
            prefixcmp(s, p)
        })
    }

    /// `git__prefixcmp_icase`.
    ///
    /// # Safety
    ///
    /// Both arguments are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__prefixcmp_icase(s: *const c_char, prefix: CStrArg<'_>) -> c_int {
        barrier(|| {
            let p = prefix.to_bytes();
            // SAFETY: as in `git__prefixcmp`.
            let s = unsafe { bounded(s, p.len()) };
            prefixcmp_icase(s, p)
        })
    }

    /// `git__prefixncmp`.
    ///
    /// # Safety
    ///
    /// `prefix` is NUL-terminated; `s` is readable for `str_n` bytes or up
    /// to a NUL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__prefixncmp(
        s: *const c_char,
        str_n: usize,
        prefix: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let p = prefix.to_bytes();
            // SAFETY: `s` is read up to its NUL or the bytes C could
            // compare, whichever comes first.
            let n = unsafe { libc::strnlen(s, str_n.min(p.len() + 1)) };
            // SAFETY: the `n` bytes just measured are readable.
            let s = unsafe { bytes(s, n) };
            prefixncmp(s, str_n, p)
        })
    }

    /// `git__prefixncmp_icase`.
    ///
    /// # Safety
    ///
    /// As `git__prefixncmp`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__prefixncmp_icase(
        s: *const c_char,
        str_n: usize,
        prefix: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let p = prefix.to_bytes();
            // SAFETY: as in `git__prefixncmp`.
            let n = unsafe { libc::strnlen(s, str_n.min(p.len() + 1)) };
            // SAFETY: as in `git__prefixncmp`.
            let s = unsafe { bytes(s, n) };
            prefixncmp_icase(s, str_n, p)
        })
    }

    /// The `len + 1` bytes of the NUL-terminated string at `s`, mutable.
    ///
    /// # Safety
    ///
    /// `s` is a writable NUL-terminated string, not otherwise accessed for
    /// `'a`.
    unsafe fn cstr_mut<'a>(s: *mut c_char) -> &'a mut [u8] {
        // SAFETY: the caller's string ends at a NUL.
        let len = unsafe { libc::strlen(s) };
        // SAFETY: `len + 1` bytes (the string and its NUL) are writable and
        // exclusively ours for `'a`.
        unsafe { core::slice::from_raw_parts_mut(s.cast::<u8>(), len + 1) }
    }

    /// `git__strntolower`.
    ///
    /// # Safety
    ///
    /// `str` is writable for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strntolower(str: *mut c_char, len: usize) {
        barrier(|| {
            if len == 0 {
                return;
            }
            // SAFETY: the caller passes `len` writable bytes.
            strntolower(unsafe { core::slice::from_raw_parts_mut(str.cast::<u8>(), len) })
        })
    }

    /// `git__strtolower`.
    ///
    /// # Safety
    ///
    /// `str` is a writable NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strtolower(str: *mut c_char) {
        // SAFETY: the caller passes a writable NUL-terminated string.
        barrier(|| strntolower(unsafe { cstr_mut(str) }))
    }

    /// `git__strtok`.
    ///
    /// # Safety
    ///
    /// `*end` is a writable NUL-terminated string; `sep` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strtok(end: *mut *mut c_char, sep: CStrArg<'_>) -> *mut c_char {
        barrier(|| {
            // SAFETY: the caller passes a valid `end`.
            let base = unsafe { *end };
            // SAFETY: `*end` is a writable NUL-terminated string.
            let buf = unsafe { cstr_mut(base) };
            let sep = sep.to_bytes();
            let mut pos = 0;
            match strtok(buf, &mut pos, sep) {
                Some(r) => {
                    // SAFETY: `end` is writable; `pos` is within the string
                    // or at its NUL.
                    unsafe { *end = base.wrapping_add(pos) };
                    base.wrapping_add(r.start)
                }
                // C leaves `*end` as it was when there is no token.
                None => core::ptr::null_mut(),
            }
        })
    }

    /// `git__strsep`.
    ///
    /// # Safety
    ///
    /// As `git__strtok`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strsep(end: *mut *mut c_char, sep: CStrArg<'_>) -> *mut c_char {
        barrier(|| {
            // SAFETY: the caller passes a valid `end`.
            let base = unsafe { *end };
            // SAFETY: `*end` is a writable NUL-terminated string.
            let buf = unsafe { cstr_mut(base) };
            let sep = sep.to_bytes();
            let mut pos = 0;
            match strsep(buf, &mut pos, sep) {
                Some(_) => {
                    // SAFETY: `end` is writable; `pos` is within the string.
                    unsafe { *end = base.wrapping_add(pos) };
                    base
                }
                None => core::ptr::null_mut(),
            }
        })
    }

    /// `git__linenlen`.
    ///
    /// # Safety
    ///
    /// `buffer` is readable for `buffer_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__linenlen(buffer: *const c_char, buffer_len: usize) -> usize {
        // SAFETY: the caller passes `buffer_len` readable bytes.
        barrier(|| linenlen(unsafe { bytes(buffer, buffer_len) }))
    }

    /// `git__memmem`.
    ///
    /// # Safety
    ///
    /// `haystack` and `needle` are readable for their lengths.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__memmem(
        haystack: *const c_void,
        haystacklen: usize,
        needle: *const c_void,
        needlelen: usize,
    ) -> *const c_void {
        barrier(|| {
            // SAFETY: the caller passes a readable haystack.
            let h = unsafe { bytes(haystack.cast(), haystacklen) };
            // SAFETY: and a readable needle.
            let n = unsafe { bytes(needle.cast(), needlelen) };
            match memmem(h, n) {
                Some(i) => haystack.cast::<u8>().wrapping_add(i).cast(),
                None => core::ptr::null(),
            }
        })
    }

    /// `git__hexdump`: print `len` bytes to stdout, through C's `stdio`.
    ///
    /// # Safety
    ///
    /// `buffer` is readable for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__hexdump(buffer: *const c_char, len: usize) {
        barrier(|| {
            // SAFETY: the caller passes `len` readable bytes.
            let text = hexdump_string(unsafe { bytes(buffer, len) });
            let Ok(text) = std::ffi::CString::new(text) else {
                return;
            };
            // SAFETY: a valid format and a NUL-terminated argument.
            unsafe { libc::printf(c"%s".as_ptr(), text.as_ptr()) };
        })
    }

    /// `git__hash`.
    ///
    /// # Safety
    ///
    /// `key` is readable for `len` bytes (`len >= 0`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__hash(key: *const c_void, len: c_int, seed: u32) -> u32 {
        barrier(|| {
            let len = usize::try_from(len).unwrap_or(0);
            // SAFETY: the caller passes `len` readable bytes.
            let key = unsafe { bytes(key.cast(), len) };
            hash(key, seed)
        })
    }

    /// The comparison `git__bsearch` takes.
    pub type BsearchCmp = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;
    /// The comparison `git__bsearch_r` takes.
    pub type BsearchCmpR = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void) -> c_int;

    /// `git__bsearch`.
    ///
    /// # Safety
    ///
    /// `array` holds `array_len` pointers (or `array_len` is 0); `compare`
    /// is sound to call with `key` and each of them; `position` is NULL or
    /// writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__bsearch(
        array: *mut *mut c_void,
        array_len: usize,
        key: *const c_void,
        compare: BsearchCmp,
        mut position: Out<'_, usize>,
    ) -> c_int {
        barrier(|| {
            let a: &[*mut c_void] = if array_len == 0 {
                &[]
            } else {
                // SAFETY: the caller passes `array_len` readable pointers.
                unsafe { core::slice::from_raw_parts(array, array_len) }
            };
            // SAFETY: the caller vouches for `compare` on `key` and each element.
            let r = bsearch(a, |e| unsafe { compare(key, *e) });
            let (pos, ret) = match r {
                Ok(p) => (p, 0),
                Err(p) => (p, ErrorCode::NOT_FOUND.raw()),
            };
            position.write(pos);
            ret
        })
    }

    /// `git__bsearch_r`.
    ///
    /// # Safety
    ///
    /// As `git__bsearch`, with `compare_r` sound to call with `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__bsearch_r(
        array: *mut *mut c_void,
        array_len: usize,
        key: *const c_void,
        compare_r: BsearchCmpR,
        payload: *mut c_void,
        mut position: Out<'_, usize>,
    ) -> c_int {
        barrier(|| {
            let a: &[*mut c_void] = if array_len == 0 {
                &[]
            } else {
                // SAFETY: the caller passes `array_len` readable pointers.
                unsafe { core::slice::from_raw_parts(array, array_len) }
            };
            // SAFETY: the caller vouches for `compare_r`.
            let r = bsearch(a, |e| unsafe { compare_r(key, *e, payload) });
            let (pos, ret) = match r {
                Ok(p) => (p, 0),
                Err(p) => (p, ErrorCode::NOT_FOUND.raw()),
            };
            position.write(pos);
            ret
        })
    }

    /// `git__parse_bool`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `value` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__parse_bool(mut out: Out<'_, c_int>, value: CStrArg<'_>) -> c_int {
        barrier(|| {
            let v = value.get().map(CStr::to_bytes);
            match parse_bool(v) {
                Some(b) => {
                    out.write(c_int::from(b));
                    0
                }
                None => -1,
            }
        })
    }

    /// `git__unescape`.
    ///
    /// # Safety
    ///
    /// `str` is NULL or a writable NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__unescape(str: *mut c_char) -> usize {
        barrier(|| {
            if str.is_null() {
                return 0;
            }
            // SAFETY: the caller passes a writable NUL-terminated string.
            let buf = unsafe { cstr_mut(str) };
            let len = buf.len() - 1;
            let n = unescape(&mut buf[..len]);
            if n != len {
                buf[n] = 0;
            }
            n
        })
    }

    /// The comparison `git__qsort_r` takes.
    pub type SortRCmp = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void) -> c_int;

    /// `git__qsort_r`: glibc's `qsort_r` (`GIT_QSORT_GNU`).
    ///
    /// # Safety
    ///
    /// `els` holds `nel` elements of `elsize` bytes; `cmp` is sound to call
    /// with pointers to any two of them and `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__qsort_r(
        els: *mut c_void,
        nel: usize,
        elsize: usize,
        cmp: SortRCmp,
        payload: *mut c_void,
    ) {
        // SAFETY: forwarded to the caller's contract.
        barrier(|| unsafe { libc::qsort_r(els, nel, elsize, Some(cmp), payload) })
    }

    /// `git__getenv`.
    ///
    /// # Safety
    ///
    /// `out` is a valid `git_str`; `name` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__getenv(out: *mut GitStr, name: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a valid, unaliased `git_str`.
            let out = unsafe { &mut *out };
            // SAFETY: the caller passes a NUL-terminated `name`.
            let name = unsafe { CStr::from_ptr(name) };
            status(getenv(out, name))
        })
    }

    /// `git__online_cpus`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__online_cpus() -> c_int {
        barrier(online_cpus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strntol() {
        let _g = crate::util::test_support::init();
        assert_eq!(strntol64(b"  -42xyz", 10), Ok((-42, 5)));
        assert_eq!(strntol64(b"0x1F", 0), Ok((31, 4)));
        assert_eq!(strntol64(b"0x", 0), Ok((0, 1)));
        assert_eq!(strntol64(b"017", 0), Ok((15, 3)));
        assert_eq!(strntol64(b"9223372036854775807", 10), Ok((i64::MAX, 19)));
        assert_eq!(strntol64(b"-9223372036854775808", 10), Ok((i64::MIN, 20)));
        assert!(strntol64(b"9223372036854775808", 10).is_err());
        assert_eq!(
            crate::util::errors::last_message().unwrap(),
            b"failed to convert string to long: overflow error"
        );
        assert!(strntol64(b"   ", 10).is_err());
        assert_eq!(
            crate::util::errors::last_message().unwrap(),
            b"failed to convert string to long: not a number"
        );
        assert_eq!(
            parse_i64(b"99999999999999999999z", 10),
            (ParseOutcome::Overflow, 20)
        );
        assert!(strntol32(b" 2147483648", 10).is_err());
        assert_eq!(
            crate::util::errors::last_message().unwrap(),
            b"failed to convert: ' 2147483648' is too large"
        );
        assert_eq!(strntol32(b"-2147483648", 10), Ok((i32::MIN, 11)));
    }

    #[test]
    fn comparisons() {
        assert_eq!(strcasecmp(b"abc", b"ABC"), 0);
        assert!(strcasecmp(b"abc", b"abd") < 0);
        assert!(strcasecmp(b"ab", b"abc") < 0);
        assert!(strcasesort_cmp(b"Abc", b"abc") < 0);
        assert_eq!(strcasesort_cmp(b"abc", b"abc"), 0);
        assert!(strcasesort_cmp(b"abc", b"ABD") < 0);
        assert_eq!(strncasecmp(b"abcX", b"ABCy", 3), 0);
        assert_ne!(strncasecmp(b"abcX", b"ABCy", 0), 0);
        assert_eq!(prefixcmp(b"refs/heads/x", b"refs/"), 0);
        assert!(prefixcmp(b"ref", b"refs/") < 0);
        assert_eq!(prefixcmp(b"", b""), 0);
        assert_eq!(prefixncmp(b"refs/heads", 4, b"refs"), 0);
        assert!(prefixncmp(b"refs/heads", 3, b"refs") < 0);
        assert_eq!(prefixcmp_icase(b"REFS/x", b"refs/"), 0);
        assert_eq!(suffixcmp(b"file.txt", b".txt"), 0);
        assert_eq!(suffixcmp(b"a", b".txt"), -1);
        assert_eq!(suffixcmp_icase(b"FILE.TXT", b".txt"), 0);
        assert_eq!(strcmp(b"a", b"a"), 0);
        assert!(strcmp(b"a", b"ab") < 0);
        assert!(strcmp(b"\xff", b"a") > 0);
        assert_eq!(strlcmp(b"abc", b"ab"), c_int::from(b'c'));
    }

    #[test]
    fn misc() {
        assert_eq!(parse_bool(None), Some(true));
        assert_eq!(parse_bool(Some(b"YeS")), Some(true));
        assert_eq!(parse_bool(Some(b"")), Some(false));
        assert_eq!(parse_bool(Some(b"maybe")), None);
        let mut s = *b"a\\b\\\\c\\";
        let n = unescape(&mut s);
        assert_eq!(&s[..n], b"ab\\c\\");
        assert_eq!(linenlen(b"ab\ncd"), 3);
        assert_eq!(linenlen(b"abcd"), 4);
        assert_eq!(memmem(b"hello world", b"o w"), Some(4));
        assert_eq!(memmem(b"hello", b""), None);
        assert_eq!(memmem(b"aab", b"ab"), Some(1));
        assert_eq!(size_t_powerof2(5), 8);
        assert_eq!(size_t_powerof2(8), 8);
        assert_eq!(
            next_line(b"ab\r\n\ncd"),
            b"\r\n\ncd".split_at(0).1.get(3..).unwrap()
        );
        assert_eq!(bsearch(&[1, 3, 5], |e| 3 - e), Ok(1));
        assert_eq!(bsearch(&[1, 3, 5], |e| 4 - e), Err(2));
        assert_eq!(bsearch::<i32>(&[], |e| 4 - e), Err(0));
    }

    #[test]
    fn hash_matches_c() {
        // Values printed by the C `git__hash`.
        assert_eq!(hash(b"", 0), 0x9eb4_3457);
        assert_eq!(hash(b"hello world", 42), 0xb196_2265);
        assert_eq!(hash(b"abc", 0), 0x723a_6130);
        assert_eq!(hash(b"abcdefgh", 7), 0x5249_098f);
        assert_eq!(hash(b"ab", 1), 0x9ced_66e5);
    }

    #[test]
    fn tokens() {
        let mut b = *b"  a b,,c\0";
        let mut pos = 0;
        let t1 = strtok(&mut b, &mut pos, b" ,").unwrap();
        assert_eq!(&b[t1], b"a");
        let t2 = strtok(&mut b, &mut pos, b" ,").unwrap();
        assert_eq!(&b[t2], b"b");
        let t3 = strtok(&mut b, &mut pos, b" ,").unwrap();
        assert_eq!(&b[t3], b"c");
        assert!(strtok(&mut b, &mut pos, b" ,").is_none());

        let mut b = *b"a,,b\0";
        let mut pos = 0;
        assert_eq!(strsep(&mut b, &mut pos, b","), Some(0..1));
        assert_eq!(strsep(&mut b, &mut pos, b","), Some(2..2));
        assert_eq!(strsep(&mut b, &mut pos, b","), None);
        assert_eq!(pos, 3);
    }

    #[test]
    fn hexdump_format() {
        // The C `git__hexdump`'s output.
        assert_eq!(
            hexdump_string(b"0123456789abcdefXY"),
            "00000000  30 31 32 33 34 35 36 37 38  39 61 62 63 64 65 66  |0123456789abcdef|\n00000010  58 59                                             |XY|\n\n"
        );
        assert_eq!(
            hexdump_string(b"0123456789"),
            "00000000  30 31 32 33 34 35 36 37 38  39                    |0123456789|\n\n"
        );
        assert_eq!(
            hexdump_string(b"01234567"),
            "00000000  30 31 32 33 34 35 36 37                          |01234567|\n\n"
        );
    }
}
