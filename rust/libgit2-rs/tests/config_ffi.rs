//! The C ABI of the config modules, driven as C drives it: raw pointers,
//! C callbacks, a backend implemented as a C table, and the error state.
//! Everything here is Rust under the exports, so it runs under Miri but
//! for the tests that touch files or patterns.

// C-style test code: each test drives the exports under one contract.
#![allow(
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    clippy::borrow_as_ptr
)]

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::null_mut;

use libgit2_rs::__test::config::git_config_backend_memory_options;
use libgit2_rs::__test::config::{
    GIT_CONFIG_LEVEL_APP, GIT_CONFIG_LEVEL_GLOBAL, GIT_CONFIG_LEVEL_LOCAL, GitConfig,
    git_config_backend, git_config_backend_entry, git_config_entry, git_config_iterator,
    git_configmap,
};
use libgit2_rs::__test::repository::GitRepository;
use libgit2_rs::__test::util::errors::{self, ErrorCode};
use libgit2_rs::__test::util::test_support;

// The C ABI under test, declared as C declares it (the signatures are the
// `src` exports').
// The handles are Rust types C sees as opaque: it only passes pointers to them.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn git_config__get_bool_force(
        config: *const GitConfig,
        key: *const c_char,
        fallback_value: c_int,
    ) -> c_int;
    fn git_config__get_int_force(
        config: *const GitConfig,
        key: *const c_char,
        fallback_value: c_int,
    ) -> c_int;
    fn git_config__lookup_entry(
        out: *mut *mut git_config_entry,
        config: *const GitConfig,
        key: *const c_char,
        no_errors: bool,
    ) -> c_int;
    fn git_config__normalize_name(input: *const c_char, out: *mut *mut c_char) -> c_int;
    fn git_config_add_backend(
        config: *mut GitConfig,
        backend: *mut git_config_backend,
        level: c_int,
        repo: *const GitRepository,
        force: c_int,
    ) -> c_int;
    fn git_config_backend_foreach_match(
        backend: *mut git_config_backend,
        regexp: *const c_char,
        cb: Option<unsafe extern "C" fn(*const git_config_entry, *mut c_void) -> c_int>,
        payload: *mut c_void,
    ) -> c_int;
    fn git_config_backend_from_string(
        out: *mut *mut git_config_backend,
        cfg: *const c_char,
        len: usize,
        opts: *mut git_config_backend_memory_options,
    ) -> c_int;
    fn git_config_delete_entry(config: *mut GitConfig, name: *const c_char) -> c_int;
    fn git_config_entry_free(entry: *mut git_config_entry);
    fn git_config_foreach(
        config: *const GitConfig,
        cb: Option<unsafe extern "C" fn(*const git_config_entry, *mut c_void) -> c_int>,
        payload: *mut c_void,
    ) -> c_int;
    fn git_config_free(config: *mut GitConfig);
    fn git_config_get_bool(out: *mut c_int, config: *const GitConfig, name: *const c_char)
    -> c_int;
    fn git_config_get_entry(
        out: *mut *mut git_config_entry,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int;
    fn git_config_get_int32(out: *mut i32, config: *const GitConfig, name: *const c_char) -> c_int;
    fn git_config_get_multivar_foreach(
        config: *const GitConfig,
        name: *const c_char,
        regexp: *const c_char,
        cb: Option<unsafe extern "C" fn(*const git_config_entry, *mut c_void) -> c_int>,
        payload: *mut c_void,
    ) -> c_int;
    fn git_config_get_string(
        out: *mut *const c_char,
        config: *const GitConfig,
        name: *const c_char,
    ) -> c_int;
    fn git_config_init_backend(backend: *mut git_config_backend, version: c_uint) -> c_int;
    fn git_config_iterator_free(iter: *mut git_config_iterator);
    fn git_config_iterator_new(
        out: *mut *mut git_config_iterator,
        config: *const GitConfig,
    ) -> c_int;
    fn git_config_lookup_map_enum(
        type_out: *mut c_int,
        str_out: *mut *const c_char,
        maps: *const git_configmap,
        map_n: usize,
        enum_val: c_int,
    ) -> c_int;
    fn git_config_lookup_map_value(
        out: *mut c_int,
        maps: *const git_configmap,
        map_n: usize,
        value: *const c_char,
    ) -> c_int;
    fn git_config_new(out: *mut *mut GitConfig) -> c_int;
    fn git_config_next(entry: *mut *mut git_config_entry, iter: *mut git_config_iterator) -> c_int;
    fn git_config_open_level(
        out: *mut *mut GitConfig,
        parent: *const GitConfig,
        level: c_int,
    ) -> c_int;
    fn git_config_parse_bool(out: *mut c_int, value: *const c_char) -> c_int;
    fn git_config_parse_int64(out: *mut i64, value: *const c_char) -> c_int;
    fn git_config_set_string(
        config: *mut GitConfig,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    fn git_config_snapshot(out: *mut *mut GitConfig, config: *mut GitConfig) -> c_int;
}

fn last_message() -> String {
    String::from_utf8_lossy(&errors::last().1).into_owned()
}

unsafe fn cs<'a>(p: *const c_char) -> &'a str {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap()
}

unsafe fn memory_config(text: &[u8], level: c_int) -> *mut GitConfig {
    unsafe {
        let mut cfg = null_mut();
        assert_eq!(git_config_new(&mut cfg), 0);
        let mut b = null_mut();
        assert_eq!(
            git_config_backend_from_string(&mut b, text.as_ptr().cast(), text.len(), null_mut()),
            0
        );
        assert_eq!(
            git_config_add_backend(cfg, b, level, core::ptr::null(), 0),
            0
        );
        cfg
    }
}

#[test]
fn entries_and_errors() {
    let _g = test_support::init();
    unsafe {
        let cfg = memory_config(
            b"[a]\n\tb = 1\n\tc\n[d \"E\"]\n\tf = yes\n",
            GIT_CONFIG_LEVEL_LOCAL,
        );
        let mut e: *mut git_config_entry = null_mut();
        assert_eq!(git_config_get_entry(&mut e, cfg, c"D.E.F".as_ptr()), 0);
        assert_eq!(cs((*e).name), "d.E.f");
        assert_eq!(cs((*e).value), "yes");
        assert_eq!(cs((*e).backend_type), "in-memory");
        assert!((*e).origin_path.is_null());
        assert_eq!((*e).level, GIT_CONFIG_LEVEL_LOCAL);
        git_config_entry_free(e);

        let mut v = 0;
        assert_eq!(git_config_get_bool(&mut v, cfg, c"a.c".as_ptr()), 0);
        assert_eq!(v, 1);
        let mut n = 0i32;
        assert_eq!(git_config_get_int32(&mut n, cfg, c"a.b".as_ptr()), 0);
        assert_eq!(n, 1);
        assert_eq!(
            git_config_get_int32(&mut n, cfg, c"a.x".as_ptr()),
            ErrorCode::NOT_FOUND.raw()
        );
        assert_eq!(last_message(), "config value 'a.x' was not found");
        assert_eq!(
            git_config_get_bool(&mut v, cfg, c"a".as_ptr()),
            ErrorCode::INVALID_SPEC.raw()
        );
        assert_eq!(last_message(), "invalid config item name 'a'");

        // Lookups that do not fail.
        assert_eq!(git_config__get_int_force(cfg, c"a.b".as_ptr(), 7), 1);
        assert_eq!(git_config__get_int_force(cfg, c"a.zz".as_ptr(), 7), 7);
        assert_eq!(git_config__get_bool_force(cfg, c"d.E.f".as_ptr(), 0), 1);
        let mut e2 = null_mut();
        assert_eq!(
            git_config__lookup_entry(&mut e2, cfg, c"a.zz".as_ptr(), false),
            0
        );
        assert!(e2.is_null());

        // A read-only config has no writer.
        assert_eq!(
            git_config_set_string(cfg, c"a.b".as_ptr(), c"2".as_ptr()),
            ErrorCode::READ_ONLY.raw()
        );
        assert_eq!(
            last_message(),
            "cannot set 'a.b': the configuration is read-only"
        );
        assert_eq!(
            git_config_set_string(cfg, c"a.b".as_ptr(), core::ptr::null()),
            -1
        );
        assert_eq!(last_message(), "the value to set cannot be NULL");
        assert_eq!(
            git_config_delete_entry(cfg, c"a.b".as_ptr()),
            ErrorCode::READ_ONLY.raw()
        );

        // A snapshot is read-only: `get_string` works on it.
        let mut snap = null_mut();
        assert_eq!(git_config_snapshot(&mut snap, cfg), 0);
        let mut s = core::ptr::null();
        assert_eq!(git_config_get_string(&mut s, snap, c"a.c".as_ptr()), 0);
        assert_eq!(cs(s), "");
        git_config_free(snap);
        git_config_free(cfg);
    }
}

unsafe extern "C" fn count_cb(e: *const git_config_entry, payload: *mut c_void) -> c_int {
    unsafe {
        let n = payload.cast::<Vec<String>>();
        (*n).push(cs((*e).name).to_owned());
        if (*n).len() == 3 { 42 } else { 0 }
    }
}

#[test]
fn iterators_and_callbacks() {
    let _g = test_support::init();
    unsafe {
        let cfg = memory_config(b"[a]\nb=1\nb=2\nb=3\nc=4\n", GIT_CONFIG_LEVEL_LOCAL);
        let mut it: *mut git_config_iterator = null_mut();
        assert_eq!(git_config_iterator_new(&mut it, cfg), 0);
        let mut e = null_mut();
        let mut names = Vec::new();
        while git_config_next(&mut e, it) == 0 {
            names.push(cs((*e).value).to_owned());
        }
        assert_eq!(names, ["1", "2", "3", "4"]);
        assert_eq!(git_config_next(&mut e, it), ErrorCode::ITER_OVER.raw());
        git_config_iterator_free(it);

        // A callback that stops: its code, and the C's message.
        let mut seen: Vec<String> = Vec::new();
        let r = git_config_foreach(cfg, Some(count_cb), (&raw mut seen).cast());
        assert_eq!(r, 42);
        assert_eq!(seen.len(), 3);
        assert_eq!(
            last_message(),
            "git_config_foreach_match callback returned 42"
        );
        errors::clear();

        let mut seen: Vec<String> = Vec::new();
        let r = git_config_get_multivar_foreach(
            cfg,
            c"a.b".as_ptr(),
            core::ptr::null(),
            Some(count_cb),
            (&raw mut seen).cast(),
        );
        assert_eq!(r, 42);
        let mut seen: Vec<String> = Vec::new();
        let r = git_config_get_multivar_foreach(
            cfg,
            c"a.q".as_ptr(),
            core::ptr::null(),
            Some(count_cb),
            (&raw mut seen).cast(),
        );
        assert_eq!(r, ErrorCode::NOT_FOUND.raw());
        assert_eq!(last_message(), "config value 'a.q' was not found");
        git_config_free(cfg);
    }
}

// A backend implemented as a C table, as a C user writes one: one value,
// and a count of the calls of `free`.
#[repr(C)]
struct CBackend {
    parent: git_config_backend,
    frees: *mut c_int,
}

#[repr(C)]
struct CEntry {
    base: git_config_backend_entry,
}

unsafe extern "C" fn c_entry_free(e: *mut git_config_backend_entry) {
    drop(unsafe { Box::from_raw(e.cast::<CEntry>()) });
}

unsafe extern "C" fn c_open(
    _: *mut git_config_backend,
    _: c_int,
    _: *const libgit2_rs::__test::repository::GitRepository,
) -> c_int {
    0
}

unsafe extern "C" fn c_get(
    _: *mut git_config_backend,
    key: *const c_char,
    out: *mut *mut git_config_backend_entry,
) -> c_int {
    unsafe {
        if CStr::from_ptr(key) != c"c.v" {
            return ErrorCode::NOT_FOUND.raw();
        }
        let e = Box::new(CEntry {
            base: git_config_backend_entry {
                entry: git_config_entry {
                    name: c"c.v".as_ptr(),
                    value: c"from-c".as_ptr(),
                    backend_type: c"c".as_ptr(),
                    origin_path: core::ptr::null(),
                    include_depth: 0,
                    level: GIT_CONFIG_LEVEL_APP,
                },
                free: Some(c_entry_free),
            },
        });
        out.write(Box::into_raw(e).cast());
        0
    }
}

unsafe extern "C" fn c_free(b: *mut git_config_backend) {
    unsafe {
        let b = Box::from_raw(b.cast::<CBackend>());
        *b.frees += 1;
    }
}

#[test]
fn c_backends() {
    let _g = test_support::init();
    unsafe {
        let mut cfg = null_mut();
        assert_eq!(git_config_new(&mut cfg), 0);

        // A version the library does not know.
        let mut bad = git_config_backend {
            version: 1024,
            ..Default::default()
        };
        assert_eq!(
            git_config_add_backend(cfg, &mut bad, 0, core::ptr::null(), 0),
            -1
        );
        assert_eq!(last_message(), "invalid version 1024 on git_config_backend");

        let mut init: git_config_backend = core::mem::zeroed();
        assert_eq!(git_config_init_backend(&mut init, 1), 0);
        assert_eq!(init.version, 1);

        let mut frees = 0;
        let b = Box::into_raw(Box::new(CBackend {
            parent: git_config_backend {
                open: Some(c_open),
                get: Some(c_get),
                free: Some(c_free),
                ..Default::default()
            },
            frees: &mut frees,
        }));
        assert_eq!(
            git_config_add_backend(cfg, b.cast(), GIT_CONFIG_LEVEL_APP, core::ptr::null(), 0),
            0
        );
        let mut e = null_mut();
        assert_eq!(git_config_get_entry(&mut e, cfg, c"C.V".as_ptr()), 0);
        assert_eq!(cs((*e).value), "from-c");
        git_config_entry_free(e);
        // A slot the backend left NULL fails, where C would crash.
        let mut it = null_mut();
        assert_eq!(git_config_iterator_new(&mut it, cfg), 0);
        assert_eq!(git_config_next(&mut e, it), -1);
        assert_eq!(
            last_message(),
            "the config backend does not implement 'iterator'"
        );
        git_config_iterator_free(it);

        // A level shares the backend: freed once, with the last config.
        let mut level = null_mut();
        assert_eq!(
            git_config_open_level(&mut level, cfg, GIT_CONFIG_LEVEL_APP),
            0
        );
        let mut other = null_mut();
        assert_eq!(
            git_config_open_level(&mut other, cfg, GIT_CONFIG_LEVEL_GLOBAL),
            ErrorCode::NOT_FOUND.raw()
        );
        assert_eq!(
            last_message(),
            "no configuration exists for the given level '4'"
        );
        git_config_free(cfg);
        assert_eq!(frees, 0);
        let mut e = null_mut();
        assert_eq!(git_config_get_entry(&mut e, level, c"c.v".as_ptr()), 0);
        git_config_entry_free(e);
        git_config_free(level);
        assert_eq!(frees, 1);
    }
}

#[test]
fn maps_and_parsers() {
    let _g = test_support::init();
    unsafe {
        let maps = [
            git_configmap {
                type_: 0,
                str_match: core::ptr::null(),
                map_value: 10,
            },
            git_configmap {
                type_: 3,
                str_match: c"auto".as_ptr(),
                map_value: 11,
            },
            git_configmap {
                type_: 2,
                str_match: core::ptr::null(),
                map_value: 0,
            },
        ];
        let mut out = 0;
        assert_eq!(
            git_config_lookup_map_value(&mut out, maps.as_ptr(), 3, c"AUTO".as_ptr()),
            0
        );
        assert_eq!(out, 11);
        assert_eq!(
            git_config_lookup_map_value(&mut out, maps.as_ptr(), 3, c"off".as_ptr()),
            0
        );
        assert_eq!(out, 10);
        assert_eq!(
            git_config_lookup_map_value(&mut out, maps.as_ptr(), 3, c"33".as_ptr()),
            0
        );
        assert_eq!(out, 33);
        assert_eq!(
            git_config_lookup_map_value(&mut out, maps.as_ptr(), 3, core::ptr::null()),
            -1
        );
        assert_eq!(last_message(), "failed to map '(null)'");
        let mut t = 0;
        let mut s = core::ptr::null();
        assert_eq!(
            git_config_lookup_map_enum(&mut t, &mut s, maps.as_ptr(), 3, 11),
            0
        );
        assert_eq!((t, cs(s)), (3, "auto"));
        assert_eq!(
            git_config_lookup_map_enum(&mut t, &mut s, maps.as_ptr(), 3, 99),
            ErrorCode::NOT_FOUND.raw()
        );

        let mut b = 0;
        assert_eq!(git_config_parse_bool(&mut b, core::ptr::null()), 0);
        assert_eq!(b, 1);
        let mut i = 0i64;
        assert_eq!(git_config_parse_int64(&mut i, c"-2m".as_ptr()), 0);
        assert_eq!(i, -2 << 20);
        assert_eq!(git_config_parse_int64(&mut i, c"1kz".as_ptr()), -1);
        let mut n = null_mut();
        assert_eq!(git_config__normalize_name(c"A.b.C".as_ptr(), &mut n), 0);
        assert_eq!(cs(n), "a.b.c");
        libgit2_rs::__test::util::alloc::free(n.cast());
    }
}

#[test]
fn backend_foreach() {
    let _g = test_support::init();
    unsafe {
        let text = b"[a]\nb=1\nc=2\n";
        let mut b: *mut git_config_backend = null_mut();
        assert_eq!(
            git_config_backend_from_string(&mut b, text.as_ptr().cast(), text.len(), null_mut()),
            0
        );
        assert_eq!(((*b).open.unwrap())(b, 3, core::ptr::null()), 0);
        let mut seen: Vec<String> = Vec::new();
        assert_eq!(
            git_config_backend_foreach_match(
                b,
                core::ptr::null(),
                Some(count_cb),
                (&raw mut seen).cast()
            ),
            0
        );
        assert_eq!(seen, ["a.b", "a.c"]);
        // The memory backend refuses writes.
        assert_eq!(((*b).set.unwrap())(b, c"a.b".as_ptr(), c"x".as_ptr()), -1);
        assert_eq!(last_message(), "this backend is read-only");
        ((*b).free.unwrap())(b);
    }
}

// A C backend whose iterator, from inside its `next`, frees the outer
// (config) iterator that is calling it: the release of a table whose slot
// is running, which must wait until the slot returns.
static OUTER: std::sync::atomic::AtomicPtr<git_config_iterator> =
    std::sync::atomic::AtomicPtr::new(null_mut());
static INNER_FREES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[repr(C)]
struct ReIter {
    parent: git_config_iterator,
    entry: Box<CEntry>,
    done: bool,
}

unsafe extern "C" fn re_next(
    out: *mut *mut git_config_backend_entry,
    it: *mut git_config_iterator,
) -> c_int {
    unsafe {
        let it = it.cast::<ReIter>();
        if (*it).done {
            return ErrorCode::ITER_OVER.raw();
        }
        (*it).done = true;
        // Free the iterator that is running us.
        let outer = OUTER.swap(null_mut(), std::sync::atomic::Ordering::SeqCst);
        git_config_iterator_free(outer);
        out.write((&raw mut (*it).entry.base).cast());
        0
    }
}

unsafe extern "C" fn re_iter_free(it: *mut git_config_iterator) {
    drop(unsafe { Box::from_raw(it.cast::<ReIter>()) });
    INNER_FREES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

unsafe extern "C" fn noop_entry_free(_: *mut git_config_backend_entry) {}

unsafe extern "C" fn re_iterator(
    out: *mut *mut git_config_iterator,
    _: *mut git_config_backend,
) -> c_int {
    let it = Box::new(ReIter {
        parent: git_config_iterator {
            next: Some(re_next),
            free: Some(re_iter_free),
            ..Default::default()
        },
        entry: Box::new(CEntry {
            base: git_config_backend_entry {
                entry: git_config_entry {
                    name: c"re.x".as_ptr(),
                    value: c"1".as_ptr(),
                    backend_type: c"c".as_ptr(),
                    origin_path: core::ptr::null(),
                    include_depth: 0,
                    level: GIT_CONFIG_LEVEL_APP,
                },
                free: Some(noop_entry_free),
            },
        }),
        done: false,
    });
    unsafe { out.write(Box::into_raw(it).cast()) };
    0
}

#[test]
fn iterator_freed_inside_its_own_slot() {
    let _g = test_support::init();
    unsafe {
        let mut cfg = null_mut();
        assert_eq!(git_config_new(&mut cfg), 0);
        let mut frees = 0;
        let b = Box::into_raw(Box::new(CBackend {
            parent: git_config_backend {
                open: Some(c_open),
                get: Some(c_get),
                iterator: Some(re_iterator),
                free: Some(c_free),
                ..Default::default()
            },
            frees: &mut frees,
        }));
        assert_eq!(
            git_config_add_backend(cfg, b.cast(), GIT_CONFIG_LEVEL_APP, core::ptr::null(), 0),
            0
        );
        let before = INNER_FREES.load(std::sync::atomic::Ordering::SeqCst);
        let mut it = null_mut();
        assert_eq!(git_config_iterator_new(&mut it, cfg), 0);
        OUTER.store(it, std::sync::atomic::Ordering::SeqCst);
        let mut e = null_mut();
        // The outer `next` runs, the inner one frees the outer iterator:
        // the release waits for the slot, which still returns the entry.
        assert_eq!(git_config_next(&mut e, it), 0);
        // The outer iterator is gone now (released when its slot ended),
        // and with it the inner one; the entry was the inner iterator's.
        assert_eq!(
            INNER_FREES.load(std::sync::atomic::Ordering::SeqCst),
            before + 1
        );
        git_config_free(cfg);
        assert_eq!(frees, 1);
    }
}

#[test]
fn entries_outlive_their_config_and_backend_list() {
    let _g = test_support::init();
    unsafe {
        let cfg = memory_config(b"[a]\nb = one\n", GIT_CONFIG_LEVEL_LOCAL);
        let mut e: *mut git_config_entry = null_mut();
        assert_eq!(git_config_get_entry(&mut e, cfg, c"a.b".as_ptr()), 0);
        // Another thread replaces the backend at that level (force) while
        // this one holds the entry, then the config is freed.
        struct SendPtr(*mut GitConfig);
        // SAFETY: a config may be used from another thread (it is `Sync`).
        unsafe impl Send for SendPtr {}
        let p = SendPtr(cfg);
        std::thread::spawn(move || {
            let p = p;
            let cfg = p.0;
            let text = b"[a]\nb = two\n";
            let mut b = null_mut();
            assert_eq!(
                git_config_backend_from_string(
                    &mut b,
                    text.as_ptr().cast(),
                    text.len(),
                    null_mut()
                ),
                0
            );
            assert_eq!(
                git_config_add_backend(cfg, b, GIT_CONFIG_LEVEL_LOCAL, core::ptr::null(), 1),
                0
            );
        })
        .join()
        .unwrap();
        let mut e2: *mut git_config_entry = null_mut();
        assert_eq!(git_config_get_entry(&mut e2, cfg, c"a.b".as_ptr()), 0);
        assert_eq!(cs((*e2).value), "two");
        git_config_free(cfg);
        // Both entries still read their backends' lists.
        assert_eq!(cs((*e).value), "one");
        assert_eq!(cs((*e).name), "a.b");
        assert_eq!(cs((*e2).value), "two");
        git_config_entry_free(e);
        git_config_entry_free(e2);
    }
}
