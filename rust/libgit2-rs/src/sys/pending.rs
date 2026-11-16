//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

// ===========================================================================
// BEGIN PENDING(u2) (WP u3's declarations of the hash, for `filebuf` and
// `futils`; replaced by u2's Rust `git_hash_ctx` and hash API when it lands)
// ===========================================================================

use core::ffi::{c_int, c_void};

// PENDING(u2): `src/util/hash.c`. `git_hash_ctx` is embedded by value in
// `git_filebuf`: its storage is mirrored here by size and alignment (checked
// by `util::filebuf`'s layout test) until the hash port defines the type.

/// `git_hash_ctx`'s storage (`struct git_hash_ctx`: a union of the SHA-1
/// and SHA-256 contexts, and the algorithm), opaque. PENDING(u2)
#[repr(C, align(8))]
pub struct GitHashCtx {
    _opaque: [u8; 2408],
}

impl GitHashCtx {
    /// A zeroed context (`{0}`), what C's `memset` leaves.
    pub const fn zeroed() -> Self {
        GitHashCtx { _opaque: [0; 2408] }
    }
}

/// `GIT_HASH_ALGORITHM_SHA1`. PENDING(u2)
pub const GIT_HASH_ALGORITHM_SHA1: c_int = 1;
/// `GIT_HASH_ALGORITHM_SHA256`. PENDING(u2)
pub const GIT_HASH_ALGORITHM_SHA256: c_int = 2;
/// `GIT_HASH_SHA256_SIZE`. PENDING(u2)
pub const GIT_HASH_SHA256_SIZE: usize = 32;
/// `GIT_HASH_MAX_SIZE`. PENDING(u2)
pub const GIT_HASH_MAX_SIZE: usize = 32;

unsafe extern "C" {
    // PENDING(u2)
    pub fn git_hash_ctx_init(ctx: *mut GitHashCtx, algorithm: c_int) -> c_int;
    // PENDING(u2)
    pub fn git_hash_ctx_cleanup(ctx: *mut GitHashCtx);
    // PENDING(u2)
    pub fn git_hash_update(ctx: *mut GitHashCtx, data: *const c_void, len: usize) -> c_int;
    // PENDING(u2)
    pub fn git_hash_final(out: *mut u8, ctx: *mut GitHashCtx) -> c_int;
    // PENDING(u2)
    pub fn git_hash_buf(out: *mut u8, data: *const c_void, len: usize, algorithm: c_int) -> c_int;
}

/// `git_hash_buf` with SHA-256 over `data`. PENDING(u2)
pub fn hash_sha256(data: &[u8]) -> crate::util::errors::Result<[u8; GIT_HASH_SHA256_SIZE]> {
    let mut out = [0u8; GIT_HASH_SHA256_SIZE];
    // SAFETY: `out` has room for a SHA-256 digest and `data` is readable for
    // its length; `git_hash_buf` keeps neither.
    let r = unsafe {
        git_hash_buf(
            out.as_mut_ptr(),
            data.as_ptr().cast(),
            data.len(),
            GIT_HASH_ALGORITHM_SHA256,
        )
    };
    crate::util::errors::ErrorCode::check(r)?;
    Ok(out)
}

// ===========================================================================
// END PENDING(u2)
// ===========================================================================

// PENDING(rand): `src/util/rand.c` (in PORTING.md §1's list; the WP that
// ports it was not named in u3's brief).
unsafe extern "C" {
    fn git_rand_next() -> u64;
}

/// `git_rand_next`. PENDING(rand)
pub fn rand_next() -> u64 {
    // SAFETY: `git_rand_next` has no preconditions (it is seeded by the
    // library's initialization, and falls back to a fixed seed otherwise).
    unsafe { git_rand_next() }
}

// PENDING(pool): `src/util/pool.c` (in PORTING.md §1's list; the WP that
// ports it was not named in u3's brief).
unsafe extern "C" {
    /// `git_pool_malloc`. PENDING(pool)
    pub fn git_pool_malloc(pool: *mut c_void, items: usize) -> *mut c_void;
}
