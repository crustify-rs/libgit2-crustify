//! Port of `src/util/varint.c`: git's offset varints (the index v4 and pack
//! encodings: big-endian base-128 with an offset of one per continuation
//! byte).
//!
//! ```
//! use libgit2_rs::__test::util::varint;
//! let mut buf = [0u8; 16];
//! let n = varint::encode(Some(&mut buf), 16384);
//! assert_eq!(&buf[..n as usize], &[0xff, 0x00]);
//! assert_eq!(varint::decode(&buf[..2]), Some((16384, 2)));
//! ```

use core::ffi::c_int;

/// `MSB(val, 7)`: the top 7 bits of a 64-bit value.
const TOP7: u64 = !0u64 << (64 - 7);

/// `git_decode_varint`, reading bytes with `byte_at(i)` in order: the value
/// and its length, `Some((0, 0))` on overflow (C's error), or `None` when
/// `byte_at` runs out.
#[inline(always)]
fn decode_with(mut byte_at: impl FnMut(usize) -> Option<u8>) -> Option<(u64, usize)> {
    let mut i = 0;
    let mut c = byte_at(i)?;
    i += 1;
    let mut val = (c & 127) as u64;
    while c & 128 != 0 {
        val = val.wrapping_add(1);
        if val == 0 || val & TOP7 != 0 {
            // overflow
            return Some((0, 0));
        }
        c = byte_at(i)?;
        i += 1;
        val = (val << 7) + (c & 127) as u64;
    }
    Some((val, i))
}

/// `git_decode_varint`: the value at the start of `buf` and its length;
/// `(0, 0)` on overflow (C's error). `None` if `buf` ends inside the
/// varint (C reads on past it).
pub fn decode(buf: &[u8]) -> Option<(u64, usize)> {
    decode_with(|i| buf.get(i).copied())
}

/// `git_encode_varint`: write `value` to the start of `buf` (if given);
/// its length, or -1 if `buf` is too small.
pub fn encode(buf: Option<&mut [u8]>, mut value: u64) -> c_int {
    let mut varint = [0u8; 16];
    let mut pos = varint.len() - 1;
    varint[pos] = (value & 127) as u8;
    loop {
        value >>= 7;
        if value == 0 {
            break;
        }
        value -= 1;
        pos -= 1;
        varint[pos] = 128 | (value & 127) as u8;
    }
    let n = varint.len() - pos;
    if let Some(buf) = buf {
        if buf.len() < n {
            return -1;
        }
        buf[..n].copy_from_slice(&varint[pos..]);
    }
    n as c_int
}

/// The exports of `varint.c`.
pub mod ffi {
    use core::ffi::{c_int, c_uchar};

    use super::*;

    /// `git_decode_varint`.
    ///
    /// # Safety
    ///
    /// `bufp` is readable up to the end of the varint it starts (C reads
    /// until a byte without the continuation bit, or until the value
    /// overflows); `varint_len` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_decode_varint(
        bufp: *const c_uchar,
        varint_len: *mut usize,
    ) -> u64 {
        // SAFETY: `decode_with` reads in order and stops where C does: the
        // caller guarantees those bytes.
        let r = decode_with(|i| Some(unsafe { bufp.wrapping_add(i).read() }));
        let (val, len) = r.unwrap_or((0, 0));
        // SAFETY: writable.
        unsafe { varint_len.write(len) };
        val
    }

    /// `git_encode_varint`.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or writable for `bufsize` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_encode_varint(
        buf: *mut c_uchar,
        bufsize: usize,
        value: u64,
    ) -> c_int {
        let mut local = [0u8; 16];
        let n = encode(Some(&mut local), value);
        if !buf.is_null() {
            if bufsize < n as usize {
                return -1;
            }
            // SAFETY: `n <= bufsize` writable bytes (possibly uninitialized,
            // so written through the pointer), not overlapping the local.
            unsafe { core::ptr::copy_nonoverlapping(local.as_ptr(), buf, n as usize) };
        }
        n
    }
}

#[cfg(test)]
#[allow(clippy::borrow_as_ptr)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in [
            0u64,
            1,
            127,
            128,
            255,
            16383,
            16384,
            1 << 32,
            u64::MAX / 3,
            u64::MAX,
        ] {
            let mut buf = [0u8; 16];
            let n = encode(Some(&mut buf), v);
            assert!(n > 0);
            assert_eq!(encode(None, v), n);
            assert_eq!(decode(&buf), Some((v, n as usize)), "{v}");
            assert_eq!(decode(&buf[..n as usize - 1]), None);
            let mut small = [0u8; 1];
            if n > 1 {
                assert_eq!(encode(Some(&mut small), v), -1);
            }
        }
        assert_eq!(encode(None, 127), 1);
        assert_eq!(encode(None, 128), 2);
        assert_eq!(encode(None, u64::MAX), 10);
    }

    #[test]
    fn overflow() {
        assert_eq!(decode(&[0xff; 12]), Some((0, 0)));
        let mut len = 9;
        // SAFETY: the varint overflows within the array.
        let v = unsafe { ffi::git_decode_varint([0xffu8; 12].as_ptr(), &mut len) };
        assert_eq!((v, len), (0, 0));
    }
}
