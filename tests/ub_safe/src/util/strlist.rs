//! UB tests of `src/util/strlist.c`'s lists and searches through the safe API alone.
//!
//! Each list owns a `git__calloc` array of `git__strdup` strings that its C releaser frees,
//! while its elements are `AllocStr` owners a consumer may replace, swap and sort: the tests
//! move strings between lists and the allocator's other owners, drop lists in every order,
//! clone them and send them across threads, so that `ASan` sees every release path and
//! `TSan` every thread a list crosses. The searches read borrowed runs of exactly the length they
//! are given, at the end of their allocations.

use std::ffi::{CStr, CString};
use std::thread;

use libgit2::libgit2::src::util::alloc::{AllocStr, StrFree};
use libgit2::libgit2::src::util::strlist::{
    NullStrList, NullStrListFree, StrList, StrListFree, contains_key, contains_prefix,
};

fn strs(list: &[AllocStr]) -> Vec<&CStr> {
    list.iter().map(AllocStr::as_c_str).collect()
}

/// Replacing, swapping and sorting elements moves owners of libgit2's strings in and out
/// of the lists; each releaser then frees exactly what its list holds.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn elements_move_between_lists_and_owners() {
    let mut list = StrListFree::copy(&[c"b", c"a", c"c"]).unwrap();
    let mut argv = NullStrListFree::copy(&[c"/bin/sh", c"-c"]).unwrap();

    list[1] = StrFree::dup(c"z").unwrap();
    std::mem::swap(&mut list[0], &mut argv[1]);
    list.sort();
    assert_eq!(strs(&list), [c"-c", c"c", c"z"]);
    assert_eq!(strs(&argv), [c"/bin/sh", c"b"]);

    // An element taken out by replacement outlives its list.
    let taken = std::mem::replace(&mut argv[0], StrFree::dup(c"/bin/true").unwrap());
    drop(argv);
    assert_eq!(taken.as_c_str(), c"/bin/sh");
    list.reverse();
    list[2] = taken;
    assert_eq!(strs(&list), [c"z", c"c", c"/bin/sh"]);
    drop(list);
}

/// A clone holds copies of its own: it outlives its source, and neither sees the other's
/// changes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn clones_are_independent_of_their_source() {
    let source = StrListFree::copy(&[c"x=1", c"y=2"]).unwrap();
    let mut copy = source.clone();
    copy[0] = StrFree::dup(c"x=3").unwrap();
    let again = copy.try_clone().unwrap();
    drop(source);
    assert_eq!(strs(&copy), [c"x=3", c"y=2"]);
    assert_eq!(strs(&again), [c"x=3", c"y=2"]);
    assert_ne!(copy[0].as_ptr(), again[0].as_ptr());

    let argv = NullStrListFree::copy(&[c"git", c"status"]).unwrap();
    let mut argv2 = argv.clone();
    argv2[1] = StrFree::dup(c"log").unwrap();
    let argv3 = argv2.try_clone().unwrap();
    drop((argv, argv2));
    assert_eq!(strs(&argv3), [c"git", c"log"]);
}

/// Empty lists, with and without an array, are read, cloned and released like any other.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_lists_clone_and_drop() {
    let none: [&CStr; 0] = [];
    let empty = StrListFree::copy(&none).unwrap();
    let bare = StrList::new();
    let terminator = NullStrListFree::copy(&none).unwrap();
    for list in [&empty, &bare, &empty.clone(), &bare.clone()] {
        assert!(list.is_empty() && list.iter().next().is_none());
        assert_eq!(format!("{list:?}"), "[]");
    }
    let terminator2 = terminator.clone();
    assert!(terminator.is_empty() && terminator2.is_empty());
    assert!(!contains_prefix(&empty, b""));
    drop((empty, bare, terminator, terminator2));
}

/// Lists are `Send` and `Sync`: they are cloned and searched from several threads at once,
/// and released on threads other than the one that copied them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lists_cross_threads() {
    fn shared<T: Send + Sync>(_: &T) {}
    let list = StrListFree::copy(&[c"HOME=/root", c"PATH=/bin"]).unwrap();
    let argv = NullStrListFree::copy(&[c"env"]).unwrap();
    shared(&list);
    shared(&argv);

    let (lists, argvs): (Vec<StrList>, Vec<NullStrList>) = thread::scope(|s| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    assert!(contains_key(&list, c"PATH=/usr/bin", b'='));
                    (list.clone(), argv.clone())
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).unzip()
    });
    let handles: Vec<_> = lists
        .into_iter()
        .zip(argvs)
        .map(|(mut l, a)| {
            thread::spawn(move || {
                l[0] = StrFree::dup(c"HOME=/home").unwrap();
                drop(a);
                strs(&l).iter().map(|s| s.to_bytes().len()).sum::<usize>()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), 19);
    }
    thread::spawn(move || drop((list, argv))).join().unwrap();
}

/// The searches read at most the prefix's length, even with no NUL after it: each prefix
/// here is a boxed slice whose allocation ends where it does.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn searches_read_only_their_arguments() {
    let owned: Vec<CString> = ["user.name=me", "user.email=me@x", "core.bare"]
        .into_iter()
        .map(|s| CString::new(s).unwrap())
        .collect();
    let list = StrListFree::copy(&owned).unwrap();
    for prefix in ["user.", "core.bare", "core.bare.long", "", "x"] {
        let exact: Box<[u8]> = prefix.as_bytes().into();
        let found = contains_prefix(&owned, &exact);
        assert_eq!(found, contains_prefix(&list, &exact), "{prefix}");
        assert_eq!(
            found,
            owned.iter().any(|s| s.to_bytes().starts_with(&exact)),
            "{prefix}"
        );
    }
    // Strings shorter than the prefix end the comparison at their terminator.
    let long: Box<[u8]> = vec![b'u'; 4096].into();
    assert!(!contains_prefix(&list, &long));

    assert!(contains_key(&list, c"user.name=you", b'='));
    assert!(!contains_key(&list, c"core.bare", b'='));
    assert!(!contains_key(&list, c"user.name=you", 0));
    let empty: [&CStr; 0] = [];
    assert!(!contains_key(&empty, c"a=b", b'='));
    assert!(!contains_prefix(&empty, b"a"));
}
