//! Port of `src/util/utf8.c`: UTF-8 decoding (from utf8proc), as libgit2
//! uses it for path validation and display widths.
//!
//! The rules are the C's, which are stricter than `core::str`'s in one way:
//! the noncharacters U+FDD0..U+FDEF and U+xxFFFE/U+xxFFFF are invalid.
//!
//! ```
//! use libgit2_rs::util::utf8;
//! assert_eq!(utf8::iterate(b"\xc2\xa2!"), Some((0xa2, 2)));
//! assert_eq!(utf8::char_length(b"\x24\xc0\xc1\x34"), 4);
//! assert_eq!(utf8::valid_buf_length(b"ab\xffcd"), 2);
//! ```

/// `utf8proc_utf8class`: the length of a sequence from its first byte; 0
/// for a continuation byte or an invalid one.
static UTF8_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = match i {
            0x00..=0x7f => 1,
            0x80..=0xbf => 0,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 0,
        };
        i += 1;
    }
    t
};

/// `utf8_charlen`: the length of the sequence at the start of `s` if its
/// first byte starts one, it fits in `s` and its continuation bytes are
/// such; `None` otherwise (and for an empty `s`).
#[inline(always)]
pub fn charlen(s: &[u8]) -> Option<usize> {
    let first = *s.first()?;
    if first < 0x80 {
        return Some(1);
    }
    let length = UTF8_CLASS[first as usize] as usize;
    if length == 0 || length > s.len() {
        return None;
    }
    for &c in &s[1..length] {
        if c & 0xc0 != 0x80 {
            return None;
        }
    }
    Some(length)
}

/// The number of leading ASCII bytes of `s`, a multiple of 8 (a fast path
/// of the scans below).
#[inline(always)]
fn ascii_words(s: &[u8]) -> usize {
    let (chunks, _) = s.as_chunks::<8>();
    chunks
        .iter()
        .take_while(|c| u64::from_ne_bytes(**c) & 0x8080_8080_8080_8080 == 0)
        .count()
        * 8
}

/// `git_utf8_iterate`: the code point at the start of `s` and its length
/// in bytes; `None` if it is invalid (overlong, a surrogate, beyond
/// U+10FFFF, a noncharacter) or `s` is empty.
pub fn iterate(s: &[u8]) -> Option<(u32, usize)> {
    let length = charlen(s)?;
    let b = |i: usize| s[i] as u32;
    let uc = match length {
        1 => b(0),
        2 => {
            let uc = ((b(0) & 0x1f) << 6) + (b(1) & 0x3f);
            if uc < 0x80 { u32::MAX } else { uc }
        }
        3 => {
            let uc = ((b(0) & 0x0f) << 12) + ((b(1) & 0x3f) << 6) + (b(2) & 0x3f);
            if uc < 0x800 || (0xd800..0xe000).contains(&uc) || (0xfdd0..0xfdf0).contains(&uc) {
                u32::MAX
            } else {
                uc
            }
        }
        _ => {
            let uc = ((b(0) & 0x07) << 18)
                + ((b(1) & 0x3f) << 12)
                + ((b(2) & 0x3f) << 6)
                + (b(3) & 0x3f);
            if !(0x10000..0x110000).contains(&uc) {
                u32::MAX
            } else {
                uc
            }
        }
    };
    if (uc & 0xffff) >= 0xfffe {
        return None;
    }
    Some((uc, length))
}

/// `git_utf8_char_length`: the number of characters in `s`, each byte that
/// does not start a valid sequence counting as one.
pub fn char_length(s: &[u8]) -> usize {
    let mut offset = 0;
    let mut count = 0;
    while offset < s.len() {
        let rest = &s[offset..];
        if rest[0] < 0x80 {
            // each ASCII byte is a character
            let n = ascii_words(rest).max(1);
            offset += n;
            count += n;
            continue;
        }
        offset += charlen(rest).unwrap_or(1);
        count += 1;
    }
    count
}

/// `git_utf8_valid_buf_length`: the length of the longest prefix of `s`
/// made of whole sequences (the first invalid one ends it).
pub fn valid_buf_length(s: &[u8]) -> usize {
    let mut offset = 0;
    while offset < s.len() {
        let rest = &s[offset..];
        if rest[0] < 0x80 {
            offset += ascii_words(rest).max(1);
            continue;
        }
        match charlen(rest) {
            Some(n) => offset += n,
            None => break,
        }
    }
    offset
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `utf8.c`.
pub mod ffi {
    use core::ffi::{c_char, c_int};

    use super::*;
    use crate::util::export::bytes;

    /// `git_utf8_iterate`. With `str_len == 0`, C reads the sequence
    /// unbounded (stopping at the first byte that does not continue it, so
    /// never past a NUL); so does this.
    ///
    /// # Safety
    ///
    /// `out` is writable; `str` is readable for `str_len` bytes, or, when
    /// `str_len` is 0, up to the end of the sequence it starts (a
    /// NUL-terminated string suffices).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_utf8_iterate(
        out: *mut u32,
        str: *const c_char,
        str_len: usize,
    ) -> c_int {
        let mut buf = [0u8; 4];
        let s: &[u8] = if str_len > 0 {
            // SAFETY: `str_len` readable bytes.
            unsafe { bytes(str, str_len) }
        } else {
            // Read as C does: the first byte, then while continuation bytes
            // follow, up to the sequence's length.
            let p = str.cast::<u8>();
            // SAFETY: the first byte is readable (the caller's contract).
            buf[0] = unsafe { p.read() };
            let length = UTF8_CLASS[buf[0] as usize] as usize;
            let mut n = 1;
            while n < length {
                // SAFETY: the previous byte continued the sequence, so this
                // one is within what the caller guarantees.
                let c = unsafe { p.wrapping_add(n).read() };
                buf[n] = c;
                n += 1;
                if c & 0xc0 != 0x80 {
                    break;
                }
            }
            &buf[..n]
        };
        let (uc, len) = iterate(s).map_or((0, -1), |(uc, len)| (uc, len as c_int));
        // SAFETY: writable.
        unsafe { out.write(uc) };
        len
    }

    /// `git_utf8_char_length`.
    ///
    /// # Safety
    ///
    /// `str` is readable for `str_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_utf8_char_length(str: *const c_char, str_len: usize) -> usize {
        // SAFETY: `str_len` readable bytes.
        char_length(unsafe { bytes(str, str_len) })
    }

    /// `git_utf8_valid_buf_length`.
    ///
    /// # Safety
    ///
    /// `str` is readable for `str_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_utf8_valid_buf_length(
        str: *const c_char,
        str_len: usize,
    ) -> usize {
        // SAFETY: `str_len` readable bytes.
        valid_buf_length(unsafe { bytes(str, str_len) })
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
mod tests {
    use super::*;

    #[test]
    fn c_cases() {
        assert_eq!(char_length(b""), 0);
        assert_eq!(char_length(b"$"), 1);
        assert_eq!(char_length(b"abcde"), 5);
        assert_eq!(char_length(b"\xc2\xa2"), 1);
        assert_eq!(char_length(b"\x24\xc2\xa2"), 2);
        assert_eq!(char_length(b"\xf0\x90\x8d\x88"), 1);
        assert_eq!(char_length(b"\x24\xc2"), 2);
        assert_eq!(char_length(b"\x24\xc2\xc2\xa2"), 3);
        assert_eq!(char_length(b"\x24\xc0\xc0\x34"), 4);
        assert_eq!(char_length(b"\x24\xf5\xfd\xc2"), 4);
    }

    #[test]
    fn iterate_rules() {
        assert_eq!(iterate(b"a"), Some((97, 1)));
        assert_eq!(iterate(b"\xc0\x80"), None); // overlong
        assert_eq!(iterate(b"\xed\xa0\x80"), None); // surrogate
        assert_eq!(iterate(b"\xef\xb7\x90"), None); // U+FDD0
        assert_eq!(iterate(b"\xef\xbf\xbe"), None); // U+FFFE
        assert_eq!(iterate(b"\xf4\x90\x80\x80"), None); // > U+10FFFF
        assert_eq!(iterate(b"\xf0\x9f\x98\x80"), Some((0x1f600, 4)));
        assert_eq!(iterate(b"\xe2\x82"), None);
        assert_eq!(iterate(b""), None);
        assert_eq!(valid_buf_length(b"\xe2\x82\xac\xe2"), 3);
    }

    #[test]
    fn export_unbounded() {
        let mut out = 7u32;
        // SAFETY: NUL-terminated strings.
        unsafe {
            assert_eq!(ffi::git_utf8_iterate(&mut out, c"\xc2\xa2".as_ptr(), 0), 2);
            assert_eq!(out, 0xa2);
            assert_eq!(ffi::git_utf8_iterate(&mut out, c"\xe2\x82".as_ptr(), 0), -1);
            assert_eq!(out, 0);
            assert_eq!(ffi::git_utf8_iterate(&mut out, c"".as_ptr(), 0), 1);
        }
    }
}
