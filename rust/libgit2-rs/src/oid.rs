//! Port of `src/libgit2/oid.c` (and the inline helpers of `oid.h`): object
//! ids.
//!
//! [`Oid`] is `git_oid` (kind 3, mirrored: it is in the public header, and C
//! embeds it by value everywhere): one byte of type, then 32 bytes of id, of
//! which SHA1 uses the first 20 (the rest stay zero). It is `Copy`, ordered
//! and compared as C's `memcmp` of the whole struct, and is the oid type of
//! the whole port: a `*const git_oid` from C is a `*const Oid`.
//!
//! [`OidType`] is `git_oid_t` for the two known types. The type byte of an
//! `Oid` that comes from C may be anything (0 for `GIT_OID_NONE`, garbage
//! from a caller), so [`Oid::kind`] is an `Option`, and the functions that
//! depend on the size (formatting, prefix comparison) behave as the C does
//! for an unknown type.
//!
//! [`OidShortener`] is `git_oid_shorten` (kind 1, opaque: C only holds the
//! pointer).
//!
//! ```
//! use libgit2_rs::{Oid, OidType};
//! # let _g = libgit2_rs::__test::util::test_support::init();
//! let id = Oid::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391", OidType::Sha1)?;
//! assert_eq!(id, Oid::EMPTY_BLOB_SHA1);
//! assert_eq!(id.hex().as_str(), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
//! assert_eq!(id.hex_path()?.as_str(), "e6/9de29bb2d1d6434b8b29ae775ad8c2e48c5391");
//! let short = Oid::from_prefix(b"e69de", OidType::Sha1)?;
//! assert!(id.ncmp(&short, 5) == 0);
//! # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
//! ```

use core::ffi::{c_int, c_uint};
use core::fmt;
use core::marker::PhantomData;

use crate::util::errors::{self, ErrorClass, ErrorCode, Result};

/// `GIT_OID_SHA1_SIZE`.
pub const GIT_OID_SHA1_SIZE: usize = 20;
/// `GIT_OID_SHA1_HEXSIZE`.
pub const GIT_OID_SHA1_HEXSIZE: usize = GIT_OID_SHA1_SIZE * 2;
/// `GIT_OID_SHA256_SIZE`.
pub const GIT_OID_SHA256_SIZE: usize = 32;
/// `GIT_OID_SHA256_HEXSIZE`.
pub const GIT_OID_SHA256_HEXSIZE: usize = GIT_OID_SHA256_SIZE * 2;
/// `GIT_OID_MAX_SIZE`.
pub const GIT_OID_MAX_SIZE: usize = GIT_OID_SHA256_SIZE;
/// `GIT_OID_MAX_HEXSIZE`.
pub const GIT_OID_MAX_HEXSIZE: usize = GIT_OID_SHA256_HEXSIZE;
/// `GIT_OID_MINPREFIXLEN`.
pub const GIT_OID_MINPREFIXLEN: usize = 4;

/// `git_oid_t` as C passes it (an `enum`, 32 bits).
#[allow(non_camel_case_types)]
pub type git_oid_t = c_uint;
/// `GIT_OID_SHA1`.
pub const GIT_OID_SHA1: git_oid_t = 1;
/// `GIT_OID_SHA256`.
pub const GIT_OID_SHA256: git_oid_t = 2;

/// The type of an object id (`git_oid_t`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OidType {
    /// SHA1: 20 bytes.
    Sha1 = 1,
    /// SHA256: 32 bytes.
    Sha256 = 2,
}

impl OidType {
    /// `GIT_OID_DEFAULT`.
    pub const DEFAULT: OidType = OidType::Sha1;

    /// The type of C's `git_oid_t` value, if it is a known one
    /// (`git_oid_type_is_valid`).
    #[inline]
    pub const fn from_raw(t: git_oid_t) -> Option<OidType> {
        match t {
            GIT_OID_SHA1 => Some(OidType::Sha1),
            GIT_OID_SHA256 => Some(OidType::Sha256),
            _ => None,
        }
    }

    /// The `git_oid_t` value.
    #[inline]
    pub const fn raw(self) -> git_oid_t {
        self as git_oid_t
    }

    /// `git_oid_size`: the size of the raw id in bytes.
    #[inline]
    pub const fn size(self) -> usize {
        match self {
            OidType::Sha1 => GIT_OID_SHA1_SIZE,
            OidType::Sha256 => GIT_OID_SHA256_SIZE,
        }
    }

    /// `git_oid_hexsize`: the length of the hex form.
    #[inline]
    pub const fn hexsize(self) -> usize {
        self.size() * 2
    }

    /// `git_oid_type_name`.
    pub const fn name(self) -> &'static str {
        match self {
            OidType::Sha1 => "sha1",
            OidType::Sha256 => "sha256",
        }
    }

    /// `git_oid_type_fromstrn` / `git_oid_type_fromstr`: `"sha1"` or
    /// `"sha256"`.
    pub fn from_name(name: &[u8]) -> Option<OidType> {
        match name {
            b"sha1" => Some(OidType::Sha1),
            b"sha256" => Some(OidType::Sha256),
            _ => None,
        }
    }
}

impl fmt::Display for OidType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `git_oid_size` of a raw `git_oid_t`: 0 for an unknown type.
#[inline]
pub const fn oid_size(t: git_oid_t) -> usize {
    match OidType::from_raw(t) {
        Some(t) => t.size(),
        None => 0,
    }
}

/// `git_oid_hexsize` of a raw `git_oid_t`: 0 for an unknown type.
#[inline]
pub const fn oid_hexsize(t: git_oid_t) -> usize {
    oid_size(t) * 2
}

/// An object id: `git_oid`, mirrored.
///
/// Every byte pattern is a valid `Oid` (C can make any). The order is C's
/// `git_oid_cmp` (a `memcmp` of the type byte, then of the 32 id bytes).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Oid {
    kind: u8,
    id: [u8; GIT_OID_MAX_SIZE],
}

/// `git_oid`.
#[allow(non_camel_case_types)]
pub type git_oid = Oid;

// ---------------------------------------------------------------------------
// Hex tables
// ---------------------------------------------------------------------------

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// `git__fromhex` as a table: the digit's value, or -1.
static FROM_HEX: [i8; 256] = {
    let mut t = [-1i8; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = crate::util::util::fromhex(c as u8) as i8;
        c += 1;
    }
    t
};

/// The two hex digits of each byte.
static HEX_PAIRS: [[u8; 2]; 256] = {
    let mut t = [[0u8; 2]; 256];
    let mut b = 0;
    while b < 256 {
        t[b] = [HEX_DIGITS[b >> 4], HEX_DIGITS[b & 15]];
        b += 1;
    }
    t
};

#[inline(always)]
fn fromhex(c: u8) -> i8 {
    FROM_HEX[c as usize]
}

/// Parse `len` hex digits, read with `byte_at(i)` in order, into `id`,
/// which must be zero. Reads as C's loop does: one digit at a time, and
/// never past the first one that is not a hex digit. On such a digit at
/// `p`, `id` holds digits `0..p` (as C leaves it) and `Err(())`.
#[inline(always)]
fn parse_hex_into(
    id: &mut [u8; GIT_OID_MAX_SIZE],
    len: usize,
    mut byte_at: impl FnMut(usize) -> u8,
) -> core::result::Result<(), ()> {
    debug_assert!(len <= GIT_OID_MAX_HEXSIZE);
    let pairs = len / 2;
    for (i, out) in id.iter_mut().enumerate().take(pairs) {
        let hi = fromhex(byte_at(2 * i));
        if hi < 0 {
            return Err(());
        }
        let lo = fromhex(byte_at(2 * i + 1));
        if lo < 0 {
            *out = (hi as u8) << 4;
            return Err(());
        }
        *out = ((hi as u8) << 4) | lo as u8;
    }
    if len & 1 == 1 {
        let hi = fromhex(byte_at(len - 1));
        if hi < 0 {
            return Err(());
        }
        id[pairs] = (hi as u8) << 4;
    }
    Ok(())
}

fn oid_error_invalid(msg: &str) -> ErrorCode {
    crate::git_error_set!(ErrorClass::INVALID, "unable to parse OID - {msg}")
}

/// `git_oid_fmt_substr`: the hex digits `start..start + count` of `id` into
/// `out[..count]`.
#[inline]
fn fmt_substr(out: &mut [u8], id: &[u8; GIT_OID_MAX_SIZE], start: usize, count: usize) {
    let end = start + count;
    let mut o = 0;
    let mut i = start / 2;
    if start & 1 == 1 {
        out[o] = HEX_DIGITS[(id[i] & 0x0f) as usize];
        o += 1;
        i += 1;
    }
    let max = end / 2;
    if i < max {
        let pairs = &id[i..max];
        let dst = &mut out[o..o + 2 * pairs.len()];
        for (d, &b) in dst.as_chunks_mut::<2>().0.iter_mut().zip(pairs) {
            *d = HEX_PAIRS[b as usize];
        }
        o += 2 * pairs.len();
        i = max;
    }
    if end & 1 == 1 {
        out[o] = HEX_DIGITS[(id[i] >> 4) as usize];
    }
}

/// `git_oid_raw_ncmp`: whether the first `len` hex digits of two raw ids
/// differ (0 if equal, 1 otherwise); `len` is capped at
/// `GIT_OID_MAX_HEXSIZE`.
pub fn raw_ncmp(a: &[u8; GIT_OID_MAX_SIZE], b: &[u8; GIT_OID_MAX_SIZE], len: usize) -> c_int {
    let mut bits = len.min(GIT_OID_MAX_HEXSIZE) * 4;
    let (wa, wb) = (words(a), words(b));
    for i in 0..4 {
        if bits == 0 {
            break;
        }
        let mask = if bits >= 64 {
            !0u64
        } else {
            !0u64 << (64 - bits)
        };
        if (wa[i] ^ wb[i]) & mask != 0 {
            return 1;
        }
        bits = bits.saturating_sub(64);
    }
    0
}

/// The id as four big-endian words (so that their order is the bytes').
#[inline(always)]
fn words(id: &[u8; GIT_OID_MAX_SIZE]) -> [u64; 4] {
    let (c, _) = id.as_chunks::<8>();
    [
        u64::from_be_bytes(c[0]),
        u64::from_be_bytes(c[1]),
        u64::from_be_bytes(c[2]),
        u64::from_be_bytes(c[3]),
    ]
}

/// `git_oid__is_hexstr`: whether `s` (up to a NUL) is all hex digits and
/// exactly the hex size of `t`.
pub fn is_hexstr(s: &[u8], t: OidType) -> bool {
    let s = match memchr::memchr(0, s) {
        Some(n) => &s[..n],
        None => s,
    };
    s.iter().all(|&c| fromhex(c) >= 0) && s.len() == t.hexsize()
}

// ---------------------------------------------------------------------------
// Oid
// ---------------------------------------------------------------------------

impl Oid {
    /// `GIT_OID_NONE`: type 0, all zero.
    pub const NONE: Oid = Oid {
        kind: 0,
        id: [0; GIT_OID_MAX_SIZE],
    };

    /// `GIT_OID_SHA1_ZERO`.
    pub const SHA1_ZERO: Oid = Oid::zero(OidType::Sha1);

    /// `GIT_OID_SHA256_ZERO`.
    pub const SHA256_ZERO: Oid = Oid::zero(OidType::Sha256);

    /// `git_oid__empty_blob_sha1`: the id of the empty blob.
    pub const EMPTY_BLOB_SHA1: Oid = Oid::from_sha1([
        0xe6, 0x9d, 0xe2, 0x9b, 0xb2, 0xd1, 0xd6, 0x43, 0x4b, 0x8b, 0x29, 0xae, 0x77, 0x5a, 0xd8,
        0xc2, 0xe4, 0x8c, 0x53, 0x91,
    ]);

    /// `git_oid__empty_tree_sha1`: the id of the empty tree.
    pub const EMPTY_TREE_SHA1: Oid = Oid::from_sha1([
        0x4b, 0x82, 0x5d, 0xc6, 0x42, 0xcb, 0x6e, 0xb9, 0xa0, 0x60, 0xe5, 0x4b, 0xf8, 0xd6, 0x92,
        0x88, 0xfb, 0xee, 0x49, 0x04,
    ]);

    /// `git_oid_clear`: the all-zero id of type `t`.
    #[inline]
    pub const fn zero(t: OidType) -> Oid {
        Oid {
            kind: t as u8,
            id: [0; GIT_OID_MAX_SIZE],
        }
    }

    /// An id from its raw fields, whatever they are (as C may make one).
    #[inline]
    pub const fn from_raw_parts(kind: u8, id: [u8; GIT_OID_MAX_SIZE]) -> Oid {
        Oid { kind, id }
    }

    /// A SHA1 id from its 20 bytes.
    #[inline]
    pub const fn from_sha1(raw: [u8; GIT_OID_SHA1_SIZE]) -> Oid {
        let mut id = [0; GIT_OID_MAX_SIZE];
        let mut i = 0;
        while i < GIT_OID_SHA1_SIZE {
            id[i] = raw[i];
            i += 1;
        }
        Oid {
            kind: OidType::Sha1 as u8,
            id,
        }
    }

    /// A SHA256 id from its 32 bytes.
    #[inline]
    pub const fn from_sha256(raw: [u8; GIT_OID_SHA256_SIZE]) -> Oid {
        Oid {
            kind: OidType::Sha256 as u8,
            id: raw,
        }
    }

    /// `git_oid_from_raw`: an id of type `t` from the first `t.size()`
    /// bytes of `raw`; `None` if `raw` is shorter.
    #[inline]
    pub fn from_bytes(raw: &[u8], t: OidType) -> Option<Oid> {
        let raw = raw.get(..t.size())?;
        let mut id = [0; GIT_OID_MAX_SIZE];
        id[..raw.len()].copy_from_slice(raw);
        Some(Oid { kind: t as u8, id })
    }

    /// `git_oid_from_prefix`: parse the hex digits of `s` (between 1 and
    /// `t.hexsize()` of them); the rest of the id is zero.
    ///
    /// Errors (class `GIT_ERROR_INVALID`) as C's: `unable to parse OID -
    /// too short`, `- too long`, `- contains invalid characters`.
    pub fn from_prefix(s: &[u8], t: OidType) -> Result<Oid> {
        let mut out = Oid::NONE;
        prefix_checks(s.len(), t.hexsize())?;
        out.kind = t as u8;
        match parse_hex_into(&mut out.id, s.len(), |i| s[i]) {
            Ok(()) => Ok(out),
            Err(()) => Err(oid_error_invalid("contains invalid characters")),
        }
    }

    /// `git_oid_from_string`: parse exactly `t.hexsize()` hex digits.
    ///
    /// The slice is the C string's bytes: if it is shorter, the error is
    /// C's for the NUL it would reach (`contains invalid characters`); if
    /// it is longer, `too long`.
    pub fn from_hex(s: &[u8], t: OidType) -> Result<Oid> {
        let hexsize = t.hexsize();
        let mut out = Oid::zero(t);
        // C reads the digits up to the first invalid one, which for a short
        // C string is its NUL.
        let r = parse_hex_into(&mut out.id, hexsize, |i| s.get(i).copied().unwrap_or(0));
        if r.is_err() {
            return Err(oid_error_invalid("contains invalid characters"));
        }
        if s.len() > hexsize && s[hexsize] != 0 {
            return Err(oid_error_invalid("too long"));
        }
        Ok(out)
    }

    /// [`from_hex`](Oid::from_hex) of a `&str`.
    pub fn from_str_hex(s: &str, t: OidType) -> Result<Oid> {
        Oid::from_hex(s.as_bytes(), t)
    }

    /// The type, if it is a known one (`git_oid_type`).
    #[inline]
    pub const fn kind(&self) -> Option<OidType> {
        OidType::from_raw(self.kind as git_oid_t)
    }

    /// The raw type byte.
    #[inline]
    pub const fn raw_kind(&self) -> u8 {
        self.kind
    }

    /// All 32 id bytes (for SHA1, the last 12 are normally zero).
    #[inline]
    pub const fn raw_id(&self) -> &[u8; GIT_OID_MAX_SIZE] {
        &self.id
    }

    /// The id bytes, mutable (all 32).
    #[inline]
    pub fn raw_id_mut(&mut self) -> &mut [u8; GIT_OID_MAX_SIZE] {
        &mut self.id
    }

    /// The `size()` id bytes of the type (empty for an unknown type).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.id[..self.size()]
    }

    /// `git_oid_size(git_oid_type(oid))`: 0 for an unknown type.
    #[inline]
    pub const fn size(&self) -> usize {
        oid_size(self.kind as git_oid_t)
    }

    /// `git_oid_hexsize(git_oid_type(oid))`: 0 for an unknown type.
    #[inline]
    pub const fn hexsize(&self) -> usize {
        self.size() * 2
    }

    /// `git_oid_is_zero`: whether the 32 id bytes are zero (whatever the
    /// type).
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.id == [0; GIT_OID_MAX_SIZE]
    }

    /// `git_oid_cmp`: C's `memcmp` of the two structs, with glibc's
    /// result: the difference of the first differing bytes.
    #[inline]
    pub fn cmp_raw(&self, other: &Oid) -> c_int {
        if self.kind != other.kind {
            return self.kind as c_int - other.kind as c_int;
        }
        let (a, b) = (words(&self.id), words(&other.id));
        for i in 0..4 {
            let x = a[i] ^ b[i];
            if x != 0 {
                let at = i * 8 + (x.leading_zeros() / 8) as usize;
                return self.id[at] as c_int - other.id[at] as c_int;
            }
        }
        0
    }

    /// `git_oid_ncmp`: 0 if the first `len` hex digits are equal; the
    /// difference of the types if they differ, 1 otherwise.
    #[inline]
    pub fn ncmp(&self, other: &Oid, len: usize) -> c_int {
        if self.kind != other.kind {
            return self.kind as c_int - other.kind as c_int;
        }
        raw_ncmp(&self.id, &other.id, len)
    }

    /// `git_oid_strcmp`: compare with the hex string `s` (up to a NUL or
    /// its end). A shorter string compares as its prefix (an odd last digit
    /// as the high nibble of a byte whose low nibble is 0); an invalid digit
    /// gives -1; otherwise the difference of the first differing bytes.
    pub fn strcmp(&self, s: &[u8]) -> c_int {
        self.strcmp_with(|i| s.get(i).copied().unwrap_or(0))
    }

    /// [`strcmp`](Oid::strcmp) reading the string with `byte_at`, in order
    /// and not past its first NUL (as C reads it).
    #[inline(always)]
    fn strcmp_with(&self, mut byte_at: impl FnMut(usize) -> u8) -> c_int {
        let size = self.size();
        let mut p = 0;
        for &byte in &self.id[..size] {
            let c = byte_at(p);
            if c == 0 {
                break;
            }
            p += 1;
            let hv = fromhex(c);
            if hv < 0 {
                return -1;
            }
            let mut strval = (hv as u8) << 4;
            let c = byte_at(p);
            if c != 0 {
                p += 1;
                let hv = fromhex(c);
                if hv < 0 {
                    return -1;
                }
                strval |= hv as u8;
            }
            if byte != strval {
                return byte as c_int - strval as c_int;
            }
            if c == 0 {
                break;
            }
        }
        0
    }

    /// `git_oid_streq`: whether [`strcmp`](Oid::strcmp) is 0.
    #[inline]
    pub fn streq(&self, s: &[u8]) -> bool {
        self.strcmp(s) == 0
    }

    /// `git_oid__cpy_prefix`: this id cut to its first `len` hex digits
    /// (`len` is capped at the maximum), the rest zero.
    pub fn prefix(&self, len: usize) -> Oid {
        let len = len.min(GIT_OID_MAX_HEXSIZE);
        let mut out = Oid {
            kind: self.kind,
            id: [0; GIT_OID_MAX_SIZE],
        };
        let n = len.div_ceil(2);
        out.id[..n].copy_from_slice(&self.id[..n]);
        if len & 1 == 1 {
            out.id[len / 2] &= 0xf0;
        }
        out
    }

    /// The hash of `hashmap_oid.h` (`git_hashmap_oid_hashcode`): the first
    /// four id bytes as a native-endian `u32`.
    #[inline]
    pub fn hashcode(&self) -> u32 {
        u32::from_ne_bytes([self.id[0], self.id[1], self.id[2], self.id[3]])
    }

    /// `git_oid_nfmt`: the first `out.len()` hex digits into `out`; bytes
    /// beyond the hex size are set to 0. Error (`unknown type`) for an
    /// unknown type, with `out` untouched.
    pub fn nfmt(&self, out: &mut [u8]) -> Result<()> {
        let hex_size = self.hexsize();
        if hex_size == 0 {
            return Err(oid_error_invalid("unknown type"));
        }
        let mut n = out.len();
        if n > hex_size {
            out[hex_size..].fill(0);
            n = hex_size;
        }
        fmt_substr(out, &self.id, 0, n);
        Ok(())
    }

    /// `git_oid_fmt_substr`: hex digits `start..start + out.len()` into
    /// `out`. Panics if the range exceeds `GIT_OID_MAX_HEXSIZE`.
    pub fn fmt_substr(&self, out: &mut [u8], start: usize) {
        assert!(
            start + out.len() <= GIT_OID_MAX_HEXSIZE,
            "oid digit range out of bounds"
        );
        let count = out.len();
        fmt_substr(out, &self.id, start, count);
    }

    /// The full hex form (`git_oid_fmt`), empty for an unknown type.
    #[inline]
    pub fn hex(&self) -> HexOid {
        let mut h = HexOid::new();
        let n = self.hexsize();
        fmt_substr(&mut h.buf[..n], &self.id, 0, n);
        h.len = n as u8;
        h
    }

    /// `git_oid_tostr` into a buffer of `n` bytes: the first `n - 1` hex
    /// digits (at most the hex size); empty for `n == 0` or an unknown type
    /// (which also sets C's `unknown type` error, as `git_oid_tostr` does).
    pub fn hex_prefix(&self, n: usize) -> HexOid {
        let mut h = HexOid::new();
        if n == 0 {
            return h;
        }
        let hex_size = self.hexsize();
        let n = n.min(hex_size + 1) - 1;
        if hex_size == 0 {
            let _ = oid_error_invalid("unknown type");
            return h;
        }
        fmt_substr(&mut h.buf[..n], &self.id, 0, n);
        h.len = n as u8;
        h
    }

    /// `git_oid_pathfmt`: `"xx/yyyy…"`, the loose object path. Error
    /// (`unknown type`) for an unknown type.
    pub fn hex_path(&self) -> Result<HexOid> {
        let hex_size = self.hexsize();
        if hex_size == 0 {
            return Err(oid_error_invalid("unknown type"));
        }
        let mut h = HexOid::new();
        write_path(&mut h.buf[..hex_size + 1], &self.id, hex_size);
        h.len = (hex_size + 1) as u8;
        Ok(h)
    }
}

fn write_path(out: &mut [u8], id: &[u8; GIT_OID_MAX_SIZE], hex_size: usize) {
    out[..2].copy_from_slice(&HEX_PAIRS[id[0] as usize]);
    out[2] = b'/';
    fmt_substr(&mut out[3..], id, 2, hex_size - 2);
}

/// The length checks of `git_oid_from_prefix`, in C's order.
#[inline]
fn prefix_checks(len: usize, hexsize: usize) -> Result<()> {
    if hexsize == 0 {
        return Err(oid_error_invalid("unknown type"));
    }
    if len == 0 {
        return Err(oid_error_invalid("too short"));
    }
    if len > hexsize {
        return Err(oid_error_invalid("too long"));
    }
    Ok(())
}

impl fmt::Display for Oid {
    /// The hex form (nothing for an unknown type).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.hex().as_str())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            Some(t) => write!(f, "Oid({t}:{})", self.hex().as_str()),
            None => write!(f, "Oid(type {}:{:02x?})", self.kind, &self.id[..]),
        }
    }
}

/// The hex form of an oid, on the stack: up to 65 bytes (a path form of a
/// SHA256 id), always followed by a NUL.
#[derive(Clone, Copy)]
pub struct HexOid {
    buf: [u8; GIT_OID_MAX_HEXSIZE + 2],
    len: u8,
}

impl HexOid {
    const fn new() -> HexOid {
        HexOid {
            buf: [0; GIT_OID_MAX_HEXSIZE + 2],
            len: 0,
        }
    }

    /// The digits.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    /// The digits, as `str` (they are ASCII).
    #[inline]
    pub fn as_str(&self) -> &str {
        // The buffer only ever holds hex digits and '/'.
        core::str::from_utf8(self.as_bytes()).unwrap_or_default()
    }

    /// The digits with their NUL.
    #[inline]
    pub fn as_c_str(&self) -> &core::ffi::CStr {
        core::ffi::CStr::from_bytes_until_nul(&self.buf).unwrap_or_default()
    }
}

impl AsRef<[u8]> for HexOid {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Display for HexOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for HexOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

// ---------------------------------------------------------------------------
// The shortener
// ---------------------------------------------------------------------------

/// `node_index`.
type NodeIndex = i16;

/// `SHRT_MAX`: the node count at which the set is full.
const MAX_NODES: usize = i16::MAX as usize;

/// A leaf's tail: the rest of the id it was made for, borrowed from the
/// caller (as C's `const char *tail`): at most `len` bytes, read as
/// [`OidShortener::add_raw`] reads its id. It always starts right after a
/// hex digit of the id, so its first byte (when `len > 0`) is readable.
#[repr(C)]
#[derive(Clone, Copy)]
struct Tail {
    ptr: *const u8,
    len: usize,
}

/// A trie node: C's union of 16 children (an index per hex digit; 0 none,
/// < 0 a leaf) and a leaf's tail. As in C, whether a node is a leaf is
/// known from the negative index that leads to it (the trie's invariant:
/// a leaf is only read as a tail, any other node only as children), and a
/// converted leaf's children are all zero.
#[repr(C)]
#[derive(Clone, Copy)]
union TrieNode {
    children: [NodeIndex; 16],
    tail: Tail,
}

const EMPTY_NODE: TrieNode = TrieNode { children: [0; 16] };

/// `git_oid_shorten`: the shortest prefix length that tells a set of SHA1
/// hex ids apart.
///
/// The algorithm, its memory layout (32-byte nodes), its limits (about
/// 32000 ids: `OID set full`) and its results are C's. As in C, the ids are
/// not copied: the shortener borrows them for `'a`.
///
/// ```
/// use libgit2_rs::OidShortener;
/// # let _g = libgit2_rs::__test::util::test_support::init();
/// let mut s = OidShortener::new(0);
/// s.add(b"22596363b3de40b06f981fb85d82312e8c0ed511")?;
/// assert_eq!(s.add(b"22596363b3de40b06f981fb85d82312e8c0ed512")?, 40);
/// # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
/// ```
pub struct OidShortener<'a> {
    nodes: Vec<TrieNode>,
    /// The ids, which the leaves' tails borrow.
    _ids: PhantomData<&'a [u8]>,
    min_length: c_int,
    full: bool,
}

impl fmt::Debug for OidShortener<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidShortener")
            .field("nodes", &self.nodes.len())
            .field("min_length", &self.min_length)
            .field("full", &self.full)
            .finish()
    }
}

#[cold]
#[inline(never)]
fn shorten_error(msg: &str) -> ErrorCode {
    crate::git_error_set!(ErrorClass::INVALID, "unable to shorten OID - {msg}")
}

#[cold]
#[inline(never)]
fn shorten_oom() -> ErrorCode {
    errors::oom()
}

impl<'a> OidShortener<'a> {
    /// `git_oid_shorten_new`: a shortener whose results are at least
    /// `min_length` (capped at `c_int::MAX`).
    pub fn new(min_length: usize) -> OidShortener<'a> {
        let mut nodes = Vec::with_capacity(16);
        nodes.push(EMPTY_NODE);
        OidShortener {
            nodes,
            _ids: PhantomData,
            min_length: c_int::try_from(min_length).unwrap_or(c_int::MAX),
            full: false,
        }
    }

    /// The current minimal length (what `git_oid_shorten_add(os, NULL)`
    /// returns).
    pub fn min_length(&self) -> usize {
        self.min_length as usize
    }

    /// The children of the normal node `idx`.
    #[inline(always)]
    fn children(&mut self, idx: usize) -> &mut [NodeIndex; 16] {
        // SAFETY: the trie's invariant: `idx` is reached through a
        // non-negative index, so it is a normal node. A leaf's link is made
        // positive only together with its conversion: in the same
        // iteration of `add`'s walk when the next digit is valid, or by
        // `add`'s error path, which empties the leaf (see there). Every node
        // is fully initialized (leaves start as `EMPTY_NODE`), and every bit
        // pattern is a valid `[i16; 16]`.
        unsafe { &mut self.nodes[idx].children }
    }

    /// Turn the leaf `idx` into a normal node with no children; its tail.
    #[inline(always)]
    fn take_tail(&mut self, idx: usize) -> Tail {
        let node = &mut self.nodes[idx];
        // SAFETY: the trie's invariant: `idx` was reached through a negative
        // index, so it is a leaf, whose `tail` was written by `push_leaf`.
        let t = unsafe { node.tail };
        // A leaf starts as `EMPTY_NODE` and only its tail is written: zeroing
        // the tail makes the whole node `EMPTY_NODE` again.
        node.tail = Tail {
            ptr: core::ptr::null(),
            len: 0,
        };
        t
    }

    /// `push_leaf`: a new leaf under `idx` at digit `push_at`, for the
    /// tail `tail`. `Err` when the set became full.
    #[inline(always)]
    fn push_leaf(&mut self, idx: usize, push_at: usize, tail: Tail) -> Result<()> {
        if self.nodes.try_reserve(1).is_err() {
            return Err(shorten_oom());
        }
        let idx_leaf = self.nodes.len();
        if idx_leaf + 1 == MAX_NODES {
            self.nodes.push(EMPTY_NODE);
            self.full = true;
            return Err(shorten_error("OID set full"));
        }
        // All 32 bytes initialized first, then the 16 of the tail.
        let mut leaf = EMPTY_NODE;
        leaf.tail = tail;
        self.nodes.push(leaf);
        self.children(idx)[push_at] = -(idx_leaf as NodeIndex);
        Ok(())
    }

    /// `git_oid_shorten_add`: add the 40-digit hex id `text_id` (read up to
    /// a NUL); the minimal length that tells every id added so far apart.
    ///
    /// Errors (class `GIT_ERROR_INVALID`): `unable to shorten OID - invalid
    /// hex value`, `- OID set full`.
    pub fn add(&mut self, text_id: &'a [u8]) -> Result<usize> {
        let len = text_id.len().min(GIT_OID_SHA1_HEXSIZE);
        // SAFETY: the `len` bytes of the slice are readable for `'a`.
        unsafe { self.add_raw(text_id.as_ptr(), len) }
    }

    /// [`add`](OidShortener::add) of the id at `p`, of at most `len`
    /// bytes, read lazily, as C reads it: byte `i` only after bytes
    /// `0..i` were hex digits (so never past a NUL), and the bytes after
    /// a leaf's position only when that leaf is split later.
    ///
    /// # Safety
    ///
    /// For every `i < len` such that bytes `0..i` are not NUL, byte `i` is
    /// readable, and stays so and unwritten for `'a` (a slice of `len`
    /// bytes, or a NUL-terminated string: C's contract).
    #[inline(always)]
    unsafe fn add_raw(&mut self, p: *const u8, len: usize) -> Result<usize> {
        if self.full {
            return Err(shorten_error("OID set full"));
        }
        let mut idx: usize = 0;
        let mut is_leaf = false;
        let mut i = 0;
        while i < GIT_OID_SHA1_HEXSIZE {
            // Byte `i`: bytes `0..i` were hex digits (the walk got here).
            let c = if i < len {
                // SAFETY: the contract, as just said.
                fromhex(unsafe { p.wrapping_add(i).read() })
            } else {
                -1
            };
            if c < 0 {
                if is_leaf {
                    // The leaf's link was made positive (as C does) but the
                    // leaf is not converted. C leaves the node holding its
                    // tail pointer, later read as children (pointer bits as
                    // indices for digits 0..3, zero for 4..f): DEFECTS.md.
                    // Here it becomes an empty normal node, which is C's
                    // result for 4..f; the leaf's id is dropped, as in C.
                    self.nodes[idx] = EMPTY_NODE;
                }
                return Err(shorten_error("invalid hex value"));
            }
            let c = (c & 15) as usize;

            if is_leaf {
                let t = self.take_tail(idx);
                // C reads the tail's next digit unchecked (and indexes the
                // children with -1 when it is not one).
                let tc = if t.len > 0 {
                    // SAFETY: a tail starts right after a hex digit of its
                    // id (`push_leaf`'s callers), so its first byte is
                    // readable by the contract of the `add_raw` that made it,
                    // for `'a`.
                    fromhex(unsafe { t.ptr.read() })
                } else {
                    -1
                };
                if tc < 0 {
                    return Err(shorten_error("invalid hex value"));
                }
                let rest = Tail {
                    ptr: t.ptr.wrapping_add(1),
                    len: t.len - 1,
                };
                self.push_leaf(idx, (tc & 15) as usize, rest)?;
            }

            let next = self.children(idx)[c];
            if next == 0 {
                let rest = Tail {
                    ptr: p.wrapping_add(i + 1),
                    len: len - i - 1,
                };
                self.push_leaf(idx, c, rest)?;
                break;
            }
            is_leaf = next < 0;
            if is_leaf {
                self.children(idx)[c] = -next;
            }
            idx = next.unsigned_abs() as usize;
            i += 1;
        }

        let i = (i + 1) as c_int;
        if i > self.min_length {
            self.min_length = i;
        }
        Ok(self.min_length as usize)
    }
}

// SAFETY: the tails are shared borrows of `[u8]` (`Sync`), so the shortener
// may move to, and be read from, another thread.
unsafe impl Send for OidShortener<'_> {}
// SAFETY: as above; `&OidShortener` only reads.
unsafe impl Sync for OidShortener<'_> {}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `oid.c`.
pub mod ffi {
    use core::cell::UnsafeCell;
    use core::ffi::{c_char, c_int, c_uchar};
    use core::ptr;

    use super::*;
    use crate::git_assert_arg;
    use crate::util::alloc;
    use crate::util::export::{barrier, status};

    /// `git_oid__empty_blob_sha1`.
    #[unsafe(no_mangle)]
    pub static git_oid__empty_blob_sha1: Oid = Oid::EMPTY_BLOB_SHA1;

    /// `git_oid__empty_tree_sha1`.
    #[unsafe(no_mangle)]
    pub static git_oid__empty_tree_sha1: Oid = Oid::EMPTY_TREE_SHA1;

    /// `git_oid_from_prefix`, reading `str` as C does: digit by digit, and
    /// never past the first that is not one.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `str` is NULL or readable up to its first
    /// non-hex byte or `len` bytes, whichever comes first.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_from_prefix(
        out: *mut Oid,
        str: *const c_char,
        len: usize,
        type_: git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!str.is_null(), "str", -1);
            // SAFETY: the caller's contract.
            status(unsafe { from_prefix_raw(out, str.cast(), len, type_) })
        })
    }

    /// The body of `git_oid_from_prefix`.
    ///
    /// # Safety
    ///
    /// As [`git_oid_from_prefix`], with both pointers non-null.
    #[inline(always)]
    unsafe fn from_prefix_raw(out: *mut Oid, s: *const u8, len: usize, t: git_oid_t) -> Result<()> {
        prefix_checks(len, oid_hexsize(t))?;
        let mut oid = Oid {
            kind: t as u8,
            id: [0; GIT_OID_MAX_SIZE],
        };
        // SAFETY: `i < len`, and `parse_hex_into` reads in order and stops
        // at the first non-hex byte: the caller's guarantee covers it.
        let r = parse_hex_into(&mut oid.id, len, |i| unsafe { s.wrapping_add(i).read() });
        // SAFETY: `out` is writable (the caller's contract); C leaves the
        // partial result there on failure too.
        unsafe { out.write(oid) };
        r.map_err(|()| oid_error_invalid("contains invalid characters"))
    }

    /// `git_oid_from_string`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `str` is NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_from_string(
        out: *mut Oid,
        str: *const c_char,
        type_: git_oid_t,
    ) -> c_int {
        barrier(|| {
            let hexsize = oid_hexsize(type_);
            if hexsize == 0 {
                return oid_error_invalid("unknown type").raw();
            }
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!str.is_null(), "str", -1);
            // SAFETY: a NUL-terminated string is readable up to its NUL,
            // which is not a hex digit.
            if let Err(e) = unsafe { from_prefix_raw(out, str.cast(), hexsize, type_) } {
                return e.raw();
            }
            // SAFETY: the `hexsize` bytes before were digits, so not the NUL:
            // this byte is within the string.
            if unsafe { str.wrapping_add(hexsize).read() } != 0 {
                return oid_error_invalid("too long").raw();
            }
            0
        })
    }

    /// `git_oid_from_raw`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `raw` is readable for the type's size.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_from_raw(
        out: *mut Oid,
        raw: *const c_uchar,
        type_: git_oid_t,
    ) -> c_int {
        barrier(|| {
            let Some(t) = OidType::from_raw(type_) else {
                return oid_error_invalid("unknown type").raw();
            };
            // Written in place, as C does (no temporary: building the
            // oid on the stack and copying it out costs a memset call, a
            // memcpy call and a store-forwarding stall per id, which
            // dominates pack.c's foreach loop over a cold index).
            let n = t.size();
            // SAFETY: `out` is writable (an `Oid`: a type byte and
            // GIT_OID_MAX_SIZE id bytes, raw places, no reference formed);
            // `raw` is readable for `n <= GIT_OID_MAX_SIZE` bytes and does
            // not overlap `out` (the C contract).
            let kind = unsafe { &raw mut (*out).kind };
            // SAFETY: as above.
            unsafe { kind.write(t as u8) };
            // SAFETY: as above.
            let id = unsafe { (&raw mut (*out).id).cast::<u8>() };
            // SAFETY: as above.
            unsafe { core::ptr::copy_nonoverlapping(raw, id, n) };
            // SAFETY: as above: `n..GIT_OID_MAX_SIZE` is within the id.
            unsafe { core::ptr::write_bytes(id.wrapping_add(n), 0, GIT_OID_MAX_SIZE - n) };
            0
        })
    }

    /// `git_oid_nfmt`.
    ///
    /// # Safety
    ///
    /// `str` is writable for `n` bytes (it may be uninitialized; not
    /// aliasing `oid`); `oid` is NULL or a valid oid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_nfmt(str: *mut c_char, n: usize, oid: *const Oid) -> c_int {
        barrier(|| {
            let dst = str.cast::<u8>();
            // SAFETY: NULL or a valid oid.
            let Some(oid) = (unsafe { oid.as_ref() }) else {
                // SAFETY: `n` writable bytes (C's output buffers may be
                // uninitialized: they are only written, through the pointer).
                unsafe { ptr::write_bytes(dst, 0, n) };
                return 0;
            };
            let hex_size = oid.hexsize();
            if hex_size == 0 {
                return oid_error_invalid("unknown type").raw();
            }
            let k = n.min(hex_size);
            let mut buf = [0u8; GIT_OID_MAX_HEXSIZE];
            fmt_substr(&mut buf[..k], &oid.id, 0, k);
            // SAFETY: `k <= n` writable bytes, not overlapping the local.
            unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), dst, k) };
            if n > hex_size {
                // SAFETY: the rest of the `n` bytes.
                unsafe { ptr::write_bytes(dst.wrapping_add(hex_size), 0, n - hex_size) };
            }
            0
        })
    }

    /// `git_oid_fmt`: the hex digits, without a NUL.
    ///
    /// # Safety
    ///
    /// `oid` is a valid oid; `str` is writable for its hex size (it may be
    /// uninitialized).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_fmt(str: *mut c_char, oid: *const Oid) -> c_int {
        barrier(|| {
            // SAFETY: a valid oid.
            let oid = unsafe { &*oid };
            let hex_size = oid.hexsize();
            if hex_size == 0 {
                return oid_error_invalid("unknown type").raw();
            }
            let mut buf = [0u8; GIT_OID_MAX_HEXSIZE];
            fmt_substr(&mut buf[..hex_size], &oid.id, 0, hex_size);
            // SAFETY: `hex_size` writable bytes, not overlapping the local.
            unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), str.cast::<u8>(), hex_size) };
            0
        })
    }

    /// `git_oid_pathfmt`: `"xx/yyyy…"`, without a NUL.
    ///
    /// # Safety
    ///
    /// `oid` is a valid oid; `str` is writable for its hex size + 1 (it may
    /// be uninitialized).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_pathfmt(str: *mut c_char, oid: *const Oid) -> c_int {
        barrier(|| {
            // SAFETY: a valid oid.
            let oid = unsafe { &*oid };
            let hex_size = oid.hexsize();
            if hex_size == 0 {
                return oid_error_invalid("unknown type").raw();
            }
            let mut buf = [0u8; GIT_OID_MAX_HEXSIZE + 1];
            write_path(&mut buf[..hex_size + 1], &oid.id, hex_size);
            // SAFETY: `hex_size + 1` writable bytes, not overlapping the local.
            unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), str.cast::<u8>(), hex_size + 1) };
            0
        })
    }

    /// `git_oid_global_init`. The thread's buffer of `git_oid_tostr_s` is a
    /// Rust thread-local, which needs no setup.
    ///
    /// # Safety
    ///
    /// None (called by `git_libgit2_init`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_global_init() -> c_int {
        0
    }

    std::thread_local! {
        /// The buffer of `git_oid_tostr_s`: no destructor, so it stays
        /// usable until the thread ends.
        static TOSTR_BUF: UnsafeCell<[u8; GIT_OID_MAX_HEXSIZE + 1]> =
            const { UnsafeCell::new([0; GIT_OID_MAX_HEXSIZE + 1]) };
    }

    /// `git_oid_tostr_s`: the hex form in a per-thread buffer, valid until
    /// the thread's next call.
    ///
    /// # Safety
    ///
    /// `oid` is a valid oid. The result is overwritten by the next call on
    /// the same thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_tostr_s(oid: *const Oid) -> *mut c_char {
        barrier(|| {
            // SAFETY: a valid oid.
            let oid = unsafe { &*oid };
            let p = TOSTR_BUF.with(|b| b.get().cast::<u8>());
            // SAFETY: the thread's own buffer, which only this function
            // writes, and which C reads only between calls (its contract);
            // no reference to it outlives this block.
            let buf = unsafe { &mut *p.cast::<[u8; GIT_OID_MAX_HEXSIZE + 1]>() };
            // `git_oid_nfmt(str, hexsize + 1, oid)`: an unknown type sets the
            // error and leaves the buffer.
            let _ = oid.nfmt(&mut buf[..oid.hexsize() + 1]);
            p.cast()
        })
    }

    /// `git_oid_allocfmt`: the hex form in a new `git__malloc` string;
    /// NULL for an unknown type or on allocation failure.
    ///
    /// # Safety
    ///
    /// `oid` is a valid oid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_allocfmt(oid: *const Oid) -> *mut c_char {
        barrier(|| {
            // SAFETY: a valid oid.
            let oid = unsafe { &*oid };
            if oid.hexsize() == 0 {
                return ptr::null_mut();
            }
            alloc::substrdup(oid.hex().as_bytes())
        })
    }

    /// `git_oid_tostr`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable for `n` bytes; `oid` is NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_tostr(
        out: *mut c_char,
        n: usize,
        oid: *const Oid,
    ) -> *mut c_char {
        barrier(|| {
            if out.is_null() || n == 0 {
                return c"".as_ptr().cast_mut();
            }
            // SAFETY: NULL or a valid oid.
            let oid = unsafe { oid.as_ref() };
            let hex_size = oid.map_or(0, Oid::hexsize);
            let n = n.min(hex_size + 1);
            let mut buf = [0u8; GIT_OID_MAX_HEXSIZE + 1];
            match oid {
                Some(oid) if hex_size == 0 => {
                    // `git_oid_nfmt(out, 0, oid)`: the unknown type's error
                    let _ = oid.nfmt(&mut buf[..0]);
                }
                Some(oid) => fmt_substr(&mut buf[..n - 1], &oid.id, 0, n - 1),
                None => {}
            }
            // `buf[n - 1]` is the NUL.
            // SAFETY: `n` is at most the caller's `n` writable bytes (which
            // may be uninitialized), not overlapping the local.
            unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), out.cast::<u8>(), n) };
            out
        })
    }

    /// `git_oid_cpy`.
    ///
    /// # Safety
    ///
    /// `out` is writable and `src` valid (they may be the same).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_cpy(out: *mut Oid, src: *const Oid) -> c_int {
        // SAFETY: the caller's contract; `copy` allows overlap.
        unsafe { ptr::copy(src, out, 1) };
        0
    }

    /// `git_oid_cmp`.
    ///
    /// # Safety
    ///
    /// Both are valid oids.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_cmp(a: *const Oid, b: *const Oid) -> c_int {
        // SAFETY: a valid oid, only read.
        let a = unsafe { &*a };
        // SAFETY: as above.
        let b = unsafe { &*b };
        a.cmp_raw(b)
    }

    /// `git_oid_equal`.
    ///
    /// # Safety
    ///
    /// Both are valid oids.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_equal(a: *const Oid, b: *const Oid) -> c_int {
        // SAFETY: a valid oid, only read.
        let a = unsafe { &*a };
        // SAFETY: as above.
        let b = unsafe { &*b };
        (a == b) as c_int
    }

    /// `git_oid_ncmp`.
    ///
    /// # Safety
    ///
    /// Both are valid oids.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_ncmp(a: *const Oid, b: *const Oid, len: usize) -> c_int {
        // SAFETY: a valid oid, only read.
        let a = unsafe { &*a };
        // SAFETY: as above.
        let b = unsafe { &*b };
        a.ncmp(b, len)
    }

    /// `git_oid_strcmp`.
    ///
    /// # Safety
    ///
    /// `oid` is valid; `str` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_strcmp(oid: *const Oid, str: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: a valid oid.
            let oid = unsafe { &*oid };
            let p = str.cast::<u8>();
            // SAFETY: `strcmp_with` reads in order and stops at the NUL:
            // within the string.
            oid.strcmp_with(|i| unsafe { p.wrapping_add(i).read() })
        })
    }

    /// `git_oid_streq`: 0 if equal, -1 otherwise.
    ///
    /// # Safety
    ///
    /// As [`git_oid_strcmp`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_streq(oid: *const Oid, str: *const c_char) -> c_int {
        // SAFETY: forwarded.
        if unsafe { git_oid_strcmp(oid, str) } == 0 {
            0
        } else {
            -1
        }
    }

    /// `git_oid_is_zero`.
    ///
    /// # Safety
    ///
    /// `id` is a valid oid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_is_zero(id: *const Oid) -> c_int {
        // SAFETY: a valid oid, only read.
        unsafe { (*id).is_zero() as c_int }
    }

    /// `git_oid_shorten`: opaque to C.
    #[allow(non_camel_case_types)]
    pub type git_oid_shorten = OidShortener<'static>;

    /// `git_oid_shorten_new`. Freed with [`git_oid_shorten_free`].
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_shorten_new(min_length: usize) -> *mut git_oid_shorten {
        barrier(|| {
            // C's check, sign extension included: SIZE_MAX - 2^31 + 1 ..=
            // SIZE_MAX pass too, and are stored negative.
            let as_int = min_length as c_int;
            git_assert_arg!(
                as_int as isize as usize == min_length,
                "(size_t)((int)min_length) == min_length",
                ptr::null_mut()
            );
            let mut os = OidShortener::new(0);
            os.min_length = as_int;
            Box::into_raw(Box::new(os))
        })
    }

    /// `git_oid_shorten_free`.
    ///
    /// # Safety
    ///
    /// `os` is NULL or from [`git_oid_shorten_new`], not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_shorten_free(os: *mut git_oid_shorten) {
        barrier(|| {
            if !os.is_null() {
                // SAFETY: a `Box` from `git_oid_shorten_new`, released once.
                drop(unsafe { Box::from_raw(os) });
            }
        })
    }

    /// `git_oid_shorten_add`.
    ///
    /// # Safety
    ///
    /// `os` is from [`git_oid_shorten_new`], not used concurrently;
    /// `text_id` is NULL or a NUL-terminated string that stays alive and
    /// unchanged until `os` is freed (C's contract: "the OID is owned by the
    /// user and will not be modified or freed"; the shortener reads it
    /// later, as C's does).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oid_shorten_add(
        os: *mut git_oid_shorten,
        text_id: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live shortener, exclusively ours for the call.
            let os = unsafe { &mut *os };
            if os.full {
                return shorten_error("OID set full").raw();
            }
            if text_id.is_null() {
                return os.min_length;
            }
            // SAFETY: a NUL-terminated string that outlives the shortener,
            // unwritten (the caller's contract: the `'static` of a
            // `git_oid_shorten`); `add_raw` reads it as C does, never past
            // its NUL.
            let r = unsafe { os.add_raw(text_id.cast(), GIT_OID_SHA1_HEXSIZE) };
            match r {
                Ok(n) => n as c_int,
                Err(e) => e.raw(),
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["oid.h"],
    types: [
        Oid => "git_oid" { kind as "type", id },
    ],
    asserts: [
        "GIT_OID_SHA1 == 1",
        "GIT_OID_SHA256 == 2",
        "sizeof(git_oid_t) == 4",
        "GIT_OID_MAX_HEXSIZE == 64",
        "GIT_OID_MINPREFIXLEN == 4",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &[u8] = b"ae90f12eea699729ed24555e40b9fd669da12a12";

    #[test]
    fn parse_and_format() {
        let _g = crate::util::test_support::init();
        let id = Oid::from_hex(HEX, OidType::Sha1).unwrap();
        assert_eq!(id.raw_kind(), 1);
        assert_eq!(id.raw_id()[0], 0xae);
        assert_eq!(id.raw_id()[19], 0x12);
        assert!(id.raw_id()[20..].iter().all(|&b| b == 0));
        assert_eq!(id.hex().as_bytes(), HEX);
        assert_eq!(id.to_string().as_bytes(), HEX);
        assert_eq!(id.hex_prefix(8).as_str(), "ae90f12");
        assert_eq!(id.hex_prefix(100).as_bytes(), HEX);
        assert_eq!(id.hex_prefix(0).as_str(), "");
        assert_eq!(&id.hex_path().unwrap().as_bytes()[..4], b"ae/9");
        let up = Oid::from_hex(b"AE90F12EEA699729ED24555E40B9FD669DA12A12", OidType::Sha1).unwrap();
        assert_eq!(up, id);
        let mut sub = [0u8; 5];
        id.fmt_substr(&mut sub, 3);
        assert_eq!(&sub, b"0f12e");
    }

    #[test]
    fn parse_errors() {
        let _g = crate::util::test_support::init();
        let msg = || crate::util::errors::last_message().unwrap();
        assert!(Oid::from_hex(&HEX[..39], OidType::Sha1).is_err());
        assert_eq!(msg(), b"unable to parse OID - contains invalid characters");
        let mut long = HEX.to_vec();
        long.push(b'a');
        assert!(Oid::from_hex(&long, OidType::Sha1).is_err());
        assert_eq!(msg(), b"unable to parse OID - too long");
        // a NUL after the digits ends the C string
        long[40] = 0;
        assert!(Oid::from_hex(&long, OidType::Sha1).is_ok());
        assert!(Oid::from_prefix(b"", OidType::Sha1).is_err());
        assert_eq!(msg(), b"unable to parse OID - too short");
        assert!(Oid::from_prefix(&long, OidType::Sha1).is_err());
        assert_eq!(msg(), b"unable to parse OID - too long");
        assert!(Oid::from_prefix(b"abg", OidType::Sha1).is_err());
        assert_eq!(msg(), b"unable to parse OID - contains invalid characters");
        let p = Oid::from_prefix(b"abc", OidType::Sha256).unwrap();
        assert_eq!(p.raw_kind(), 2);
        assert_eq!(&p.raw_id()[..3], &[0xab, 0xc0, 0]);
    }

    #[test]
    fn partial_parse_is_c_s() {
        let mut id = [0u8; 32];
        assert!(parse_hex_into(&mut id, 6, |i| b"abcdxf"[i]).is_err());
        assert_eq!(&id[..3], &[0xab, 0xcd, 0]);
        let mut id = [0u8; 32];
        assert!(parse_hex_into(&mut id, 6, |i| b"abcdex"[i]).is_err());
        assert_eq!(&id[..3], &[0xab, 0xcd, 0xe0]);
    }

    #[test]
    fn compare() {
        let a = Oid::from_hex(HEX, OidType::Sha1).unwrap();
        let mut b = a;
        b.raw_id_mut()[5] += 3;
        assert_eq!(a.cmp_raw(&b), -3);
        assert_eq!(b.cmp_raw(&a), 3);
        assert!(a < b);
        assert_eq!(a.cmp_raw(&a), 0);
        assert_eq!(a.ncmp(&b, 10), 0);
        assert_eq!(a.ncmp(&b, 11), 0);
        assert_eq!(a.ncmp(&b, 12), 1);
        let c = Oid::zero(OidType::Sha256);
        assert_eq!(a.ncmp(&c, 1), -1);
        assert_eq!(a.cmp_raw(&c), -1);
        assert!(a.streq(HEX));
        assert!(a.streq(b"ae90"));
        assert!(a.streq(b"ae9"));
        assert!(!a.streq(b"ae91"));
        assert_eq!(a.strcmp(b"ae91"), 0x90 - 0x91);
        assert_eq!(a.strcmp(b"zz"), -1);
        assert!(Oid::SHA1_ZERO.is_zero());
        assert!(Oid::NONE.is_zero());
        assert!(!a.is_zero());
        assert_eq!(a.prefix(3).raw_id()[..3], [0xae, 0x90, 0]);
        assert_eq!(a.prefix(4).raw_id()[..3], [0xae, 0x90, 0]);
        assert_eq!(a.prefix(5).raw_id()[..3], [0xae, 0x90, 0xf0]);
        assert_eq!(a.hashcode(), u32::from_ne_bytes([0xae, 0x90, 0xf1, 0x2e]));
    }

    #[test]
    fn unknown_type() {
        let _g = crate::util::test_support::init();
        let o = Oid::from_raw_parts(7, [1; 32]);
        assert_eq!(o.kind(), None);
        assert_eq!(o.hexsize(), 0);
        assert_eq!(o.hex().as_str(), "");
        let mut buf = [b'x'; 4];
        assert!(o.nfmt(&mut buf).is_err());
        assert_eq!(&buf, b"xxxx");
        assert!(o.hex_path().is_err());
        assert!(!format!("{o:?}").is_empty());
    }

    #[test]
    fn nfmt_zero_fills() {
        let _g = crate::util::test_support::init();
        let a = Oid::from_hex(HEX, OidType::Sha1).unwrap();
        let mut buf = [b'x'; 45];
        a.nfmt(&mut buf).unwrap();
        assert_eq!(&buf[..40], HEX);
        assert!(buf[40..].iter().all(|&b| b == 0));
    }

    #[test]
    fn sha256() {
        let _g = crate::util::test_support::init();
        let hex = b"d3e63d2f2e43d1fee23a74bf19a0ede156cba2d1bd602eba13de433cea1bb512";
        let a = Oid::from_hex(hex, OidType::Sha256).unwrap();
        assert_eq!(a.hex().as_bytes(), hex);
        assert_eq!(a.hex_path().unwrap().as_bytes().len(), 65);
        assert_eq!(a.as_bytes().len(), 32);
        assert!(Oid::from_hex(&hex[..40], OidType::Sha256).is_err());
    }

    #[test]
    fn oid_types() {
        assert_eq!(OidType::from_name(b"sha1"), Some(OidType::Sha1));
        assert_eq!(OidType::from_name(b"sha256"), Some(OidType::Sha256));
        assert_eq!(OidType::from_name(b"sha2"), None);
        assert_eq!(OidType::from_raw(0), None);
        assert_eq!(oid_hexsize(2), 64);
        assert_eq!(oid_size(3), 0);
        assert!(is_hexstr(HEX, OidType::Sha1));
        assert!(!is_hexstr(&HEX[1..], OidType::Sha1));
        assert_eq!(Oid::from_bytes(&[1; 19], OidType::Sha1), None);
    }

    #[test]
    fn shortener() {
        let _g = crate::util::test_support::init();
        let mut s = OidShortener::new(0);
        assert_eq!(
            s.add(b"22596363b3de40b06f981fb85d82312e8c0ed511").unwrap(),
            1
        );
        assert_eq!(
            s.add(b"ce08fe4884650f067bd5703b6a59a8b3b3c99a09").unwrap(),
            1
        );
        assert_eq!(
            s.add(b"16a0123456789abcdef4b775213c23a8bd74f5e0").unwrap(),
            1
        );
        assert_eq!(
            s.add(b"22a0123456789abcdef4b775213c23a8bd74f5e0").unwrap(),
            3
        );
        assert_eq!(
            s.add(b"ce08fe4884650f067bd5703b6a59a8b3b3c99a00").unwrap(),
            40
        );
        assert!(s.add(b"zz").is_err());
        // a short id whose tail is later needed
        let mut s = OidShortener::new(4);
        assert_eq!(s.add(b"ab").unwrap(), 4);
        assert!(s.add(b"abcd").is_err());
    }

    /// U1-1: an add that fails right after reaching a leaf, then more adds
    /// through that node.
    #[test]
    fn shortener_fails_after_a_leaf() {
        let _g = crate::util::test_support::init();
        let mut s = OidShortener::new(0);
        assert_eq!(
            s.add(b"abcdef0123456789abcdef0123456789abcdef01").unwrap(),
            1
        );
        assert!(s.add(b"ax").is_err());
        // C: the node reads as empty for digits 4..f
        assert_eq!(
            s.add(b"a4ffffffffffffffffffffffffffffffffffffff").unwrap(),
            2
        );
        assert_eq!(
            s.add(b"abffffffffffffffffffffffffffffffffffffff").unwrap(),
            2
        );
        // digits 0..3 (C's undefined behaviour) read as empty too
        assert_eq!(
            s.add(b"a0ffffffffffffffffffffffffffffffffffffff").unwrap(),
            2
        );
        assert_eq!(s.add(b"a0fe").unwrap(), 4);
    }

    /// Random valid, invalid and short ids, interleaved, then valid ones
    /// again: every add returns (Miri checks the trie's memory).
    #[test]
    fn shortener_random_failures() {
        let _g = crate::util::test_support::init();
        let mut st = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = move || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            st
        };
        let n = if cfg!(miri) { 300 } else { 5000 };
        let ids: Vec<Vec<u8>> = (0..n)
            .map(|_| {
                // few distinct digits, so that walks go deep and meet leaves
                let len = match rnd() % 4 {
                    0 => (rnd() % 40) as usize,
                    _ => 40,
                };
                let mut v: Vec<u8> = (0..len).map(|_| b"012ab"[(rnd() % 5) as usize]).collect();
                if len > 0 && rnd() % 3 == 0 {
                    let at = (rnd() % len as u64) as usize;
                    v[at] = b"xg \0"[(rnd() % 4) as usize];
                }
                v
            })
            .collect();
        let mut s = OidShortener::new(0);
        let mut ok = 0;
        for id in ids.iter().chain(ids.iter()) {
            match s.add(id) {
                Ok(n) => {
                    assert!((1..=41).contains(&n));
                    ok += 1;
                }
                Err(e) => assert_eq!(e, ErrorCode::GENERIC),
            }
        }
        assert!(ok > 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "32000 insertions: too slow under Miri")]
    fn shortener_fills_up() {
        let _g = crate::util::test_support::init();
        let ids: Vec<[u8; 40]> = (0..40000u32)
            .map(|n| {
                let mut h = [0u8; 40];
                let x = n.wrapping_mul(2_654_435_761);
                for (i, d) in h.iter_mut().enumerate() {
                    *d = HEX_DIGITS[((x >> ((i % 8) * 4)) as usize ^ i) & 15];
                }
                h
            })
            .collect();
        let mut s = OidShortener::new(0);
        let mut n = 0u32;
        let r = loop {
            match s.add(&ids[n as usize]) {
                Ok(_) => n += 1,
                Err(e) => break e,
            }
            assert!(n < 40000);
        };
        assert_eq!(r, ErrorCode::GENERIC);
        assert!(s.full);
        assert!(s.add(HEX).is_err());
    }
}
