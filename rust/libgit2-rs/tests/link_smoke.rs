#![cfg(all(feature = "link-c", not(miri)))]
use libgit2_rs as _;
unsafe extern "C" {
    fn git_libgit2_init() -> i32;
    fn git_libgit2_shutdown() -> i32;
}
#[test]
fn links_c() {
    // SAFETY: `git_libgit2_init` may be called at any time.
    assert!(unsafe { git_libgit2_init() } > 0);
    // SAFETY: pairs with the init above.
    unsafe { git_libgit2_shutdown() };
}
