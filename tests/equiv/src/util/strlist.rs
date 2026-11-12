//! Equivalence of `src/util/strlist.c`'s wrappers with its C routines, run on independently
//! owned inputs built from one description.
//!
//! - Copies: `StrListFree::copy` and `NullStrListFree::copy` against `git_strlist_copy` and
//!   `git_strlist_copy_with_null`, compared string by string, and a NULL-terminated array by its
//!   layout, through a sequence of replacements and clones; the C side releases its arrays with
//!   `git_strlist_free` and `git_strlist_free_with_null`.
//! - Searches: `contains_prefix` and `contains_key` against the C routines called on a raw `const
//!   char **` of the same strings, on a table of cases and after each step of a sequence that edits
//!   both lists alike. `contains_key` compares a key's name with its delimiter, where
//!   `git_strlist_contains_key` compares the name alone: it answers as C's
//!   `git_strlist_contains_prefix` of the name and delimiter, and as `git_strlist_contains_key`
//!   wherever no string's name begins with the key's and is longer.

use std::ffi::{CStr, CString, c_char};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::libgit2::init;
use libgit2::libgit2::src::util::alloc::{AllocStr, StrFree};
use libgit2::libgit2::src::util::strlist::{
    NullStrListFree, StrListFree, contains_key, contains_prefix,
};

/// A list C copied and owns: `len` strings of libgit2's allocator, released on drop.
struct CList {
    array: *mut *mut c_char,
    len: usize,
    null_terminated: bool,
}

impl CList {
    /// `git_strlist_copy` or `git_strlist_copy_with_null` of `strs`.
    fn copy(strs: &[&CStr], null_terminated: bool) -> Self {
        let ptrs: Vec<*const c_char> = strs.iter().map(|s| s.as_ptr()).collect();
        let mut array = ptr::null_mut();
        let copy = if null_terminated {
            ffi::git_strlist_copy_with_null
        } else {
            ffi::git_strlist_copy
        };
        // SAFETY: `ptrs.len()` live strings borrowed for the call; `array` is a writable slot.
        // libgit2 is initialised by the caller.
        let rc = unsafe { copy(&raw mut array, ptrs.as_ptr().cast_mut(), ptrs.len()) };
        assert_eq!(rc, 0);
        Self {
            array,
            len: strs.len(),
            null_terminated,
        }
    }

    /// The slot at `i`, which may be the terminator.
    fn slot(&self, i: usize) -> *mut *mut c_char {
        assert!(i < self.len + usize::from(self.null_terminated));
        // SAFETY: `i` is within the array's slots, checked above.
        unsafe { self.array.add(i) }
    }

    fn get(&self, i: usize) -> &CStr {
        assert!(i < self.len);
        // SAFETY: the slot is initialised.
        let string = unsafe { self.slot(i).read() };
        // SAFETY: the slot holds a live string the list owns, unchanged while borrowed.
        unsafe { CStr::from_ptr(string) }
    }

    fn strs(&self) -> Vec<&CStr> {
        (0..self.len).map(|i| self.get(i)).collect()
    }

    /// Replaces the string at `i` with a C copy of `s`, releasing the old one, as C code
    /// edits such an array.
    fn set(&mut self, i: usize, s: &CStr) {
        assert!(i < self.len);
        let slot = self.slot(i);
        // SAFETY: a live string borrowed for the call; libgit2 is initialised.
        let copy = unsafe { ffi::git__strdup(s.as_ptr()) };
        assert!(!copy.is_null());
        // SAFETY: the slot is initialised.
        let old = unsafe { slot.read() };
        // SAFETY: the slot held a live string the list owns, released once and replaced.
        unsafe { ffi::git__free(old.cast()) };
        // SAFETY: the slot is writable.
        unsafe { slot.write(copy) };
    }

    fn terminator(&self) -> *mut c_char {
        assert!(self.null_terminated);
        // SAFETY: the terminator's slot is initialised.
        unsafe { self.slot(self.len).read() }
    }

    fn contains_prefix(&self, prefix: &[u8]) -> bool {
        // SAFETY: the list's live strings; `prefix` holds `prefix.len()` readable bytes.
        unsafe {
            ffi::git_strlist_contains_prefix(
                self.array.cast(),
                self.len,
                prefix.as_ptr().cast(),
                prefix.len(),
            )
        }
    }

    fn contains_key(&self, key: &CStr, delimiter: u8) -> bool {
        // SAFETY: the list's live strings; `key` is NUL-terminated.
        unsafe {
            ffi::git_strlist_contains_key(
                self.array.cast(),
                self.len,
                key.as_ptr(),
                c_char::from_ne_bytes([delimiter]),
            )
        }
    }
}

impl Drop for CList {
    fn drop(&mut self) {
        if self.null_terminated {
            // SAFETY: the NULL-terminated array and strings this list owns, released once.
            unsafe { ffi::git_strlist_free_with_null(self.array) }
        } else {
            // SAFETY: the array and `len` strings this list owns, released once.
            unsafe { ffi::git_strlist_free(self.array, self.len) }
        }
    }
}

fn strs(list: &[AllocStr]) -> Vec<&CStr> {
    list.iter().map(AllocStr::as_c_str).collect()
}

/// `key`'s name and first `delimiter`, which `contains_key` searches for; `None` without one.
fn name_with_delimiter(key: &CStr, delimiter: u8) -> Option<&[u8]> {
    let key = key.to_bytes();
    key.iter()
        .position(|&b| b == delimiter)
        .map(|end| &key[..=end])
}

/// What `contains_key` answers on `c`'s strings, from the C: `git_strlist_contains_prefix` of
/// the name and delimiter, which `git_strlist_contains_key` answers too unless it finds the
/// name alone at the start of a longer one. Returns the answer and whether C's diverges.
fn key_in_c(c: &CList, key: &CStr, delimiter: u8) -> (bool, bool) {
    let c_key = c.contains_key(key, delimiter);
    let Some(prefix) = name_with_delimiter(key, delimiter) else {
        assert!(!c_key);
        return (false, false);
    };
    let found = c.contains_prefix(prefix);
    // C's answer covers the corrected one: it compares fewer bytes.
    assert!(c_key || !found, "{key:?}");
    (found, c_key != found)
}

/// The copies hold what C's do, after each replacement and clone, and a NULL-terminated
/// list's array ends with C's terminator.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_match_c_step_by_step() {
    let _runtime = init().unwrap();
    let start = [c"core.bare=false", c"user.name=me", c""];
    for null_terminated in [false, true] {
        let mut c = CList::copy(&start, null_terminated);
        let (mut rust_counted, mut rust_null) = (
            StrListFree::copy(&start).unwrap(),
            NullStrListFree::copy(&start).unwrap(),
        );
        let rust: &mut [AllocStr] = if null_terminated {
            &mut rust_null
        } else {
            &mut rust_counted
        };
        assert_eq!(strs(rust), c.strs());

        let edits = [(0, c"core.bare=true"), (2, c"x"), (1, c"")];
        for (i, s) in edits {
            c.set(i, s);
            rust[i] = StrFree::dup(s).unwrap();
            assert_eq!(strs(rust), c.strs());
        }
        if null_terminated {
            assert!(c.terminator().is_null());
            let array = rust_null.as_ptr();
            // SAFETY: the list's array holds its strings, then the terminator.
            let slot = unsafe { array.add(rust_null.len()) };
            // SAFETY: as above, the terminator's slot is initialised.
            let terminator = unsafe { slot.read() };
            assert!(terminator.is_null());
            let cloned = rust_null.clone();
            let c2 = CList::copy(&c.strs(), true);
            assert_eq!(strs(&cloned), c2.strs());
        } else {
            let cloned = rust_counted.clone();
            let c2 = CList::copy(&c.strs(), false);
            assert_eq!(strs(&cloned), c2.strs());
        }
    }

    // Empty copies: an array of nothing, or of the terminator alone.
    let none: [&CStr; 0] = [];
    let (c, c_null) = (CList::copy(&none, false), CList::copy(&none, true));
    assert!(!c.array.is_null() && c_null.terminator().is_null());
    let (rust, rust_null) = (
        StrListFree::copy(&none).unwrap(),
        NullStrListFree::copy(&none).unwrap(),
    );
    assert!(rust.is_empty() && !rust.as_ptr().is_null() && rust_null.is_empty());
    // SAFETY: the empty NULL-terminated list's array holds the terminator.
    assert!(unsafe { rust_null.as_ptr().read() }.is_null());
}

/// The searches answer as C's do, on a table of prefixes and keys.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn searches_match_c() {
    let _runtime = init().unwrap();
    let env = [
        c"HOME=/root",
        c"PATH=/bin:/usr/bin",
        c"FOOBAR=2",
        c"EMPTY=",
        c"NOEQ",
    ];
    let c = CList::copy(&env, false);
    let list = StrListFree::copy(&env).unwrap();
    let prefixes: [&[u8]; 9] = [
        b"HOME",
        b"HOME=/root",
        b"HOME=/root/",
        b"",
        b"PATH=/bin\0junk",
        b"PATH=/bin:/usr/bin\0junk",
        b"NOEQ\0",
        b"noeq",
        b"F",
    ];
    for prefix in prefixes {
        let want = c.contains_prefix(prefix);
        assert_eq!(contains_prefix(&env, prefix), want, "{prefix:?}");
        assert_eq!(contains_prefix(&list, prefix), want, "{prefix:?}");
    }
    let keys = [
        (c"HOME=/home/me", b'='),
        (c"FOO=1", b'='),
        (c"FOOBAR=3", b'='),
        (c"USER=me", b'='),
        (c"NOEQ", b'='),
        (c"=leading", b'='),
        (c"PATH:x", b':'),
        (c"PATH=/bin:x", b':'),
        (c"EMPTY=x", b'='),
        (c"HOME=/x", 0),
        (c"", b'='),
    ];
    let mut diverging = Vec::new();
    for (key, delimiter) in keys {
        let (want, diverges) = key_in_c(&c, key, delimiter);
        assert_eq!(contains_key(&env, key, delimiter), want, "{key:?}");
        assert_eq!(contains_key(&list, key, delimiter), want, "{key:?}");
        if diverges {
            diverging.push(key);
        }
    }
    // C's answers the wrapper corrects: the name is found at the start of a longer one
    // (`FOOBAR`, `PATH=/bin`), and an empty name before the delimiter in every string.
    assert_eq!(diverging, [c"FOO=1", c"=leading", c"PATH:x"]);
    assert!(c.contains_key(c"FOO=1", b'=') && !contains_key(&env, c"FOO=1", b'='));
}

/// The searches answer as C's do after each step of a sequence that edits both lists alike,
/// one of them reached through a NULL-terminated list's elements; `contains_key` as C's
/// corrected search does.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn searches_match_c_across_edits() {
    let _runtime = init().unwrap();
    let start = [c"a.b=1", c"c.d=2"];
    let mut c = CList::copy(&start, true);
    let mut rust = NullStrListFree::copy(&start).unwrap();
    let probe = |c: &CList, rust: &[AllocStr]| {
        for prefix in [&b"a."[..], b"c.d", b"e", b"a.b=1\0"] {
            assert_eq!(contains_prefix(rust, prefix), c.contains_prefix(prefix));
        }
        for key in [c"a.b=9", c"c.d=9", c"e.f=9", c"a=9"] {
            assert_eq!(contains_key(rust, key, b'='), key_in_c(c, key, b'=').0);
        }
    };
    probe(&c, &rust);
    for (i, s) in [(0, c"e.f=3"), (1, c"a=4"), (0, c"")] {
        c.set(i, s);
        rust[i] = StrFree::dup(s).unwrap();
        probe(&c, &rust);
    }
    let owned: Vec<CString> = c.strs().into_iter().map(CStr::to_owned).collect();
    assert_eq!(
        contains_key(&owned, c"a=0", b'='),
        key_in_c(&c, c"a=0", b'=').0
    );
}
