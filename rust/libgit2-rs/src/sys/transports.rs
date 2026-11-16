//! `src/libgit2/transports/*` (C that stays C): the global init function of
//! `git_libgit2_init`, and the globals the options write.

use core::ffi::c_int;
use core::sync::atomic::AtomicBool;

unsafe extern "C" {
    /// `git_transport_ssh_libssh2_global_init`.
    pub fn git_transport_ssh_libssh2_global_init() -> c_int;
    /// `git_smart__ofs_delta_enabled` (`bool`).
    pub static git_smart__ofs_delta_enabled: AtomicBool;
    /// `git_http__expect_continue` (`bool`).
    pub static git_http__expect_continue: AtomicBool;
}
