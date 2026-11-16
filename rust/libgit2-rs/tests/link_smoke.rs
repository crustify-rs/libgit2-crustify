#![cfg(all(feature = "link-c", not(miri)))]
use libgit2_rs as _;
unsafe extern "C" {
    fn git_libgit2_init() -> i32;
    fn git_libgit2_shutdown() -> i32;
}
#[test]
fn links_c() {
    // SAFETY: init/shutdown pair.
    unsafe {
        assert!(git_libgit2_init() > 0);
        git_libgit2_shutdown();
    }
}
