//! The C ABI of the objects component, called as C calls it: raw pointers,
//! out-parameters, the write stream's function table, argument checks and
//! their messages.

// The tests read C's structs field by field in one justified block each.
#![allow(clippy::multiple_unsafe_ops_per_block)]

use core::ffi::{CStr, c_char, c_int};
use core::ptr;
use std::path::{Path, PathBuf};

use libgit2_rs::Oid;
use libgit2_rs::blob::{GitWriteStream, ffi as blob};
use libgit2_rs::object::{GIT_OBJECT_ANY, GIT_OBJECT_BLOB, GIT_OBJECT_TAG, GitObject, ffi as obj};
use libgit2_rs::signature::ffi as sig;
use libgit2_rs::strarray::GitStrArray;
#[allow(non_camel_case_types)]
type git_repository = core::ffi::c_void;
unsafe extern "C" {
    fn git_repository_open(out: *mut *mut git_repository, path: *const c_char) -> c_int;
    fn git_repository_free(repo: *mut git_repository);
}
use libgit2_rs::tag::{GitTag, ffi as tag};
use libgit2_rs::util::errors;
use libgit2_rs::util::test_support;

fn last_message() -> Vec<u8> {
    errors::last_message().unwrap_or_default()
}

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

struct Repo {
    raw: *mut git_repository,
    dir: PathBuf,
}

impl Repo {
    fn scratch(name: &str) -> Repo {
        let dir = std::env::temp_dir().join(format!("l_obj_ffi-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&resources().join("testrepo.git"), &dir);
        let c = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
        let mut raw = ptr::null_mut();
        // SAFETY: a NUL-terminated path and a writable out.
        assert_eq!(unsafe { git_repository_open(&raw mut raw, c.as_ptr()) }, 0);
        Repo { raw, dir }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: the repository opened above, freed once.
        unsafe { git_repository_free(self.raw) };
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const TAG_TEXT: &[u8] = b"object e90810b8df3e80c413d903f631643c716887138d\n\
type commit\n\
tag v1.0\n\
tagger Vicent Marti <vicent@github.com> 1288977662 +0100\n\
\n\
This is a test tag\n";

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn tag_from_raw_fields_as_c_reads_them() {
    let _g = test_support::init();
    let mut o: *mut GitObject = ptr::null_mut();
    // SAFETY: a readable buffer that outlives the object; a writable out.
    let rc = unsafe {
        obj::git_object__from_raw(
            &raw mut o,
            TAG_TEXT.as_ptr().cast(),
            TAG_TEXT.len(),
            GIT_OBJECT_TAG,
            1,
        )
    };
    assert_eq!(rc, 0);
    let t = o.cast::<GitTag>();
    // SAFETY: a live tag, read through the exports C uses.
    unsafe {
        assert_eq!(tag::git_tag_target_type(t), 1);
        assert_eq!(CStr::from_ptr(tag::git_tag_name(t)).to_bytes(), b"v1.0");
        let tagger = tag::git_tag_tagger(t);
        assert_eq!(CStr::from_ptr((*tagger).name).to_bytes(), b"Vicent Marti");
        assert_eq!((*tagger).when.offset, 60);
        assert_eq!(
            CStr::from_ptr(tag::git_tag_message(t)).to_bytes(),
            b"This is a test tag\n"
        );
        assert_eq!(obj::git_object_type(o), GIT_OBJECT_TAG);
        assert!(obj::git_object_owner(o).is_null());
    }
    // A second reference, then both given back.
    let mut d = ptr::null_mut();
    // SAFETY: a live object and a writable out.
    assert_eq!(unsafe { obj::git_object_dup(&raw mut d, o) }, 0);
    // SAFETY: the two references, each given back once.
    unsafe {
        obj::git_object_free(d);
        obj::git_object_free(o);
    }

    // Invalid content: GIT_EINVALID, and the out-parameter NULL.
    let mut o = std::ptr::dangling_mut::<GitObject>();
    // SAFETY: as above.
    let rc = unsafe {
        obj::git_object__from_raw(
            &raw mut o,
            b"object x".as_ptr().cast(),
            8,
            GIT_OBJECT_TAG,
            1,
        )
    };
    assert_eq!(rc, -21);
    assert!(o.is_null());
    assert_eq!(last_message(), b"failed to parse tag: object field invalid");
    let mut valid = -1;
    // SAFETY: valid pointers.
    let rc = unsafe {
        obj::git_object_rawcontent_is_valid(
            &raw mut valid,
            b"object x".as_ptr().cast(),
            8,
            GIT_OBJECT_TAG,
            1,
        )
    };
    assert_eq!((rc, valid), (0, 0));
    // SAFETY: as above (a NULL buffer).
    let rc = unsafe {
        obj::git_object_rawcontent_is_valid(&raw mut valid, ptr::null(), 0, GIT_OBJECT_TAG, 1)
    };
    assert_eq!(rc, -1);
    assert_eq!(last_message(), b"invalid argument: 'buf'");
    // A type that is not an object type.
    // SAFETY: as above.
    let rc = unsafe {
        obj::git_object__from_raw(&raw mut o, b"x".as_ptr().cast(), 1, GIT_OBJECT_ANY, 1)
    };
    assert_eq!(rc, -3);
    assert_eq!(last_message(), b"the requested type is invalid");
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn argument_checks() {
    let _g = test_support::init();
    // SAFETY: NULL arguments, which the exports check.
    unsafe {
        assert!(obj::git_object_id(ptr::null()).is_null());
        assert_eq!(last_message(), b"invalid argument: 'obj'");
        assert_eq!(obj::git_object_type(ptr::null()), -1);
        assert_eq!(blob::git_blob_rawsize(ptr::null()), u64::MAX);
        assert_eq!(blob::git_blob_rawcontent(ptr::null()), ptr::null());
        let mut out = ptr::null_mut();
        assert_eq!(
            obj::git_object_lookup(&raw mut out, ptr::null_mut(), ptr::null(), GIT_OBJECT_ANY),
            -1
        );
        assert_eq!(last_message(), b"invalid argument: 'repo'");
        assert_eq!(
            obj::git_object_peel(&raw mut out, ptr::null(), GIT_OBJECT_BLOB),
            -1
        );
        assert_eq!(last_message(), b"invalid argument: 'object'");
        let mut so = ptr::null_mut();
        assert_eq!(
            sig::git_signature_new(&raw mut so, ptr::null(), c"e".as_ptr(), 0, 0),
            -1
        );
        assert_eq!(last_message(), b"invalid argument: 'name'");
        let mut valid = 7;
        assert_eq!(tag::git_tag_name_is_valid(&raw mut valid, ptr::null()), 0);
        assert_eq!(valid, 0);
        assert_eq!(
            obj::git_object_type2string(GIT_OBJECT_BLOB),
            c"blob".as_ptr()
        );
        assert!(*obj::git_object_type2string(99) == 0);
        assert_eq!(obj::git_object_string2type(c"tree".as_ptr()), 2);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn peel_errors_are_cs() {
    let _g = test_support::init();
    let r = Repo::scratch("peel");
    let id = Oid::from_str_hex(
        "1385f264afb75a56a5bec74243be9b367ba4ca08",
        libgit2_rs::OidType::Sha1,
    )
    .unwrap();
    let mut o = ptr::null_mut();
    // SAFETY: a live repository, a valid id, a writable out.
    let rv =
        unsafe { obj::git_object_lookup(&raw mut o, r.raw.cast(), &raw const id, GIT_OBJECT_BLOB) };
    assert_eq!(rv, 0);
    let mut p = ptr::null_mut();
    // SAFETY: a live object.
    let rc = unsafe { obj::git_object_peel(&raw mut p, o, 1) };
    assert_eq!(rc, -12);
    assert_eq!(
        last_message(),
        b"the git_object of id '1385f264afb75a56a5bec74243be9b367ba4ca08' can not be successfully peeled into a commit (git_object_t=1).".to_vec()
    );
    // SAFETY: as above, with an invalid target type.
    let rc = unsafe { obj::git_object_peel(&raw mut p, o, 7) };
    assert_eq!(rc, -1);
    assert!(last_message().starts_with(b"invalid argument: 'target_type == GIT_OBJECT_TAG ||"));
    // SAFETY: the reference taken above.
    unsafe { obj::git_object_free(o) };
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn write_stream_through_its_table() {
    let _g = test_support::init();
    let r = Repo::scratch("stream");
    let mut s: *mut GitWriteStream = ptr::null_mut();
    // SAFETY: a live repository, a writable out, a NUL-terminated hint.
    let rv = unsafe {
        blob::git_blob_create_from_stream(&raw mut s, r.raw.cast(), c"hint.txt".as_ptr())
    };
    assert_eq!(rv, 0);
    let data = b"streamed through the C table\n";
    for chunk in data.chunks(5) {
        // SAFETY: the stream's own slot, called with the stream, as C.
        let rc = unsafe { ((*s).write.unwrap())(s, chunk.as_ptr().cast::<c_char>(), chunk.len()) };
        assert_eq!(rc, 0);
    }
    let mut id = Oid::NONE;
    // SAFETY: a stream of `git_blob_create_from_stream`, not used after.
    let rv = unsafe { blob::git_blob_create_from_stream_commit(&raw mut id, s) };
    assert_eq!(rv, 0);
    assert_eq!(
        id,
        libgit2_rs::object::id_from_buffer(
            data,
            libgit2_rs::ObjectType::Blob,
            libgit2_rs::OidType::Sha1
        )
        .unwrap()
    );

    // close, then free, as filter.c's users do.
    let mut s: *mut GitWriteStream = ptr::null_mut();
    // SAFETY: as above, no hint.
    let rv = unsafe { blob::git_blob_create_from_stream(&raw mut s, r.raw.cast(), ptr::null()) };
    assert_eq!(rv, 0);
    // SAFETY: the stream's slots, each with the stream; `free` last.
    unsafe {
        assert_eq!(((*s).write.unwrap())(s, c"x".as_ptr(), 1), 0);
        assert_eq!(((*s).close.unwrap())(s), 0);
        ((*s).free.unwrap())(s);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn tag_lists_and_short_ids() {
    let _g = test_support::init();
    let r = Repo::scratch("lists");
    let mut a = GitStrArray {
        strings: ptr::null_mut(),
        count: 0,
    };
    // SAFETY: a writable array, a NUL-terminated pattern, a live repository.
    let rv = unsafe { tag::git_tag_list_match(&raw mut a, c"e*".as_ptr(), r.raw.cast()) };
    assert_eq!(rv, 0);
    assert_eq!(a.count, 1);
    // SAFETY: the array's first element.
    let first = unsafe { *a.strings };
    // SAFETY: a NUL-terminated string of the array.
    assert_eq!(unsafe { CStr::from_ptr(first) }.to_bytes(), b"e90810b");
    // SAFETY: the array libgit2 filled, freed once.
    unsafe { libgit2_rs::strarray::ffi::git_strarray_dispose(&raw mut a) };

    let mut n = 0;
    unsafe extern "C" fn count(_: *const c_char, _: *mut Oid, p: *mut core::ffi::c_void) -> c_int {
        // SAFETY: the counter passed below.
        unsafe { *p.cast::<c_int>() += 1 };
        0
    }
    // SAFETY: a live repository; the callback and its payload live for the call.
    let rv = unsafe { tag::git_tag_foreach(r.raw.cast(), Some(count), (&raw mut n).cast()) };
    assert_eq!(rv, 0);
    assert!(n >= 7);

    let id = Oid::from_str_hex(
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        libgit2_rs::OidType::Sha1,
    )
    .unwrap();
    let mut o = ptr::null_mut();
    // SAFETY: a live repository, a valid id, a writable out.
    let rv =
        unsafe { obj::git_object_lookup(&raw mut o, r.raw.cast(), &raw const id, GIT_OBJECT_ANY) };
    assert_eq!(rv, 0);
    let mut b = libgit2_rs::buf::GitBuf::new();
    // SAFETY: a valid `git_buf` and a live object.
    assert_eq!(unsafe { obj::git_object_short_id(&raw mut b, o) }, 0);
    assert_eq!(b.as_bytes(), b"a65fedf");
    // SAFETY: the reference taken above.
    unsafe { obj::git_object_free(o) };
}
