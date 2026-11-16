//! The exports of pack.c, mwindow.c and odb_pack.c, called as C calls them
//! (raw pointers, uninitialized out-parameters, C callbacks), on the test
//! fixtures.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
#![cfg(not(miri))]

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::MaybeUninit;
use core::ptr;
use std::ffi::CString;

use libgit2_rs::__test::mwindow::{GitMwindow, GitMwindowFile};
use libgit2_rs::__test::odb_pack::{GitOdbBackend, GitOdbBackendPackOptions};
use libgit2_rs::__test::oid::{GIT_OID_SHA1, GIT_OID_SHA256, git_oid_t};
use libgit2_rs::__test::pack::ffi::ForeachEntryOffsetCb;
use libgit2_rs::__test::pack::{
    GIT_OBJECT_BLOB, GIT_OBJECT_COMMIT, GIT_PACKFILE_OFS_DELTA, GIT_PACKFILE_REF_DELTA,
    GitPackEntry, GitPackFile, GitPackfileStream, GitRawobj,
};
use libgit2_rs::__test::util::{errors, test_support};
use libgit2_rs::{Oid, OidType};

// The C ABI under test, declared as C declares it (the signatures are the
// `src` exports').
// The handles are Rust types C sees as opaque: it only passes pointers to them.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn get_delta_base(
        delta_base_out: *mut i64,
        p: *mut GitPackFile,
        w_curs: *mut *mut GitMwindow,
        curpos: *mut i64,
        type_: c_int,
        delta_obj_offset: i64,
    ) -> c_int;
    fn git_mwindow_close(window: *mut *mut GitMwindow);
    fn git_mwindow_contains(win: *mut GitMwindow, offset: i64, extra: i64) -> c_int;
    fn git_mwindow_free_all(mwf: *mut GitMwindowFile) -> c_int;
    fn git_mwindow_get_pack(
        out: *mut *mut GitPackFile,
        path: *const c_char,
        oid_type: git_oid_t,
    ) -> c_int;
    fn git_mwindow_open(
        mwf: *mut GitMwindowFile,
        cursor: *mut *mut GitMwindow,
        offset: i64,
        extra: usize,
        left: *mut c_uint,
    ) -> *mut u8;
    fn git_mwindow_put_pack(pack: *mut GitPackFile) -> c_int;
    fn git_odb_backend_one_pack(
        backend_out: *mut *mut GitOdbBackend,
        idx: *const c_char,
        opts: *const GitOdbBackendPackOptions,
    ) -> c_int;
    fn git_odb_backend_pack(
        backend_out: *mut *mut GitOdbBackend,
        objects_dir: *const c_char,
        opts: *const GitOdbBackendPackOptions,
    ) -> c_int;
    fn git_odb_backend_pack_options_init(
        opts: *mut GitOdbBackendPackOptions,
        version: c_uint,
    ) -> c_int;
    fn git_pack__lookup_id(
        id_lookup_table: *const c_void,
        stride: usize,
        lo: c_uint,
        hi: c_uint,
        id_prefix: *const u8,
        oid_type: git_oid_t,
    ) -> c_int;
    fn git_pack_entry_find(
        e: *mut GitPackEntry,
        p: *mut GitPackFile,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int;
    fn git_pack_foreach_entry(
        p: *mut GitPackFile,
        cb: libgit2_rs::__test::sys::midx::OdbForeachCb,
        data: *mut c_void,
    ) -> c_int;
    fn git_pack_foreach_entry_offset(
        p: *mut GitPackFile,
        cb: ForeachEntryOffsetCb,
        data: *mut c_void,
    ) -> c_int;
    fn git_packfile__name(out: *mut *mut c_char, path: *const c_char) -> c_int;
    fn git_packfile__object_header(
        out: *mut usize,
        hdr: *mut u8,
        size: usize,
        type_: c_int,
    ) -> c_int;
    fn git_packfile_alloc(
        pack_out: *mut *mut GitPackFile,
        path: *const c_char,
        oid_type: git_oid_t,
    ) -> c_int;
    fn git_packfile_free(p: *mut GitPackFile, unlink_packfile: bool);
    fn git_packfile_resolve_header(
        size_p: *mut usize,
        type_p: *mut c_int,
        p: *mut GitPackFile,
        offset: i64,
    ) -> c_int;
    fn git_packfile_stream_dispose(obj: *mut GitPackfileStream);
    fn git_packfile_stream_open(
        obj: *mut GitPackfileStream,
        p: *mut GitPackFile,
        curpos: i64,
    ) -> c_int;
    fn git_packfile_stream_read(
        obj: *mut GitPackfileStream,
        buffer: *mut c_void,
        len: usize,
    ) -> isize;
    fn git_packfile_unpack(obj: *mut GitRawobj, p: *mut GitPackFile, obj_offset: *mut i64)
    -> c_int;
    fn git_packfile_unpack_header(
        size_p: *mut usize,
        type_p: *mut c_int,
        p: *mut GitPackFile,
        w_curs: *mut *mut GitMwindow,
        curpos: *mut i64,
    ) -> c_int;
}

const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/resources");
/// A commit of `testrepo.git`'s big pack (`pack-a81e…`).
const COMMIT: &[u8] = b"007e075337848055a92e218bdfe137451a4c9635";

fn msg() -> String {
    String::from_utf8_lossy(&errors::last_message().unwrap_or_default()).into_owned()
}

fn idx_path(repo: &str, prefix: &str) -> CString {
    let dir = format!("{RES}/{repo}/objects/pack");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".idx"))
        .collect();
    names.sort();
    CString::new(format!("{dir}/{}", names[0])).unwrap()
}

unsafe extern "C" fn count_cb(_: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: the tests pass a `usize` counter.
    let n = unsafe { &mut *payload.cast::<usize>() };
    *n += 1;
    if *n == 3 { 42 } else { 0 }
}

unsafe extern "C" fn count_all_cb(_: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: the tests pass a `usize` counter.
    unsafe { *payload.cast::<usize>() += 1 };
    0
}

unsafe extern "C" fn offset_cb(_: *const Oid, off: i64, payload: *mut c_void) -> c_int {
    // SAFETY: the tests pass a `Vec<i64>`.
    unsafe { (*payload.cast::<Vec<i64>>()).push(off) };
    0
}

#[test]
fn alloc_find_unpack_free() {
    let _g = test_support::init();
    let path = idx_path("testrepo.git", "pack-a81e");
    // SAFETY: as C calls them.
    unsafe {
        let mut p = MaybeUninit::<*mut GitPackFile>::uninit();
        assert_eq!(
            git_packfile_alloc(p.as_mut_ptr(), path.as_ptr(), GIT_OID_SHA1),
            0
        );
        let p = p.assume_init();
        // C reads these fields.
        assert!((*p).name().to_bytes().ends_with(b".pack"));
        assert_eq!((*p).mwf.fd(), -1);
        assert!((*p).mwf.size() > 0);

        let id = Oid::from_hex(COMMIT, OidType::Sha1).unwrap();
        let mut e = MaybeUninit::<GitPackEntry>::uninit();
        assert_eq!(git_pack_entry_find(e.as_mut_ptr(), p, &id, 40), 0);
        let e = e.assume_init();
        assert_eq!((e.id, e.p), (id, p));
        assert!((*p).mwf.fd() >= 0, "the pack file is open after a find");

        let mut size = 0usize;
        let mut kind = 0;
        assert_eq!(
            git_packfile_resolve_header(&mut size, &mut kind, p, e.offset),
            0
        );
        assert_eq!(kind, GIT_OBJECT_COMMIT);

        let mut raw = MaybeUninit::<GitRawobj>::uninit();
        let mut off = e.offset;
        assert_eq!(git_packfile_unpack(raw.as_mut_ptr(), p, &mut off), 0);
        let raw = raw.assume_init();
        assert_eq!((raw.len, raw.type_), (size, GIT_OBJECT_COMMIT));
        assert!(off > e.offset);
        assert_eq!(*raw.data.cast::<u8>().add(raw.len), 0);
        assert!(std::slice::from_raw_parts(raw.data.cast::<u8>(), 5) == b"tree ");
        libgit2_rs::__test::util::alloc::free(raw.data);

        // The header, by hand, and a stream over the data.
        let mut w: *mut GitMwindow = ptr::null_mut();
        let mut curpos = e.offset;
        assert_eq!(
            git_packfile_unpack_header(&mut size, &mut kind, p, &mut w, &mut curpos),
            0
        );
        assert!(w.is_null(), "unpack_header closes its window");
        let mut s = MaybeUninit::<GitPackfileStream>::uninit();
        assert_eq!(git_packfile_stream_open(s.as_mut_ptr(), p, curpos), 0);
        let mut out = Vec::new();
        loop {
            let mut buf = [0u8; 7];
            let n = git_packfile_stream_read(s.as_mut_ptr(), buf.as_mut_ptr().cast(), buf.len());
            if n == -6 {
                continue;
            }
            assert!(n >= 0);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        assert_eq!(out.len(), size);
        assert_eq!((*s.as_ptr()).done, 1);
        git_packfile_stream_dispose(s.as_mut_ptr());

        // Missing and ambiguous.
        let missing =
            Oid::from_hex(b"ffffffffffffffffffffffffffffffffffffffff", OidType::Sha1).unwrap();
        let mut e2 = MaybeUninit::<GitPackEntry>::uninit();
        assert_eq!(git_pack_entry_find(e2.as_mut_ptr(), p, &missing, 40), -3);
        assert_eq!(
            msg(),
            "object not found - failed to find offset for pack entry (ffffffffffffffffffffffffffffffffffffffff)"
        );
        assert_eq!(
            git_pack_entry_find(e2.as_mut_ptr(), p, &id.prefix(1), 1),
            -5
        );
        assert_eq!(
            msg(),
            "ambiguous OID prefix - found multiple offsets for pack entry"
        );
        assert_eq!(
            git_pack_entry_find(e2.as_mut_ptr(), ptr::null_mut(), &id, 40),
            -1
        );
        assert_eq!(msg(), "invalid argument: 'p'");

        // Iteration, with a callback that stops (the message is set only if
        // none is).
        errors::clear();
        let mut n = 0usize;
        assert_eq!(git_pack_foreach_entry(p, count_cb, (&raw mut n).cast()), 42);
        assert_eq!(msg(), "git_pack_foreach_entry callback returned 42");
        let mut offs: Vec<i64> = Vec::new();
        assert_eq!(
            git_pack_foreach_entry_offset(p, offset_cb, (&raw mut offs).cast()),
            0
        );
        assert_eq!(offs.len(), 1628);

        git_packfile_free(p, false);
        git_packfile_free(ptr::null_mut(), false);
    }
}

#[test]
fn delta_bases() {
    let _g = test_support::init();
    let path = idx_path("testrepo.git", "pack-a81e");
    // SAFETY: as C calls them.
    unsafe {
        let mut p = ptr::null_mut();
        assert_eq!(git_packfile_alloc(&mut p, path.as_ptr(), GIT_OID_SHA1), 0);
        let mut offs: Vec<i64> = Vec::new();
        assert_eq!(
            git_pack_foreach_entry_offset(p, offset_cb, (&raw mut offs).cast()),
            0
        );
        let mut deltas = 0;
        for &o in &offs {
            let mut w: *mut GitMwindow = ptr::null_mut();
            let (mut size, mut kind, mut curpos) = (0usize, 0, o);
            assert_eq!(
                git_packfile_unpack_header(&mut size, &mut kind, p, &mut w, &mut curpos),
                0
            );
            if kind == GIT_PACKFILE_OFS_DELTA || kind == GIT_PACKFILE_REF_DELTA {
                deltas += 1;
                let mut base = 0i64;
                assert_eq!(
                    get_delta_base(&mut base, p, &mut w, &mut curpos, kind, o),
                    0
                );
                assert!(offs.contains(&base));
                assert!(kind != GIT_PACKFILE_OFS_DELTA || base < o);
                git_mwindow_close(&mut w);
                let mut b2 = 0;
                assert_eq!(
                    get_delta_base(ptr::null_mut(), p, &mut w, &mut curpos, kind, o),
                    -1
                );
                assert_eq!(msg(), "invalid argument: 'delta_base_out'");
                let _ = &mut b2;
            }
            assert!(w.is_null());
        }
        assert!(deltas > 100, "the fixture has deltas");
        git_packfile_free(p, false);
    }
}

#[test]
fn names_headers_lookup_id() {
    let _g = test_support::init();
    // SAFETY: as C calls them.
    unsafe {
        let mut out = ptr::null_mut();
        assert_eq!(git_packfile__name(&mut out, c"dir/pack-1.idx".as_ptr()), 0);
        assert_eq!(CStr::from_ptr(out), c"dir/pack-1.pack");
        libgit2_rs::__test::util::alloc::free(out.cast());
        assert_eq!(git_packfile__name(&mut out, c"abc".as_ptr()), -3);
        assert_eq!(msg(), "object not found - invalid packfile path");

        let mut hdr = [0u8; 10];
        let mut n = 0usize;
        assert_eq!(
            git_packfile__object_header(&mut n, hdr.as_mut_ptr(), 1000, GIT_OBJECT_BLOB),
            0
        );
        assert_eq!(&hdr[..n], &[0xb8, 0x3e]);
        assert_eq!(
            git_packfile__object_header(&mut n, hdr.as_mut_ptr(), 1, 5 + 3),
            -1
        );

        let table: Vec<u8> = (0u8..8).flat_map(|i| [i * 2; 20]).collect();
        let mut probe = [0u8; 32];
        probe[..20].fill(6);
        assert_eq!(
            git_pack__lookup_id(
                table.as_ptr().cast(),
                20,
                0,
                8,
                probe.as_ptr(),
                GIT_OID_SHA1
            ),
            3
        );
        probe[..20].fill(7);
        assert_eq!(
            git_pack__lookup_id(
                table.as_ptr().cast(),
                20,
                0,
                8,
                probe.as_ptr(),
                GIT_OID_SHA1
            ),
            -5
        );
        assert_eq!(
            git_pack__lookup_id(ptr::null(), 20, 0, 0, probe.as_ptr(), GIT_OID_SHA1),
            -1
        );

        let mut p = ptr::null_mut();
        assert_eq!(
            git_packfile_alloc(&mut p, c"/nonexistent/x.idx".as_ptr(), GIT_OID_SHA1),
            -3
        );
        assert!(p.is_null());
        assert_eq!(msg(), "object not found - packfile not found");
        assert_eq!(git_packfile_alloc(&mut p, ptr::null(), GIT_OID_SHA1), -3);
        assert_eq!(msg(), "object not found - invalid packfile path");
    }
}

#[test]
fn the_pack_cache_shares_and_counts() {
    let _g = test_support::init();
    let path = idx_path("testrepo.git", "pack-d85f");
    // SAFETY: as C calls them.
    unsafe {
        let (mut a, mut b) = (ptr::null_mut(), ptr::null_mut());
        assert_eq!(git_mwindow_get_pack(&mut a, path.as_ptr(), GIT_OID_SHA1), 0);
        assert_eq!(git_mwindow_get_pack(&mut b, path.as_ptr(), 0), 0);
        assert_eq!(a, b);
        // Other tests of this process may hold it too.
        let n = (*a).refcount.get();
        assert!(n >= 2);
        assert_eq!(git_mwindow_put_pack(a), 0);
        assert_eq!((*b).refcount.get(), n - 1);
        assert_eq!(git_mwindow_put_pack(b), 0);
    }
}

#[test]
fn windows_and_cursors() {
    let _g = test_support::init();
    let path = idx_path("testrepo.git", "pack-a81e");
    // SAFETY: as C calls them.
    unsafe {
        let mut p = ptr::null_mut();
        assert_eq!(git_packfile_alloc(&mut p, path.as_ptr(), GIT_OID_SHA1), 0);
        let id = Oid::from_hex(COMMIT, OidType::Sha1).unwrap();
        let mut e = MaybeUninit::<GitPackEntry>::uninit();
        assert_eq!(git_pack_entry_find(e.as_mut_ptr(), p, &id, 40), 0);
        let mwf = &raw mut (*p).mwf;
        let size = (*p).mwf.size();
        let mut w: *mut GitMwindow = ptr::null_mut();
        let mut left: c_uint = 0;
        let d = git_mwindow_open(mwf, &mut w, 0, 20, &mut left);
        assert!(!d.is_null() && !w.is_null());
        assert_eq!(left as i64, size);
        assert_eq!(std::slice::from_raw_parts(d, 4), b"PACK");
        assert_eq!(git_mwindow_contains(w, 100, 20), 1);
        // Same window for a nearby offset.
        let w0 = w;
        let d2 = git_mwindow_open(mwf, &mut w, 100, 20, &mut left);
        assert_eq!((d2, w), (d.add(100), w0));
        // Past the end: refused, and the cursor released.
        let d3 = git_mwindow_open(mwf, &mut w, size + 10, 20, &mut left);
        assert!(d3.is_null() && w.is_null());
        assert!(msg().starts_with("invalid pack window offset"));
        git_mwindow_close(&mut w);
        // All windows unused: they can be freed.
        assert_eq!(git_mwindow_free_all(mwf), 0);
        // A window in use cannot.
        let _ = git_mwindow_open(mwf, &mut w, 0, 20, &mut left);
        assert_eq!(git_mwindow_free_all(mwf), -1);
        assert_eq!(msg(), "unrecoverable internal error: 'w->inuse_cnt == 0'");
        git_mwindow_close(&mut w);
        assert_eq!(git_mwindow_free_all(mwf), 0);
        git_packfile_free(p, false);
    }
}

fn backend_slot<T>(b: *mut GitOdbBackend, f: impl FnOnce(&GitOdbBackend) -> Option<T>) -> T {
    // SAFETY: a live backend.
    f(unsafe { &*b }).expect("slot set")
}

#[test]
fn pack_backend_table() {
    let _g = test_support::init();
    let objects = CString::new(format!("{RES}/testrepo.git/objects")).unwrap();
    // SAFETY: as C (the odb) calls them.
    unsafe {
        let mut opts = MaybeUninit::<GitOdbBackendPackOptions>::uninit();
        assert_eq!(git_odb_backend_pack_options_init(opts.as_mut_ptr(), 0), -1);
        assert_eq!(msg(), "invalid version 0 on git_odb_backend_pack_options");
        assert_eq!(git_odb_backend_pack_options_init(opts.as_mut_ptr(), 1), 0);
        let opts = opts.assume_init();
        assert_eq!(
            opts,
            GitOdbBackendPackOptions {
                version: 1,
                oid_type: 0
            }
        );

        let mut b = MaybeUninit::<*mut GitOdbBackend>::uninit();
        assert_eq!(
            git_odb_backend_pack(b.as_mut_ptr(), objects.as_ptr(), &opts),
            0
        );
        let b = b.assume_init();
        assert_eq!((*b).version, 1);
        assert!((*b).write.is_none() && (*b).writestream.is_none() && (*b).readstream.is_none());

        let id = Oid::from_hex(COMMIT, OidType::Sha1).unwrap();
        let read = backend_slot(b, |t| t.read);
        let (mut data, mut len, mut kind) = (ptr::null_mut(), 0usize, 0);
        assert_eq!(read(&mut data, &mut len, &mut kind, b, &id), 0);
        assert_eq!(kind, GIT_OBJECT_COMMIT);
        libgit2_rs::__test::util::alloc::free(data);

        let read_header = backend_slot(b, |t| t.read_header);
        let (mut hlen, mut hkind) = (0usize, 0);
        assert_eq!(read_header(&mut hlen, &mut hkind, b, &id), 0);
        assert_eq!((hlen, hkind), (len, kind));
        assert_eq!(read_header(ptr::null_mut(), &mut hkind, b, &id), -1);
        assert_eq!(msg(), "invalid argument: 'len_p'");

        let exists = backend_slot(b, |t| t.exists);
        assert_eq!(exists(b, &id), 1);
        let missing =
            Oid::from_hex(b"0000000000000000000000000000000000000001", OidType::Sha1).unwrap();
        assert_eq!(exists(b, &missing), 0);
        assert_eq!(
            msg(),
            "object not found - failed to find pack entry (0000000000000000000000000000000000000001)"
        );

        let exists_prefix = backend_slot(b, |t| t.exists_prefix);
        let mut out = Oid::from_hex(COMMIT, OidType::Sha1).unwrap();
        assert_eq!(exists_prefix(&mut out, b, &missing.prefix(10), 10), -3);
        assert_eq!(out, Oid::NONE, "C copies the zeroed entry's id");
        assert_eq!(exists_prefix(&mut out, b, &id.prefix(10), 10), 0);
        assert_eq!(out, id);

        let read_prefix = backend_slot(b, |t| t.read_prefix);
        let mut full = Oid::NONE;
        assert_eq!(
            read_prefix(
                &mut full,
                &mut data,
                &mut len,
                &mut kind,
                b,
                &id.prefix(8),
                8
            ),
            0
        );
        assert_eq!(full, id);
        libgit2_rs::__test::util::alloc::free(data);
        assert_eq!(
            read_prefix(&mut full, &mut data, &mut len, &mut kind, b, &id, 3),
            -5
        );

        let foreach = backend_slot(b, |t| t.foreach);
        let mut n = 0usize;
        assert_eq!(foreach(b, Some(count_all_cb), (&raw mut n).cast()), 0);
        assert!(n > 1600);
        assert_eq!(foreach(b, None, ptr::null_mut()), -1);
        assert_eq!(msg(), "invalid argument: 'cb'");

        let refresh = backend_slot(b, |t| t.refresh);
        assert_eq!(refresh(b), 0);

        let free = backend_slot(b, |t| t.free);
        free(b);

        // One pack; a missing one fails with -1.
        let mut one = ptr::null_mut();
        let path = idx_path("testrepo_256.git", "pack-b87f");
        let opts256 = GitOdbBackendPackOptions {
            version: 1,
            oid_type: GIT_OID_SHA256,
        };
        assert_eq!(
            git_odb_backend_one_pack(&mut one, path.as_ptr(), &opts256),
            0
        );
        let mut n = 0usize;
        assert_eq!(
            backend_slot(one, |t| t.foreach)(one, Some(count_all_cb), (&raw mut n).cast()),
            0
        );
        assert!(n > 0);
        backend_slot(one, |t| t.free)(one);
        let mut none = ptr::null_mut();
        assert_eq!(
            git_odb_backend_one_pack(&mut none, c"/nonexistent/p.idx".as_ptr(), ptr::null()),
            -1
        );
        assert!(none.is_null());
    }
}
