//! `src/libgit2/transaction.c` (C that stays C): the transaction that
//! `git_config_lock` returns.

use core::ffi::{c_int, c_void};

use crate::config::GitConfig;
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
/// `data` (with `git_config_unlock`) when it is committed or freed.
///
/// # Safety
///
/// `out` is writable; `cfg` is a live config, which the transaction keeps
/// (without a reference) until it is freed; `data` is what
/// `git_config_unlock` expects for `cfg`.
pub unsafe fn config_new(
    out: *mut *mut git_transaction,
    cfg: *mut GitConfig,
    data: *mut c_void,
) -> Result<()> {
    // SAFETY: per this function's contract.
    let r = unsafe { git_transaction_config_new(out, cfg.cast(), data) };
    ErrorCode::check(r).map(drop)
}
