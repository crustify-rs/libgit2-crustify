//! `src/libgit2/mailmap.c` (stays C).

use core::ffi::{c_int, c_void};

use crate::commit::GitSignature;

unsafe extern "C" {
    /// `git_mailmap_resolve_signature`.
    pub fn git_mailmap_resolve_signature(
        out: *mut *mut GitSignature,
        mailmap: *const c_void,
        sig: *const GitSignature,
    ) -> c_int;
}

/// `git_mailmap_resolve_signature`.
///
/// # Safety
///
/// `out` is writable; `mailmap` is NULL or a live mailmap; `sig` is a live
/// signature (the C asserts it is not NULL).
pub unsafe fn resolve_signature(
    out: *mut *mut GitSignature,
    mailmap: *const c_void,
    sig: *const GitSignature,
) -> c_int {
    // SAFETY: `out` is writable, `mailmap` NULL or live, `sig` a live
    // signature, all for the call (this function's contract, which is the
    // C's).
    unsafe { git_mailmap_resolve_signature(out, mailmap, sig) }
}
