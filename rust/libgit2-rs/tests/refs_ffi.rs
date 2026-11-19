//! The C ABI of refs.c, refdb.c, refdb_fs.c and reflog.c, called from Rust
//! with raw pointers as C would, on edge cases and NULLs.

#![cfg(feature = "link-c")]
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{null, null_mut};

use libgit2_rs::__test::oid::Oid;
use libgit2_rs::__test::refdb::{GitRefdb, git_refdb_backend, git_reference_iterator};
use libgit2_rs::__test::reflog::{GitReflog, GitReflogEntry};
use libgit2_rs::__test::refs::GitReference;
use libgit2_rs::__test::refs::ffi::ForeachNameCb;
use libgit2_rs::__test::repository::GitRepository as git_repository;
use libgit2_rs::__test::signature::GitSignature;
use libgit2_rs::__test::util::str::GitStr;
use libgit2_rs::__test::util::{errors, test_support};
use libgit2_rs::OidType;

// The C ABI under test, declared as C declares it (the signatures are the
// `src` exports').
// The handles are Rust types C sees as opaque: it only passes pointers to them.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn git_refdb_backend_fs(out: *mut *mut git_refdb_backend, repo: *mut git_repository) -> c_int;
    fn git_refdb_compress(db: *mut GitRefdb) -> c_int;
    fn git_refdb_exists(exists: *mut c_int, db: *mut GitRefdb, ref_name: *const c_char) -> c_int;
    fn git_refdb_free(db: *mut GitRefdb);
    fn git_refdb_has_log(db: *mut GitRefdb, refname: *const c_char) -> c_int;
    fn git_refdb_init_backend(backend: *mut git_refdb_backend, version: c_uint) -> c_int;
    fn git_refdb_iterator(
        out: *mut *mut git_reference_iterator,
        db: *mut GitRefdb,
        glob: *const c_char,
    ) -> c_int;
    fn git_refdb_iterator_free(iter: *mut git_reference_iterator);
    fn git_refdb_iterator_next_name(
        out: *mut *const c_char,
        iter: *mut git_reference_iterator,
    ) -> c_int;
    fn git_refdb_lock(
        payload: *mut *mut c_void,
        db: *mut GitRefdb,
        refname: *const c_char,
    ) -> c_int;
    fn git_refdb_lookup(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        ref_name: *const c_char,
    ) -> c_int;
    fn git_refdb_open(out: *mut *mut GitRefdb, repo: *mut git_repository) -> c_int;
    fn git_refdb_resolve(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        ref_name: *const c_char,
        max_nesting: c_int,
    ) -> c_int;
    fn git_refdb_set_backend(db: *mut GitRefdb, backend: *mut git_refdb_backend) -> c_int;
    fn git_refdb_unlock(
        db: *mut GitRefdb,
        payload: *mut c_void,
        success: c_int,
        update_reflog: c_int,
        r: *const GitReference,
        sig: *const GitSignature,
        message: *const c_char,
    ) -> c_int;
    fn git_reference__alloc(
        name: *const c_char,
        oid: *const Oid,
        peel: *const Oid,
    ) -> *mut GitReference;
    fn git_reference__alloc_symbolic(
        name: *const c_char,
        target: *const c_char,
    ) -> *mut GitReference;
    fn git_reference__cmp_cb(a: *const c_void, b: *const c_void) -> c_int;
    fn git_reference__is_per_worktree_ref(ref_name: *const c_char) -> c_int;
    fn git_reference__is_pseudoref(ref_name: *const c_char) -> c_int;
    fn git_reference__lookup_loose(
        out: *mut *mut GitReference,
        ref_dir: *const c_char,
        ref_name: *const c_char,
        oid_type: c_int,
    ) -> c_int;
    fn git_reference__name_is_valid(
        valid: *mut c_int,
        refname: *const c_char,
        flags: c_uint,
    ) -> c_int;
    fn git_reference__normalize_name(buf: *mut GitStr, name: *const c_char, flags: c_uint)
    -> c_int;
    fn git_reference__realloc(
        ptr_to_ref: *mut *mut GitReference,
        name: *const c_char,
    ) -> *mut GitReference;
    fn git_reference__shorthand(name: *const c_char) -> *const c_char;
    fn git_reference_cmp(ref1: *const GitReference, ref2: *const GitReference) -> c_int;
    fn git_reference_dup(dest: *mut *mut GitReference, source: *mut GitReference) -> c_int;
    fn git_reference_foreach_name(
        repo: *mut git_repository,
        callback: ForeachNameCb,
        payload: *mut c_void,
    ) -> c_int;
    fn git_reference_free(reference: *mut GitReference);
    fn git_reference_is_branch(r: *const GitReference) -> c_int;
    fn git_reference_is_tag(r: *const GitReference) -> c_int;
    fn git_reference_name(r: *const GitReference) -> *const c_char;
    fn git_reference_name_is_valid(valid: *mut c_int, refname: *const c_char) -> c_int;
    fn git_reference_normalize_name(
        buffer_out: *mut c_char,
        buffer_size: usize,
        name: *const c_char,
        flags: c_uint,
    ) -> c_int;
    fn git_reference_shorthand(r: *const GitReference) -> *const c_char;
    fn git_reference_symbolic_target(r: *const GitReference) -> *const c_char;
    fn git_reference_target(r: *const GitReference) -> *const Oid;
    fn git_reference_target_peel(r: *const GitReference) -> *const Oid;
    fn git_reference_type(r: *const GitReference) -> c_int;
    fn git_reflog_append(
        reflog: *mut GitReflog,
        new_oid: *const Oid,
        committer: *const GitSignature,
        msg: *const c_char,
    ) -> c_int;
    fn git_reflog_delete(repo: *mut git_repository, name: *const c_char) -> c_int;
    fn git_reflog_drop(reflog: *mut GitReflog, idx: usize, rewrite_previous_entry: c_int) -> c_int;
    fn git_reflog_entry_byindex(reflog: *const GitReflog, idx: usize) -> *const GitReflogEntry;
    fn git_reflog_entry_committer(entry: *const GitReflogEntry) -> *const GitSignature;
    fn git_reflog_entry_id_new(entry: *const GitReflogEntry) -> *const Oid;
    fn git_reflog_entry_id_old(entry: *const GitReflogEntry) -> *const Oid;
    fn git_reflog_entry_message(entry: *const GitReflogEntry) -> *const c_char;
    fn git_reflog_entrycount(reflog: *mut GitReflog) -> usize;
    fn git_reflog_free(reflog: *mut GitReflog);
    fn git_reflog_read(
        reflog: *mut *mut GitReflog,
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int;
    fn git_reflog_rename(
        repo: *mut git_repository,
        old_name: *const c_char,
        new_name: *const c_char,
    ) -> c_int;
    fn git_reflog_write(reflog: *mut GitReflog) -> c_int;
    fn git_repository_free(repo: *mut git_repository);
    fn git_repository_open(repo_out: *mut *mut git_repository, path: *const c_char) -> c_int;
}

fn last() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

fn cs<'a>(p: *const c_char) -> &'a str {
    assert!(!p.is_null());
    // SAFETY: a NUL-terminated string the callee returned.
    unsafe { CStr::from_ptr(p) }.to_str().unwrap()
}

fn oid(s: &str) -> Oid {
    Oid::from_hex(s.as_bytes(), OidType::Sha1).unwrap()
}

#[test]
fn alloc_and_getters() {
    let _g = test_support::init();
    let id = oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
    let peel = oid("1111111111111111111111111111111111111111");
    // SAFETY: the C contract: valid strings and oids, references freed once.
    unsafe {
        let d = git_reference__alloc(c"refs/heads/x".as_ptr(), &id, &peel);
        assert!(!d.is_null());
        assert_eq!(cs(git_reference_name(d)), "refs/heads/x");
        assert_eq!(git_reference_type(d), 1);
        assert_eq!(*git_reference_target(d), id);
        assert_eq!(*git_reference_target_peel(d), peel);
        assert!(git_reference_symbolic_target(d).is_null());
        assert_eq!(cs(git_reference_shorthand(d)), "x");
        assert_eq!(git_reference_is_branch(d), 1);
        assert_eq!(git_reference_is_tag(d), 0);
        let s = git_reference__alloc_symbolic(c"HEAD".as_ptr(), c"refs/heads/x".as_ptr());
        assert_eq!(git_reference_type(s), 2);
        assert_eq!(cs(git_reference_symbolic_target(s)), "refs/heads/x");
        assert!(git_reference_target(s).is_null());
        assert!(git_reference_target_peel(s).is_null());
        assert!(git_reference_cmp(s, d) < 0);
        assert_eq!(git_reference__cmp_cb(s.cast(), s.cast()), 0);

        let mut dup: *mut GitReference = null_mut();
        assert_eq!(git_reference_dup(&mut dup, d), 0);
        assert_eq!(git_reference_cmp(dup, d), 0);
        assert!((*dup).db.is_null());

        // The flexible name is reallocated in place of the old block.
        let mut p = dup;
        let r = git_reference__realloc(&mut p, c"refs/heads/a-much-much-longer-name".as_ptr());
        assert!(p.is_null());
        assert_eq!(
            cs(git_reference_name(r)),
            "refs/heads/a-much-much-longer-name"
        );
        assert_eq!(*git_reference_target(r), id);

        git_reference_free(r);
        git_reference_free(s);
        git_reference_free(d);
        git_reference_free(null_mut());

        // NULLs, with C's assertion messages.
        assert!(git_reference__alloc(null(), &id, null()).is_null());
        assert_eq!(last(), "invalid argument: 'name'");
        assert!(git_reference__alloc(c"x".as_ptr(), null(), null()).is_null());
        assert_eq!(last(), "invalid argument: 'oid'");
        assert!(git_reference__alloc_symbolic(c"x".as_ptr(), null()).is_null());
        assert_eq!(last(), "invalid argument: 'target'");
        assert!(git_reference_name(null()).is_null());
        assert_eq!(last(), "invalid argument: 'ref'");
        assert_eq!(git_reference_type(null()), -1);
        assert!(git_reference_target(null()).is_null());
        assert_eq!(git_reference_cmp(null(), null()), -1);
        assert_eq!(last(), "invalid argument: 'ref1'");
        assert!(git_reference__realloc(null_mut(), c"x".as_ptr()).is_null());
        assert_eq!(last(), "invalid argument: 'ptr_to_ref'");
    }
}

#[test]
fn names() {
    let _g = test_support::init();
    let mut buf = [0 as c_char; 16];
    // SAFETY: the C contract: `buf` writable for its length.
    unsafe {
        assert_eq!(
            git_reference_normalize_name(buf.as_mut_ptr(), 16, c"refs//heads/x".as_ptr(), 0),
            0
        );
        assert_eq!(cs(buf.as_ptr()), "refs/heads/x");
        assert_eq!(
            git_reference_normalize_name(buf.as_mut_ptr(), 12, c"refs/heads/x".as_ptr(), 0),
            -6
        );
        assert_eq!(
            last(),
            "the provided buffer is too short to hold the normalization of 'refs/heads/x'"
        );
        // A size of 0 wraps in C's check, then git_str_copy_cstr refuses it.
        assert_eq!(
            git_reference_normalize_name(buf.as_mut_ptr(), 0, c"refs/heads/x".as_ptr(), 0),
            -1
        );
        assert_eq!(last(), "invalid argument: 'datasize'");
        assert_eq!(
            git_reference_normalize_name(buf.as_mut_ptr(), 16, c"refs/heads/".as_ptr(), 0),
            -12
        );
        assert_eq!(
            last(),
            "the given reference name 'refs/heads/' is not valid"
        );

        let mut valid = -1;
        assert_eq!(
            git_reference_name_is_valid(&mut valid, c"refs/heads/x".as_ptr()),
            0
        );
        assert_eq!(valid, 1);
        assert_eq!(git_reference_name_is_valid(&mut valid, c"x".as_ptr()), 0);
        assert_eq!(valid, 0);
        assert_eq!(git_reference__name_is_valid(&mut valid, null(), 0), -1);
        assert_eq!(last(), "unrecoverable internal error: 'valid && refname'");

        // The name may point into the output buffer: as C, it is read
        // after the buffer is cleared, which empties a name at its start
        // (an invalid name, as in C: rust/difftest/args_diff.c).
        let mut s = GitStr::new();
        s.puts(b"refs//heads//y").unwrap();
        let p = s.as_ptr();
        assert_eq!(git_reference__normalize_name(&mut s, p, 0), -12);
        assert_eq!(
            git_reference__normalize_name(&mut s, c"a..b".as_ptr(), 1),
            -12
        );
        assert_eq!(s.as_bytes(), b"");

        assert_eq!(
            cs(git_reference__shorthand(c"refs/remotes/o/m".as_ptr())),
            "o/m"
        );
        assert_eq!(git_reference__is_pseudoref(c"FETCH_HEAD".as_ptr()), 1);
        assert_eq!(
            git_reference__is_per_worktree_ref(c"refs/bisect/x".as_ptr()),
            1
        );
        assert_eq!(
            git_reference__is_per_worktree_ref(c"refs/heads/x".as_ptr()),
            0
        );
    }
}

struct Repo(*mut git_repository);

impl Repo {
    fn open(name: &str, tag: &str) -> (Repo, std::path::PathBuf) {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/resources")
            .join(name);
        let dst = std::env::temp_dir().join(format!("l-refs-ffi-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        let st = std::process::Command::new("cp")
            .arg("-r")
            .arg(&src)
            .arg(&dst)
            .status()
            .unwrap();
        assert!(st.success());
        let p = std::ffi::CString::new(dst.to_str().unwrap()).unwrap();
        let mut r = null_mut();
        // SAFETY: a path and a writable out-parameter.
        assert_eq!(unsafe { git_repository_open(&mut r, p.as_ptr()) }, 0);
        (Repo(r), dst)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: our repository.
        unsafe { git_repository_free(self.0) };
    }
}

#[test]
fn refdb_and_backends() {
    let _g = test_support::init();
    let (repo, dir) = Repo::open("testrepo.git", "refdb");
    // SAFETY: the C contract throughout: live objects, freed once.
    unsafe {
        let mut db: *mut GitRefdb = null_mut();
        assert_eq!(git_refdb_open(&mut db, repo.0), 0);
        let mut exists = 0;
        assert_eq!(
            git_refdb_exists(&mut exists, db, c"refs/heads/master".as_ptr()),
            0
        );
        assert_eq!(exists, 1);
        assert_eq!(
            git_refdb_exists(&mut exists, db, c"refs/heads/nope".as_ptr()),
            0
        );
        assert_eq!(exists, 0);
        let mut r: *mut GitReference = null_mut();
        assert_eq!(git_refdb_lookup(&mut r, db, c"HEAD".as_ptr()), 0);
        assert_eq!((*r).db, db);
        git_reference_free(r);
        assert_eq!(git_refdb_resolve(&mut r, db, c"HEAD".as_ptr(), 0), 0);
        assert_eq!(git_reference_type(r), 2);
        git_reference_free(r);
        assert_eq!(git_refdb_resolve(&mut r, db, c"HEAD".as_ptr(), -1), 0);
        assert_eq!(cs(git_reference_name(r)), "refs/heads/master");
        git_reference_free(r);

        let mut it: *mut git_reference_iterator = null_mut();
        assert_eq!(git_refdb_iterator(&mut it, db, c"refs/heads/*".as_ptr()), 0);
        let mut n = 0;
        let mut name: *const c_char = null();
        while git_refdb_iterator_next_name(&mut name, it) == 0 {
            assert!(cs(name).starts_with("refs/heads/"));
            n += 1;
        }
        assert!(n > 5);
        git_refdb_iterator_free(it);

        // Backends: version and completeness checks.
        let mut b = git_refdb_backend::default();
        assert_eq!(git_refdb_init_backend(&mut b, 2), -1);
        assert_eq!(last(), "invalid version 2 on git_refdb_backend");
        assert_eq!(git_refdb_init_backend(&mut b, 1), 0);
        assert_eq!(b.version, 1);
        assert_eq!(git_refdb_set_backend(db, &mut b), -21);
        assert_eq!(last(), "incomplete refdb backend implementation");
        b.version = 7;
        assert_eq!(git_refdb_set_backend(db, &mut b), -1);
        assert_eq!(last(), "invalid version 7 on git_refdb_backend");

        // A second filesystem backend, swapped in.
        let mut fs: *mut git_refdb_backend = null_mut();
        assert_eq!(git_refdb_backend_fs(&mut fs, repo.0), 0);
        assert_eq!(git_refdb_set_backend(db, fs), 0);
        assert_eq!(
            git_refdb_exists(&mut exists, db, c"refs/heads/master".as_ptr()),
            0
        );
        assert_eq!(exists, 1);
        assert_eq!(git_refdb_compress(db), 0);
        assert!(!dir.join("refs/heads/master").exists());

        let mut payload: *mut c_void = null_mut();
        assert_eq!(
            git_refdb_lock(&mut payload, db, c"refs/heads/locked".as_ptr()),
            0
        );
        assert!(dir.join("refs/heads/locked.lock").exists());
        assert_eq!(
            git_refdb_unlock(db, payload, 0, 0, null(), null(), null()),
            0
        );
        assert!(!dir.join("refs/heads/locked.lock").exists());

        assert_eq!(git_refdb_has_log(db, c"HEAD".as_ptr()), 1);
        assert_eq!(git_refdb_has_log(db, null()), -1);
        assert_eq!(last(), "invalid argument: 'refname'");
        git_refdb_free(db);
        git_refdb_free(null_mut());

        // Loose lookups without a result.
        let gitdir = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
        assert_eq!(
            git_reference__lookup_loose(null_mut(), gitdir.as_ptr(), c"HEAD".as_ptr(), 1),
            0
        );
        assert_eq!(
            git_reference__lookup_loose(null_mut(), gitdir.as_ptr(), c"nope".as_ptr(), 1),
            -3
        );
    }
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reflog_abi() {
    let _g = test_support::init();
    let (repo, dir) = Repo::open("testrepo.git", "reflog");
    // SAFETY: the C contract throughout.
    unsafe {
        let mut log: *mut GitReflog = null_mut();
        assert_eq!(git_reflog_read(&mut log, repo.0, c"HEAD".as_ptr()), 0);
        let n = git_reflog_entrycount(log);
        assert!(n > 1);
        let e = git_reflog_entry_byindex(log, 0);
        assert!(!e.is_null());
        assert_eq!(*git_reflog_entry_id_new(e), (*e).oid_cur);
        assert_eq!(*git_reflog_entry_id_old(e), (*e).oid_old);
        assert!(!git_reflog_entry_committer(e).is_null());
        assert!(git_reflog_entry_byindex(log, n).is_null());
        let committer = git_reflog_entry_committer(e);
        let id = oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
        assert_eq!(git_reflog_append(log, &id, committer, c"a\nb".as_ptr()), 0);
        assert_eq!(
            cs(git_reflog_entry_message(git_reflog_entry_byindex(log, 0))),
            "a b"
        );
        assert_eq!(git_reflog_drop(log, n + 5, 1), -3);
        assert_eq!(last(), format!("no reflog entry at index {}", n + 5));
        // Dropping the oldest clears the old id of the next one.
        assert_eq!(git_reflog_drop(log, n, 1), 0);
        assert!((*git_reflog_entry_byindex(log, n - 1)).oid_old.is_zero());
        assert_eq!(git_reflog_write(log), 0);
        git_reflog_free(log);
        git_reflog_free(null_mut());

        assert_eq!(git_reflog_entrycount(null_mut()), 0);
        assert_eq!(last(), "invalid argument: 'reflog'");
        assert!(git_reflog_entry_message(null()).is_null());
        assert_eq!(last(), "invalid argument: 'entry'");
        assert_eq!(git_reflog_append(null_mut(), &id, null(), null()), -1);
        assert_eq!(git_reflog_write(null_mut()), -1);
        assert_eq!(
            git_reflog_rename(
                repo.0,
                c"refs/heads/nope".as_ptr(),
                c"refs/heads/n2".as_ptr()
            ),
            -3
        );
        assert_eq!(git_reflog_delete(repo.0, c"refs/heads/nope".as_ptr()), 0);
    }
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn foreach_stops() {
    let _g = test_support::init();
    let (repo, dir) = Repo::open("testrepo.git", "foreach");
    unsafe extern "C" fn count(_name: *const c_char, payload: *mut c_void) -> c_int {
        // SAFETY: our payload.
        let n = unsafe { &mut *payload.cast::<c_int>() };
        *n += 1;
        if *n == 3 { 42 } else { 0 }
    }
    let mut n: c_int = 0;
    // As C: the callback's error is set only if no other one is.
    errors::clear();
    // SAFETY: a live repository; our callback and payload.
    let r = unsafe { git_reference_foreach_name(repo.0, count, (&raw mut n).cast()) };
    assert_eq!((r, n), (42, 3));
    // The first walk loads the configuration, which leaves a message
    // (`~/.gitconfig` missing) as in C (rust/difftest/refs_diff.c); the
    // second one sets the callback's.
    errors::clear();
    n = 0;
    // SAFETY: as above.
    let r = unsafe { git_reference_foreach_name(repo.0, count, (&raw mut n).cast()) };
    assert_eq!((r, n), (42, 3));
    assert_eq!(last(), "git_reference_foreach_name callback returned 42");
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}
