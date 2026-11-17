//! `src/libgit2/transaction.c` (C that stays C): the transaction that
//! `git_config_lock` returns.

use core::ffi::{c_int, c_void};

use crate::config::GitConfig;
use crate::util::cabi::Out;
use crate::util::errors::{ErrorCode, Result};

/// `git_transaction`, opaque.
#[repr(C)]
pub struct git_transaction {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn git_transaction_config_new(
        out: *mut *mut git_transaction,
        cfg: *mut c_void,
        data: *mut c_void,
    ) -> c_int;
}

/// `git_transaction_config_new`: a transaction over `cfg` that unlocks
/// `data` (with `git_config_unlock`) when it is committed or freed; it is
/// written to `out` (NULL fails as C's `GIT_ASSERT_ARG(out)`).
///
/// # Safety
///
/// `cfg` is a live config, which the transaction keeps (without a
/// reference) until it is freed; `data` is what `git_config_unlock`
/// expects for `cfg`.
pub unsafe fn config_new(
    out: Out<'_, *mut git_transaction>,
    cfg: *mut GitConfig,
    data: *mut c_void,
) -> Result<()> {
    // SAFETY: `out` is NULL (which the C rejects) or writable for the
    // call, and C writes it once, as `Out` would; `cfg` and `data` are as
    // the C requires (this function's contract); the C reads neither
    // during the call, it only stores them.
    let r = unsafe { git_transaction_config_new(out.as_ptr(), cfg.cast(), data) };
    ErrorCode::check(r).map(drop)
}
