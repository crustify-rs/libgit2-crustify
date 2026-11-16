//! `src/util/hash.c` and the backends of this configuration: SHA-1 with
//! collision detection (`hash/collisiondetect.c` over `hash/sha1dc`, here
//! the `sha1collisiondetection` crate, a Rust port of sha1dc) and SHA-256
//! (`hash/openssl.c`, here [`sha256`]).
//!
//! # Types
//!
//! - [`Hasher`] is the Rust-native computation; [`HashAlgorithm`] picks it.
//! - [`HashCtx`] is `git_hash_ctx`, which C that stays embeds by value
//!   (indexer, pack-objects, filebuf, diff, midx, commit-graph, …). It is
//!   mirrored as an opaque blob: `hash.h` declares it, under `GIT_RUST`, as
//!   `uint64_t opaque[GIT_HASH_CTX_WORDS]` (size and alignment checked by
//!   the layout test), and only this module reads it: an algorithm tag
//!   followed by the [`Hasher`] in place.
//!
//! # Semantics kept from the C
//!
//! The SHA-1 is sha1dc's with its defaults (safe hash on, unavoidable
//! bit conditions on, collision detection on): on a detected collision,
//! `git_hash_final` writes sha1dc's "safe" digest and fails with
//! `GIT_ERROR_SHA` "SHA1 collision attack detected". A finished
//! computation is not reset: `git_hash_init` restarts it.

use core::cell::RefCell;
use core::ffi::{c_char, c_int, c_void};
use core::mem::MaybeUninit;

use sha1collisiondetection::{Output as Sha1Output, Sha1CD};

use super::errors::{ErrorClass, ErrorCode, Result};
use crate::git_error_set;

pub mod sha256;

pub use sha256::Sha256;

/// `GIT_HASH_SHA1_SIZE`.
pub const SHA1_SIZE: usize = 20;
/// `GIT_HASH_SHA256_SIZE`.
pub const SHA256_SIZE: usize = sha256::SIZE;
/// `GIT_HASH_MAX_SIZE`.
pub const MAX_SIZE: usize = SHA256_SIZE;

/// `git_hash_algorithm_t`'s values (`GIT_HASH_ALGORITHM_*`).
pub mod raw_algorithm {
    use core::ffi::c_int;
    /// `GIT_HASH_ALGORITHM_NONE`
    pub const NONE: c_int = 0;
    /// `GIT_HASH_ALGORITHM_SHA1`
    pub const SHA1: c_int = 1;
    /// `GIT_HASH_ALGORITHM_SHA256`
    pub const SHA256: c_int = 2;
}

/// A hash algorithm (`git_hash_algorithm_t` without `NONE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    /// SHA-1 with collision detection.
    Sha1,
    /// SHA-256.
    Sha256,
}

impl HashAlgorithm {
    /// From C's `git_hash_algorithm_t`; `None` for `NONE` and unknown values.
    pub const fn from_raw(raw: c_int) -> Option<HashAlgorithm> {
        match raw {
            raw_algorithm::SHA1 => Some(HashAlgorithm::Sha1),
            raw_algorithm::SHA256 => Some(HashAlgorithm::Sha256),
            _ => None,
        }
    }

    /// C's `git_hash_algorithm_t` value.
    pub const fn raw(self) -> c_int {
        match self {
            HashAlgorithm::Sha1 => raw_algorithm::SHA1,
            HashAlgorithm::Sha256 => raw_algorithm::SHA256,
        }
    }

    /// The digest size (`git_hash_size`).
    pub const fn size(self) -> usize {
        match self {
            HashAlgorithm::Sha1 => SHA1_SIZE,
            HashAlgorithm::Sha256 => SHA256_SIZE,
        }
    }
}

/// `git_hash_size`: the digest size of a C algorithm value, 0 if unknown.
pub const fn size(raw: c_int) -> usize {
    match HashAlgorithm::from_raw(raw) {
        Some(a) => a.size(),
        None => 0,
    }
}

/// A digest: up to [`MAX_SIZE`] bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest {
    bytes: [u8; MAX_SIZE],
    len: u8,
}

impl Digest {
    /// The digest's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// The digest in lowercase hexadecimal (`git_hash_fmt`).
    pub fn to_hex(&self) -> String {
        let mut s = vec![0u8; 2 * self.len as usize];
        fmt_into(&mut s, self.as_bytes());
        // Only hexadecimal digits were written.
        String::from_utf8(s).unwrap_or_default()
    }
}

impl core::fmt::Debug for Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Digest({})", self.to_hex())
    }
}

/// A hash computation (`git_hash_ctx`'s Rust-native state).
#[derive(Clone, Debug)]
// sha1dc's state is large; it lives in place (in C's `git_hash_ctx`, on the
// stack), never boxed.
#[allow(clippy::large_enum_variant)]
pub enum Hasher {
    /// SHA-1 with collision detection (sha1dc).
    Sha1(Sha1CD),
    /// SHA-256.
    Sha256(Sha256),
}

// `git_hash_ctx_cleanup` frees nothing; `HashCtx` relies on it.
const _: () = assert!(!core::mem::needs_drop::<Hasher>());

impl Hasher {
    /// A new computation (`git_hash_ctx_init`).
    pub fn new(algorithm: HashAlgorithm) -> Hasher {
        match algorithm {
            HashAlgorithm::Sha1 => Hasher::Sha1(Sha1CD::default()),
            HashAlgorithm::Sha256 => Hasher::Sha256(Sha256::new()),
        }
    }

    /// The algorithm.
    pub fn algorithm(&self) -> HashAlgorithm {
        match self {
            Hasher::Sha1(_) => HashAlgorithm::Sha1,
            Hasher::Sha256(_) => HashAlgorithm::Sha256,
        }
    }

    /// Restart the computation (`git_hash_init`).
    pub fn reset(&mut self) {
        match self {
            Hasher::Sha1(h) => h.reset(),
            Hasher::Sha256(h) => *h = Sha256::new(),
        }
    }

    /// Hash `data` (`git_hash_update`).
    #[inline]
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
        }
    }

    /// Write the digest to `out` (its first [`HashAlgorithm::size`] bytes)
    /// and return whether sha1dc detected a collision (then `out` holds the
    /// "safe" digest), as `SHA1DCFinal`. The computation must be
    /// [`reset`](Self::reset) before reuse.
    pub fn finalize_raw(&mut self, out: &mut [u8; MAX_SIZE]) -> bool {
        match self {
            Hasher::Sha1(h) => {
                let mut o = Sha1Output::default();
                let collision = h.finalize_into_dirty_cd(&mut o).is_err();
                out[..SHA1_SIZE].copy_from_slice(&o);
                collision
            }
            Hasher::Sha256(h) => {
                let mut o = [0; SHA256_SIZE];
                h.finalize_into(&mut o);
                out[..SHA256_SIZE].copy_from_slice(&o);
                false
            }
        }
    }

    /// The digest (`git_hash_final`): fails with `GIT_ERROR_SHA` on a
    /// detected SHA-1 collision.
    pub fn finalize(&mut self) -> Result<Digest> {
        let mut bytes = [0; MAX_SIZE];
        if self.finalize_raw(&mut bytes) {
            return Err(collision_error());
        }
        Ok(Digest {
            bytes,
            len: self.algorithm().size() as u8,
        })
    }
}

fn collision_error() -> ErrorCode {
    git_error_set!(ErrorClass::SHA, "SHA1 collision attack detected")
}

fn unknown_algorithm() -> ErrorCode {
    git_error_set!(ErrorClass::INTERNAL, "unknown hash algorithm")
}

/// The algorithm of a C value, or the C's "unknown hash algorithm" error.
pub fn algorithm(raw: c_int) -> Result<HashAlgorithm> {
    HashAlgorithm::from_raw(raw).ok_or_else(unknown_algorithm)
}

std::thread_local! {
    /// A computation per algorithm, which the one-shot functions restart
    /// instead of building (and zeroing) sha1dc's 2.4 KiB of state each
    /// time.
    static ONESHOT: RefCell<[Option<Hasher>; 2]> = const { RefCell::new([None, None]) };
}

/// Run `f` on a new computation of `algorithm`.
fn with_oneshot<R>(algorithm: HashAlgorithm, f: impl FnOnce(&mut Hasher) -> R) -> R {
    let mut f = Some(f);
    let slot = match algorithm {
        HashAlgorithm::Sha1 => 0,
        HashAlgorithm::Sha256 => 1,
    };
    let reused = ONESHOT.try_with(|cell| {
        // Not re-entered (nothing here calls out), but be safe if it were.
        let mut hashers = cell.try_borrow_mut().ok()?;
        let h = match &mut hashers[slot] {
            Some(h) => {
                h.reset();
                h
            }
            empty => empty.insert(Hasher::new(algorithm)),
        };
        f.take().map(|f| f(h))
    });
    match (reused, f) {
        (Ok(Some(r)), _) => r,
        // The thread's storage is gone or busy: a computation of our own.
        (_, Some(f)) => f(&mut Hasher::new(algorithm)),
        (_, None) => unreachable!("`f` ran but returned nothing"),
    }
}

/// `git_hash_buf`.
pub fn hash_buf(data: &[u8], algorithm: HashAlgorithm) -> Result<Digest> {
    with_oneshot(algorithm, |h| {
        h.update(data);
        h.finalize()
    })
}

/// `git_hash_vec`.
pub fn hash_vec(data: &[&[u8]], algorithm: HashAlgorithm) -> Result<Digest> {
    with_oneshot(algorithm, |h| {
        for d in data {
            h.update(d);
        }
        h.finalize()
    })
}

/// `git_hash_fmt` without the NUL: write `hash` in lowercase hexadecimal
/// to the first `2 * hash.len()` bytes of `out`.
///
/// # Panics
///
/// If `out` is shorter than that.
pub fn fmt_into(out: &mut [u8], hash: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (o, b) in out[..2 * hash.len()]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(hash)
    {
        o[0] = HEX[(b >> 4) as usize];
        o[1] = HEX[(b & 0x0f) as usize];
    }
}

// ---------------------------------------------------------------------------
// `git_hash_ctx`
// ---------------------------------------------------------------------------

/// `git_hash_ctx`: the algorithm C gave `git_hash_ctx_init`, then the
/// computation, in place. C sees it as an opaque blob of the same size and
/// alignment (`hash.h`), and embeds it by value.
///
/// `hasher` is initialized exactly when `algorithm` is a known algorithm
/// and the context went through [`HashCtx::init`] (`git_hash_ctx_init`);
/// that is the contract of the `git_hash_*` exports, as it was C's.
#[repr(C)]
pub struct HashCtx {
    algorithm: c_int,
    hasher: MaybeUninit<Hasher>,
}

/// `GIT_HASH_CTX_WORDS` in `hash.h`: the blob's size in `uint64_t`s.
pub const HASH_CTX_WORDS: usize = core::mem::size_of::<HashCtx>() / 8;
const _: () = assert!(core::mem::size_of::<HashCtx>().is_multiple_of(8));
const _: () = assert!(core::mem::align_of::<HashCtx>() == 8);
// `hash.h` spells the size out; the layout test checks it against the C.
const _: () = assert!(HASH_CTX_WORDS == 300, "update GIT_HASH_CTX_WORDS in hash.h");

impl HashCtx {
    /// A zeroed context (C's `{0}`, as in `GIT_FILEBUF_INIT`): no
    /// algorithm, no computation. (For `git_filebuf`, WP u3.)
    pub const fn zeroed() -> HashCtx {
        HashCtx {
            algorithm: 0,
            hasher: MaybeUninit::zeroed(),
        }
    }

    /// A context for `algorithm`, as `git_hash_ctx_init` leaves it.
    pub fn new(algorithm: HashAlgorithm) -> HashCtx {
        HashCtx {
            algorithm: algorithm.raw(),
            hasher: MaybeUninit::new(Hasher::new(algorithm)),
        }
    }

    /// `git_hash_ctx_init`: record `raw` and start a computation, or fail
    /// with "unknown hash algorithm" (the context then has no computation;
    /// `raw` is still recorded, as C does).
    pub fn init(&mut self, raw: c_int) -> Result<()> {
        self.algorithm = raw;
        let a = algorithm(raw)?;
        self.hasher.write(Hasher::new(a));
        Ok(())
    }

    /// `git_hash_init`: restart the computation of the recorded algorithm.
    pub fn restart(&mut self) -> Result<()> {
        let a = algorithm(self.algorithm)?;
        self.hasher.write(Hasher::new(a));
        Ok(())
    }

    /// The computation, or the "unknown hash algorithm" error.
    ///
    /// # Safety
    ///
    /// The context went through [`init`](Self::init) (or [`new`](Self::new))
    /// with the algorithm it records.
    pub unsafe fn hasher(&mut self) -> Result<&mut Hasher> {
        if HashAlgorithm::from_raw(self.algorithm).is_none() {
            return Err(unknown_algorithm());
        }
        // SAFETY: a known algorithm is recorded, so (the caller vouches)
        // `init` wrote the computation.
        Ok(unsafe { self.hasher.assume_init_mut() })
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// `git_str_vec`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitStrVec {
    /// `data`
    pub data: *mut c_void,
    /// `len`
    pub len: usize,
}

pub mod ffi {
    use super::*;
    use crate::git_assert_arg;
    use crate::util::export::{barrier, bytes, status};

    /// `git_hash_global_init`: nothing to set up.
    ///
    /// # Safety
    ///
    /// None (an `extern "C"` init function of libgit2's runtime).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_global_init() -> c_int {
        0
    }

    /// Convert a context pointer.
    ///
    /// # Safety
    ///
    /// A non-null `ctx` points to a `git_hash_ctx` (initialized or not)
    /// that nothing else accesses during the call.
    unsafe fn ctx<'a>(ctx: *mut HashCtx) -> Result<&'a mut HashCtx> {
        git_assert_arg!(!ctx.is_null(), "ctx");
        // SAFETY: the caller's contract; `HashCtx` is valid for any bytes
        // (its computation is `MaybeUninit`).
        Ok(unsafe { &mut *ctx })
    }

    /// `git_hash_ctx_init`.
    ///
    /// # Safety
    ///
    /// `ctx` points to a `git_hash_ctx` (its contents are overwritten) that
    /// nothing else accesses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_ctx_init(ctx_: *mut HashCtx, algorithm: c_int) -> c_int {
        barrier(|| {
            // SAFETY: forwarded to the caller.
            status(unsafe { ctx(ctx_) }.and_then(|c| c.init(algorithm)))
        })
    }

    /// `git_hash_ctx_cleanup`: the computation holds no resources (as
    /// neither backend did), so this does nothing.
    ///
    /// # Safety
    ///
    /// None beyond C's: `ctx` is not read.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_ctx_cleanup(ctx: *mut HashCtx) {
        let _ = ctx;
    }

    /// `git_hash_init`: restart the computation.
    ///
    /// # Safety
    ///
    /// `ctx` went through `git_hash_ctx_init`, and nothing else accesses it
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_init(ctx_: *mut HashCtx) -> c_int {
        barrier(|| {
            // SAFETY: forwarded to the caller.
            status(unsafe { ctx(ctx_) }.and_then(|c| c.restart()))
        })
    }

    /// `git_hash_update`.
    ///
    /// # Safety
    ///
    /// As `git_hash_init`; `data` is readable for `len` bytes (or `len` is
    /// 0) and not written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_update(
        ctx_: *mut HashCtx,
        data: *const c_void,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: forwarded to the caller.
            let r = unsafe { ctx(ctx_) }.and_then(|c| {
                // SAFETY: the caller initialized the context.
                let h = unsafe { c.hasher() }?;
                // SAFETY: the caller's contract on `data`.
                h.update(unsafe { bytes(data.cast::<c_char>(), len) });
                Ok(())
            });
            status(r)
        })
    }

    /// `git_hash_final`: write the digest (its algorithm's size) to `out`.
    ///
    /// # Safety
    ///
    /// As `git_hash_init`; `out` is writable for the digest's size.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_final(out: *mut u8, ctx_: *mut HashCtx) -> c_int {
        barrier(|| {
            // SAFETY: forwarded to the caller.
            let r = unsafe { ctx(ctx_) }.and_then(|c| {
                // SAFETY: the caller initialized the context.
                let h = unsafe { c.hasher() }?;
                let mut d = [0; MAX_SIZE];
                let collision = h.finalize_raw(&mut d);
                let n = h.algorithm().size();
                // SAFETY: the caller's contract on `out`; `d` is a distinct
                // local.
                unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, n) };
                if collision {
                    Err(collision_error())
                } else {
                    Ok(())
                }
            });
            status(r)
        })
    }

    /// Write `d` to `out` (the digest's size).
    ///
    /// # Safety
    ///
    /// `out` is writable for `d`'s size.
    unsafe fn write_digest(out: *mut u8, d: &Digest) {
        let b = d.as_bytes();
        // SAFETY: the caller's contract; `b` is a distinct local.
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
    }

    /// `git_hash_buf`.
    ///
    /// # Safety
    ///
    /// `data` is readable for `len` bytes (or `len` is 0); `out` is
    /// writable for the algorithm's digest size.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_buf(
        out: *mut u8,
        data: *const c_void,
        len: usize,
        algorithm: c_int,
    ) -> c_int {
        barrier(|| {
            let r = super::algorithm(algorithm).and_then(|a| {
                // SAFETY: the caller's contract on `data`.
                let data = unsafe { bytes(data.cast::<c_char>(), len) };
                let mut d = [0; MAX_SIZE];
                let collision = with_oneshot(a, |h| {
                    h.update(data);
                    h.finalize_raw(&mut d)
                });
                let d = Digest {
                    bytes: d,
                    len: a.size() as u8,
                };
                // SAFETY: the caller's contract on `out`.
                unsafe { write_digest(out, &d) };
                if collision {
                    Err(collision_error())
                } else {
                    Ok(())
                }
            });
            status(r)
        })
    }

    /// `git_hash_vec`.
    ///
    /// # Safety
    ///
    /// `vec` holds `n` entries (or `n` is 0), each readable for its `len`
    /// bytes; `out` is writable for the algorithm's digest size.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_vec(
        out: *mut u8,
        vec: *const GitStrVec,
        n: usize,
        algorithm: c_int,
    ) -> c_int {
        barrier(|| {
            let r = super::algorithm(algorithm).and_then(|a| {
                let vec: &[GitStrVec] = if n == 0 || vec.is_null() {
                    &[]
                } else {
                    // SAFETY: the caller passes `n` entries.
                    unsafe { core::slice::from_raw_parts(vec, n) }
                };
                let mut d = [0; MAX_SIZE];
                let collision = with_oneshot(a, |h| {
                    for v in vec {
                        // SAFETY: the caller's contract on each entry.
                        h.update(unsafe { bytes(v.data.cast::<c_char>(), v.len) });
                    }
                    h.finalize_raw(&mut d)
                });
                let d = Digest {
                    bytes: d,
                    len: a.size() as u8,
                };
                // SAFETY: the caller's contract on `out`.
                unsafe { write_digest(out, &d) };
                if collision {
                    Err(collision_error())
                } else {
                    Ok(())
                }
            });
            status(r)
        })
    }

    /// `git_hash_fmt`: `hash_len` bytes of `hash` in hexadecimal, and a
    /// NUL.
    ///
    /// # Safety
    ///
    /// `hash` is readable for `hash_len` bytes; `out` is writable for
    /// `2 * hash_len + 1` bytes and does not overlap `hash`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hash_fmt(
        out: *mut c_char,
        hash: *const u8,
        hash_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract on `hash`.
            let hash = unsafe { bytes(hash.cast::<c_char>(), hash_len) };
            const HEX: &[u8; 16] = b"0123456789abcdef";
            // SAFETY: the caller's contract on `out` (writable, maybe
            // uninitialized, so only written), disjoint from `hash`.
            let out = unsafe {
                core::slice::from_raw_parts_mut(out.cast::<MaybeUninit<u8>>(), 2 * hash_len + 1)
            };
            for (o, b) in out.as_chunks_mut::<2>().0.iter_mut().zip(hash) {
                o[0].write(HEX[(b >> 4) as usize]);
                o[1].write(HEX[(b & 0x0f) as usize]);
            }
            out[2 * hash_len].write(0);
            0
        })
    }
}

crate::c_layout_test! {
    headers: ["hash.h"],
    types: [
        HashCtx => "git_hash_ctx" {},
        GitStrVec => "git_str_vec" { data, len },
    ],
    asserts: [
        "GIT_HASH_ALGORITHM_NONE == 0",
        "GIT_HASH_ALGORITHM_SHA1 == 1",
        "GIT_HASH_ALGORITHM_SHA256 == 2",
        "GIT_HASH_MAX_SIZE == 32",
        "sizeof(git_hash_algorithm_t) == sizeof(int)",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_answers() {
        let _g = crate::util::test_support::init();
        let d = hash_buf(b"", HashAlgorithm::Sha1).unwrap();
        assert_eq!(d.to_hex(), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        let d = hash_buf(b"hello world", HashAlgorithm::Sha1).unwrap();
        assert_eq!(d.to_hex(), "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
        let d = hash_vec(&[b"hello", b" ", b"world"], HashAlgorithm::Sha1).unwrap();
        assert_eq!(d.to_hex(), "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }

    #[test]
    fn sha256_known_answers() {
        let _g = crate::util::test_support::init();
        let d = hash_buf(b"abc", HashAlgorithm::Sha256).unwrap();
        assert_eq!(
            d.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(d.as_bytes().len(), 32);
    }

    #[test]
    fn ctx_unknown_algorithm() {
        let _g = crate::util::test_support::init();
        let mut c = HashCtx::new(HashAlgorithm::Sha1);
        assert_eq!(c.init(7), Err(ErrorCode::GENERIC));
        assert_eq!(
            crate::util::errors::last(),
            (ErrorClass::INTERNAL, b"unknown hash algorithm".to_vec())
        );
        // SAFETY: an unknown algorithm is recorded: the computation is not
        // read.
        assert!(unsafe { c.hasher() }.is_err());
        assert!(c.restart().is_err());
        assert!(c.init(raw_algorithm::SHA256).is_ok());
        // SAFETY: just initialized.
        let h = unsafe { c.hasher() }.unwrap();
        h.update(b"abc");
        assert_eq!(h.finalize().unwrap().to_hex()[..8], *"ba7816bf");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(raw_algorithm::NONE), 0);
        assert_eq!(size(raw_algorithm::SHA1), 20);
        assert_eq!(size(raw_algorithm::SHA256), 32);
        assert_eq!(size(-1), 0);
    }
}
