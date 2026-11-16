//! One object shared by several threads through dup/free and the accessors,
//! as the cache shares objects (review l-obj-r1, LO-16, adopted). The tag
//! comes from `git_object__from_raw`, so the whole chain (refcount, the
//! cache's `git_cached_obj_decref`, `git_object__free`, the tag's free) is
//! Rust, and Miri runs it.

// One justified block per thread step.
#![allow(clippy::multiple_unsafe_ops_per_block)]

use core::ffi::c_char;
use core::ptr;

use libgit2_rs::__test::object::{GIT_OBJECT_TAG, GitObject, ffi as o};
use libgit2_rs::__test::tag::{GitTag, ffi as t};

/// The shared object pointer.
struct Shared(*mut GitObject);
// SAFETY: the object is shared as the cache shares it: its header is read
// only and its count atomic; each thread uses only references it took.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

#[test]
fn shared_tag_dup_free_accessors() {
    let _g = libgit2_rs::__test::util::test_support::init();
    let text: &'static [u8] = b"object e90810b8df3e80c413d903f631643c716887138d\ntype blob\ntag t\ntagger A U Thor <a@b.c> 1234567890 +0100\n\nmessage\n";
    let mut obj: *mut GitObject = ptr::null_mut();
    // SAFETY: a static buffer (outliving the object) and a writable out.
    let rc = unsafe {
        o::git_object__from_raw(
            &raw mut obj,
            text.as_ptr().cast::<c_char>(),
            text.len(),
            GIT_OBJECT_TAG,
            1,
        )
    };
    assert_eq!(rc, 0);
    let shared = Shared(obj);
    let s = &shared;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(move || {
                for _ in 0..if cfg!(miri) { 5 } else { 2000 } {
                    let mut d: *mut GitObject = ptr::null_mut();
                    // SAFETY: the shared object is live (the main thread holds
                    // a reference until the scope ends); each step uses the
                    // reference this thread took, given back at the end.
                    unsafe {
                        assert_eq!(o::git_object_dup(&raw mut d, s.0), 0);
                        let _ = *o::git_object_id(d);
                        assert_eq!(o::git_object_type(d), GIT_OBJECT_TAG);
                        let tg = d.cast::<GitTag>();
                        assert_eq!(*t::git_tag_name(tg) as u8, b't');
                        assert!(!t::git_tag_tagger(tg).is_null());
                        assert_eq!(*t::git_tag_message(tg) as u8, b'm');
                        let _ = *t::git_tag_target_id(tg);
                        o::git_object_free(d);
                    }
                }
            });
        }
    });
    // SAFETY: the main thread's reference, the last one.
    unsafe { o::git_object_free(shared.0) };
}
