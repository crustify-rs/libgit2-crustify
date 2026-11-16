//! Rust equivalents of `src/util/ctype_compat.h`: the character classes
//! libgit2 uses, as in the "C" locale (which libgit2 runs in; see
//! `rust/DIVERGENCES.md`). Each takes a byte, as the C macros take
//! `(unsigned char)c`.

/// `git__tolower`.
#[inline(always)]
pub const fn tolower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

/// `git__toupper`.
#[inline(always)]
pub const fn toupper(c: u8) -> u8 {
    c.to_ascii_uppercase()
}

/// `git__isalpha`.
#[inline(always)]
pub const fn isalpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// `git__isdigit`.
#[inline(always)]
pub const fn isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// `git__isalnum`.
#[inline(always)]
pub const fn isalnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// `git__isspace`: space, `\t`, `\n`, `\v`, `\f`, `\r` (unlike
/// `u8::is_ascii_whitespace`, which leaves out `\v`).
#[inline(always)]
pub const fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `git__isxdigit`.
#[inline(always)]
pub const fn isxdigit(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// `git__isprint`: `' '` to `'~'`.
#[inline(always)]
pub const fn isprint(c: u8) -> bool {
    matches!(c, b' '..=b'~')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_match_the_c_locale() {
        let spaces: Vec<u8> = (0..=255u8).filter(|&c| isspace(c)).collect();
        assert_eq!(spaces, b"\t\n\x0b\x0c\r ");
        assert_eq!(tolower(b'Q'), b'q');
        assert_eq!(tolower(0xc4), 0xc4);
        assert!(isprint(b'~') && !isprint(0x7f));
        assert!(isxdigit(b'F') && !isxdigit(b'g'));
    }
}
