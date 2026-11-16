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

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::null_mut;

use libgit2_rs::config::ffi::*;
use libgit2_rs::config::{
    GIT_CONFIG_LEVEL_APP, GIT_CONFIG_LEVEL_GLOBAL, GIT_CONFIG_LEVEL_LOCAL, GitConfig,
    git_config_backend, git_config_backend_entry, git_config_entry, git_config_iterator,
    git_configmap,
};
use libgit2_rs::config_mem::ffi::git_config_backend_from_string;
use libgit2_rs::util::errors::{self, ErrorCode};
use libgit2_rs::util::test_support;

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
    _: *const libgit2_rs::sys::pending::git_repository,
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
        libgit2_rs::util::alloc::free(n.cast());
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
