//! SHA-256 (FIPS 180-4), the Rust replacement of libgit2's OpenSSL SHA-256
//! backend (`src/util/hash/openssl.c`, `GIT_SHA256_OPENSSL`).
//!
//! The portable implementation is plain safe Rust. On x86-64 CPUs with AVX2
//! and BMI1/2 (detected at run time) the blocks go through a faster path
//! that computes the message schedule of two blocks at once with AVX2,
//! interleaved with the scalar rounds of the first, as OpenSSL's
//! `sha256_block_data_order_avx2` does; the rounds use BMI's `rorx`/`andn`.
//! The `sha2` crate's portable code ran at half OpenSSL's speed on such
//! CPUs (see `rust/bench/RESULTS.md`), hence this implementation.

/// The size of a SHA-256 digest (`GIT_HASH_SHA256_SIZE`).
pub const SIZE: usize = 32;

const BLOCK: usize = 64;

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A SHA-256 computation (`git_hash_sha256_ctx`).
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; BLOCK],
    /// Bytes hashed so far (mod 2^64); `total % 64` of them are in `buf`.
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl core::fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Sha256 { .. }")
    }
}

impl Sha256 {
    /// A new computation (`SHA256_Init`).
    pub const fn new() -> Sha256 {
        Sha256 {
            state: H0,
            buf: [0; BLOCK],
            total: 0,
        }
    }

    /// Hash `data` (`SHA256_Update`).
    pub fn update(&mut self, mut data: &[u8]) {
        let fill = (self.total % BLOCK as u64) as usize;
        self.total = self.total.wrapping_add(data.len() as u64);
        if fill > 0 {
            let take = data.len().min(BLOCK - fill);
            self.buf[fill..fill + take].copy_from_slice(&data[..take]);
            data = &data[take..];
            if fill + take < BLOCK {
                return;
            }
            let block = self.buf;
            compress(&mut self.state, core::slice::from_ref(&block));
        }
        let (blocks, rest) = data.as_chunks::<BLOCK>();
        compress(&mut self.state, blocks);
        self.buf[..rest.len()].copy_from_slice(rest);
    }

    /// Write the digest of what was hashed to `out` (`SHA256_Final`). The
    /// computation must be restarted (`*self = Sha256::new()`) before reuse.
    pub fn finalize_into(&mut self, out: &mut [u8; SIZE]) {
        let fill = (self.total % BLOCK as u64) as usize;
        let bits = self.total.wrapping_mul(8);
        let mut tail = [[0u8; BLOCK]; 2];
        let n = if fill < 56 { 1 } else { 2 };
        tail[0][..fill].copy_from_slice(&self.buf[..fill]);
        tail[0][fill] = 0x80;
        tail[n - 1][56..].copy_from_slice(&bits.to_be_bytes());
        compress(&mut self.state, &tail[..n]);
        for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            o.copy_from_slice(&s.to_be_bytes());
        }
    }

    /// The digest of `data`.
    pub fn digest(data: &[u8]) -> [u8; SIZE] {
        let mut h = Sha256::new();
        h.update(data);
        let mut out = [0; SIZE];
        h.finalize_into(&mut out);
        out
    }
}

/// Process whole blocks.
fn compress(state: &mut [u32; 8], blocks: &[[u8; BLOCK]]) {
    if blocks.is_empty() {
        return;
    }
    #[cfg(all(target_arch = "x86_64", not(miri)))]
    if std::is_x86_feature_detected!("avx2")
        && std::is_x86_feature_detected!("bmi1")
        && std::is_x86_feature_detected!("bmi2")
    {
        // SAFETY: the CPU has the features `avx2::compress` is compiled for.
        unsafe { avx2::compress(state, blocks) };
        return;
    }
    compress_soft(state, blocks);
}

/// One SHA-256 round on the working variables named in rotated order.
macro_rules! round {
    ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $wk:expr) => {
        let hw = $h.wrapping_add($wk);
        // Ch(e, f, g): the two terms have no bit in common, so `+` is `^`.
        let ch = (!$e & $g).wrapping_add($e & $f);
        let s1 = $e.rotate_right(6) ^ $e.rotate_right(11) ^ $e.rotate_right(25);
        let t1 = hw.wrapping_add(ch).wrapping_add(s1);
        let s0 = $a.rotate_right(2) ^ $a.rotate_right(13) ^ $a.rotate_right(22);
        // Maj(a, b, c); `a ^ b` is the next round's `b ^ c`.
        let maj = (($a ^ $b) & ($b ^ $c)) ^ $b;
        $d = $d.wrapping_add(hw).wrapping_add(ch).wrapping_add(s1);
        $h = t1.wrapping_add(s0.wrapping_add(maj));
    };
}

/// Eight rounds on the state `$s`, reading `W[t] + K[t]` from `$wk(t)`
/// for `t` in `$t..$t + 8` (after eight rounds the variables are back in
/// their places).
macro_rules! rounds8 {
    ($s:ident, $wk:ident, $t:expr) => {
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = $s;
        round!(a, b, c, d, e, f, g, h, $wk!($t));
        round!(h, a, b, c, d, e, f, g, $wk!($t + 1));
        round!(g, h, a, b, c, d, e, f, $wk!($t + 2));
        round!(f, g, h, a, b, c, d, e, $wk!($t + 3));
        round!(e, f, g, h, a, b, c, d, $wk!($t + 4));
        round!(d, e, f, g, h, a, b, c, $wk!($t + 5));
        round!(c, d, e, f, g, h, a, b, $wk!($t + 6));
        round!(b, c, d, e, f, g, h, a, $wk!($t + 7));
        $s = [a, b, c, d, e, f, g, h];
    };
}

/// The portable implementation.
fn compress_soft(state: &mut [u32; 8], blocks: &[[u8; BLOCK]]) {
    for block in blocks {
        let mut w = [0u32; 64];
        for (w, b) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        for t in 16..64 {
            let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
            let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
            w[t] = w[t - 16]
                .wrapping_add(s0)
                .wrapping_add(w[t - 7])
                .wrapping_add(s1);
        }
        macro_rules! wk {
            ($t:expr) => {
                w[$t].wrapping_add(K[$t])
            };
        }
        let mut s = *state;
        let mut t = 0;
        while t < 64 {
            rounds8!(s, wk, t);
            t += 8;
        }
        for (st, v) in state.iter_mut().zip(s) {
            *st = st.wrapping_add(v);
        }
    }
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
mod avx2 {
    use super::{BLOCK, K};
    use core::arch::x86_64::*;

    /// `K`, each group of four repeated for the two 128-bit lanes.
    static KK: [u32; 128] = {
        let mut k = [0u32; 128];
        let mut i = 0;
        while i < 64 {
            k[(i / 4) * 8 + i % 4] = K[i];
            k[(i / 4) * 8 + 4 + i % 4] = K[i];
            i += 1;
        }
        k
    };

    macro_rules! ror {
        ($x:expr, $n:literal) => {
            _mm256_or_si256(
                _mm256_srli_epi32::<$n>($x),
                _mm256_slli_epi32::<{ 32 - $n }>($x),
            )
        };
    }

    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn sigma0(x: __m256i) -> __m256i {
        _mm256_xor_si256(
            _mm256_xor_si256(ror!(x, 7), ror!(x, 18)),
            _mm256_srli_epi32::<3>(x),
        )
    }

    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn sigma1(x: __m256i) -> __m256i {
        _mm256_xor_si256(
            _mm256_xor_si256(ror!(x, 17), ror!(x, 19)),
            _mm256_srli_epi32::<10>(x),
        )
    }

    /// `W[t..t + 4]` of both blocks from `W[t - 16..t]` (`x0` to `x3`).
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn schedule(x0: __m256i, x1: __m256i, x2: __m256i, x3: __m256i) -> __m256i {
        let w15 = _mm256_alignr_epi8::<4>(x1, x0);
        let w7 = _mm256_alignr_epi8::<4>(x3, x2);
        let t = _mm256_add_epi32(_mm256_add_epi32(x0, sigma0(w15)), w7);
        // σ1 of W[t - 2], W[t - 1] completes W[t], W[t + 1] ...
        let v = _mm256_shuffle_epi32::<0b11_10_11_10>(x3);
        let t = _mm256_add_epi32(
            t,
            _mm256_blend_epi32::<0b0011_0011>(_mm256_setzero_si256(), sigma1(v)),
        );
        // ... whose σ1 completes W[t + 2], W[t + 3].
        let v = _mm256_shuffle_epi32::<0b01_00_01_00>(t);
        _mm256_add_epi32(
            t,
            _mm256_blend_epi32::<0b1100_1100>(_mm256_setzero_si256(), sigma1(v)),
        )
    }

    /// Bytes `16 * i..16 * i + 16` of `b`.
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn load(b: &[u8; BLOCK], i: usize) -> __m128i {
        let s = &b[16 * i..16 * i + 16];
        #[allow(clippy::cast_ptr_alignment)] // `loadu` needs no alignment
        let p = s.as_ptr().cast::<__m128i>();
        // SAFETY: `p` points to the 16 bytes of `s`, which the unaligned
        // load reads.
        unsafe { _mm_loadu_si128(p) }
    }

    /// `dst = v`.
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn store(dst: &mut [u32], v: __m256i) {
        let dst = &mut dst[..8];
        #[allow(clippy::cast_ptr_alignment)] // `storeu` needs no alignment
        let p = dst.as_mut_ptr().cast::<__m256i>();
        // SAFETY: `p` points to the 32 bytes of `dst`, which the unaligned
        // store writes.
        unsafe { _mm256_storeu_si256(p, v) }
    }

    /// `KK[8 * i..8 * i + 8]`.
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    fn kk(i: usize) -> __m256i {
        let k = &KK[8 * i..8 * i + 8];
        #[allow(clippy::cast_ptr_alignment)] // `loadu` needs no alignment
        let p = k.as_ptr().cast::<__m256i>();
        // SAFETY: `p` points to the 32 bytes of `k`, which the unaligned
        // load reads.
        unsafe { _mm256_loadu_si256(p) }
    }

    /// Process `b1` and, if `two`, then `b2`.
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    #[inline]
    fn compress2(state: &mut [u32; 8], b1: &[u8; BLOCK], b2: &[u8; BLOCK], two: bool) {
        let bswap = _mm256_set_epi8(
            12, 13, 14, 15, 8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3, 12, 13, 14, 15, 8, 9, 10, 11, 4,
            5, 6, 7, 0, 1, 2, 3,
        );
        // W[t] + K[t] of block 1 at `wk[8 * (t / 4) + t % 4]`, of block 2
        // four further.
        let mut wk = [0u32; 128];
        let mut x = [_mm256_setzero_si256(); 4];
        for (i, x) in x.iter_mut().enumerate() {
            *x = _mm256_shuffle_epi8(_mm256_set_m128i(load(b2, i), load(b1, i)), bswap);
            store(&mut wk[8 * i..], _mm256_add_epi32(*x, kk(i)));
        }
        // Keep `wk` in memory: the rounds read it from there, which is
        // cheaper than extracting lanes from the vectors.
        core::hint::black_box(&mut wk);
        let [mut x0, mut x1, mut x2, mut x3] = x;
        macro_rules! sched {
            ($i:expr) => {{
                let n = schedule(x0, x1, x2, x3);
                (x0, x1, x2, x3) = (x1, x2, x3, n);
                store(&mut wk[8 * $i..], _mm256_add_epi32(n, kk($i)));
                core::hint::black_box(&mut wk);
            }};
        }
        // Block 1's `W[t] + K[t]`, then block 2's.
        macro_rules! w1 {
            ($t:expr) => {
                wk[(($t) >> 2) * 8 + (($t) & 3)]
            };
        }
        macro_rules! w2 {
            ($t:expr) => {
                wk[(($t) >> 2) * 8 + 4 + (($t) & 3)]
            };
        }
        let mut s = *state;
        // Block 1, computing the schedule four groups ahead of the rounds.
        let mut g = 0;
        while g < 12 {
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut gg, mut h] = s;
            let t = 4 * g;
            round!(a, b, c, d, e, f, gg, h, w1!(t));
            round!(h, a, b, c, d, e, f, gg, w1!(t + 1));
            sched!(g + 4);
            round!(gg, h, a, b, c, d, e, f, w1!(t + 2));
            round!(f, gg, h, a, b, c, d, e, w1!(t + 3));
            round!(e, f, gg, h, a, b, c, d, w1!(t + 4));
            round!(d, e, f, gg, h, a, b, c, w1!(t + 5));
            sched!(g + 5);
            round!(c, d, e, f, gg, h, a, b, w1!(t + 6));
            round!(b, c, d, e, f, gg, h, a, w1!(t + 7));
            s = [a, b, c, d, e, f, gg, h];
            g += 2;
        }
        rounds8!(s, w1, 48);
        rounds8!(s, w1, 56);
        for (st, v) in state.iter_mut().zip(s) {
            *st = st.wrapping_add(v);
        }
        if two {
            let mut s = *state;
            let mut t = 0;
            while t < 64 {
                rounds8!(s, w2, t);
                t += 8;
            }
            for (st, v) in state.iter_mut().zip(s) {
                *st = st.wrapping_add(v);
            }
        }
    }

    /// Process `blocks`, two at a time.
    #[target_feature(enable = "avx2,bmi1,bmi2")]
    pub(super) fn compress(state: &mut [u32; 8], blocks: &[[u8; BLOCK]]) {
        let (pairs, rest) = blocks.as_chunks::<2>();
        for [b1, b2] in pairs {
            compress2(state, b1, b2, true);
        }
        if let [b] = rest {
            compress2(state, b, b, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest as _;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn known_answers() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&Sha256::digest(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    fn data(n: usize) -> Vec<u8> {
        let mut x: u32 = 0x9e37_79b9;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn matches_the_reference_at_every_length_and_split() {
        let d = data(if cfg!(miri) { 300 } else { 1100 });
        for n in 0..d.len() {
            let want = sha2::Sha256::digest(&d[..n]);
            assert_eq!(Sha256::digest(&d[..n])[..], want[..], "length {n}");
            if n % 7 == 0 {
                // Fed in uneven pieces.
                let mut h = Sha256::new();
                let mut rest = &d[..n];
                let mut step = 1;
                while !rest.is_empty() {
                    let k = step.min(rest.len());
                    h.update(&rest[..k]);
                    rest = &rest[k..];
                    step = step * 3 % 97 + 1;
                }
                let mut out = [0; SIZE];
                h.finalize_into(&mut out);
                assert_eq!(out[..], want[..], "length {n} in pieces");
            }
        }
    }

    #[test]
    fn soft_and_dispatched_compress_agree() {
        let d = data(64 * 9);
        let (blocks, _) = d.as_chunks::<BLOCK>();
        for n in 0..=blocks.len() {
            let mut a = H0;
            let mut b = H0;
            compress_soft(&mut a, &blocks[..n]);
            compress(&mut b, &blocks[..n]);
            assert_eq!(a, b, "{n} blocks");
        }
    }
}
