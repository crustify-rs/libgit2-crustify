//! OpenSSL's libcrypto SHA-256 (`SHA256_Init`/`_Update`/`_Final`), which
//! `src/util/hash/openssl.c` used in this configuration
//! (`GIT_SHA256_OPENSSL`) and which libgit2 links anyway (`USE_HTTPS`).
//! OpenSSL picks the fastest code for the CPU (SHA extensions, AVX2, …).
//!
//! `SHA256_CTX` is plain data: no pointers, no resources, nothing to free
//! (OpenSSL's own `SHA256_Final` only cleanses it). So it is mirrored by
//! value ([`Sha256Ctx`]) and lives in place, in `git_hash_ctx` or on the
//! stack, as the C's did; an owner (an `EVP_MD_CTX` behind a `CBox`)
//! would allocate per computation, which the C did not.

use core::ffi::{c_int, c_void};

/// `SHA256_CTX`, mirrored (`<openssl/sha.h>`; layout test in
/// `util/hash.rs`). Valid in any state for OpenSSL once `SHA256_Init`
/// wrote it; [`Sha256Ctx::new`] is the only constructor.
#[repr(C)]
#[derive(Clone)]
pub struct Sha256Ctx {
    h: [u32; 8],
    nl: u32,
    nh: u32,
    data: [u32; 16],
    num: u32,
    md_len: u32,
}

#[link(name = "crypto")]
unsafe extern "C" {
    fn SHA256_Init(c: *mut Sha256Ctx) -> c_int;
    fn SHA256_Update(c: *mut Sha256Ctx, data: *const c_void, len: usize) -> c_int;
    fn SHA256_Final(md: *mut u8, c: *mut Sha256Ctx) -> c_int;
}

impl Sha256Ctx {
    /// A started computation (`SHA256_Init`); `None` if OpenSSL fails
    /// (it does not, for SHA-256).
    pub fn new() -> Option<Sha256Ctx> {
        let mut c = Sha256Ctx {
            h: [0; 8],
            nl: 0,
            nh: 0,
            data: [0; 16],
            num: 0,
            md_len: 0,
        };
        c.init().then_some(c)
    }

    /// Restart (`SHA256_Init`); `false` if OpenSSL fails.
    pub fn init(&mut self) -> bool {
        // SAFETY: `self` is a `SHA256_CTX`, which `SHA256_Init` overwrites.
        unsafe { SHA256_Init(self) == 1 }
    }

    /// Hash `data` (`SHA256_Update`); `false` if OpenSSL fails.
    pub fn update(&mut self, data: &[u8]) -> bool {
        // SAFETY: `self` was initialized by `SHA256_Init` (the only
        // constructor calls it) and stays valid for OpenSSL in every state
        // it leaves; `data` is readable for its length.
        unsafe { SHA256_Update(self, data.as_ptr().cast(), data.len()) == 1 }
    }

    /// Write the digest (`SHA256_Final`); `false` if OpenSSL fails. As
    /// OpenSSL's, a later `update` or `finalize` without `init` continues
    /// from the state it leaves.
    pub fn finalize(&mut self, out: &mut [u8; 32]) -> bool {
        // SAFETY: as in `update`; `out` is writable for 32 bytes.
        unsafe { SHA256_Final(out.as_mut_ptr(), self) == 1 }
    }
}
