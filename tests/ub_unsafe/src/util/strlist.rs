//! `src/util/strlist.c`: the lists' unsafe adoptions, `StrList::from_raw_parts` and
//! `NullStrList::from_raw`, called within their contracts, edge cases included. One suite
//! per function.
//!
//! The arrays they adopt are those libgit2 makes, handed over by the safe API's
//! `into_raw_parts` / `into_raw` or by the C copies themselves; ASan/LSan flag an adoption
//! that loses, leaks or double-frees a string or an array.

use core::ffi::{CStr, c_char};
use core::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::util::alloc::{AllocStr, StrFree};
use libgit2::libgit2::src::util::strlist::{NullStrList, NullStrListFree, StrList, StrListFree};

fn strs(list: &[AllocStr]) -> Vec<&CStr> {
    list.iter().map(AllocStr::as_c_str).collect()
}

/// Releases the string at `slot` and leaves NULL there, as C's arrays may hold.
///
/// # Safety
///
/// `slot` holds a live string of libgit2's allocator, which the caller gives up.
unsafe fn null_out(slot: *mut *mut c_char) {
    // SAFETY: per this function's contract, the slot is initialised.
    let string = unsafe { slot.read() };
    // SAFETY: per this function's contract, a live string of libgit2's allocator, released
    // once here.
    drop(unsafe { AllocStr::from_raw(string) });
    // SAFETY: the slot is writable, and no longer owns a string.
    unsafe { slot.write(ptr::null_mut()) };
}

mod str_list_from_raw_parts {
    use super::*;

    /// A list given up and adopted again keeps its strings, and its releaser frees them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopts_what_a_list_gave_up() {
        let mut list = StrListFree::copy(&[c"a=1", c"b=2"]).unwrap();
        list[0] = StrFree::dup(c"c=3").unwrap();
        let (array, len) = list.into_raw_parts();
        assert_eq!(len, 2);
        // SAFETY: the array and strings a list gave up, of libgit2's allocator, handed back.
        let back = unsafe { StrList::from_raw_parts(array, len) }.unwrap();
        assert_eq!(strs(&back), [c"c=3", c"b=2"]);
        assert_eq!(back.as_ptr(), array.cast_const().cast());
    }

    /// The array `git_strlist_copy` makes, adopted as C hands it out.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopts_what_git_strlist_copy_made() {
        let source = [c"x".as_ptr(), c"yz".as_ptr()];
        // The C copy needs libgit2's allocator, initialised by a first safe copy.
        drop(StrListFree::copy(&[c"init"]).unwrap());
        let mut out: *mut *mut c_char = ptr::null_mut();
        // SAFETY: two live strings borrowed for the call; `out` is a writable slot.
        let rc = unsafe { ffi::git_strlist_copy(&raw mut out, source.as_ptr().cast_mut(), 2) };
        assert_eq!(rc, 0);
        // SAFETY: `out` is the copy's array of two strings, given up to the list.
        let list = unsafe { StrList::from_raw_parts(out, 2) }.unwrap();
        assert_eq!(strs(&list), [c"x", c"yz"]);
    }

    /// An empty array, NULL or allocated, is the empty list; a count of strings that are not
    /// there, or a NULL string, is refused and leaves ownership with the caller.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refuses_what_a_list_never_holds() {
        // SAFETY: NULL with 0 is the empty list without an array.
        let bare = unsafe { StrList::from_raw_parts(ptr::null_mut(), 0) }.unwrap();
        assert!(bare.is_empty() && bare.as_ptr().is_null());
        // SAFETY: a NULL array with a count is refused before anything is read.
        assert!(unsafe { StrList::from_raw_parts(ptr::null_mut(), 3) }.is_none());

        let none: [&CStr; 0] = [];
        let (array, len) = StrListFree::copy(&none).unwrap().into_raw_parts();
        assert!(!array.is_null() && len == 0);
        // SAFETY: the zero-byte array an empty copy allocated, handed back.
        let empty = unsafe { StrList::from_raw_parts(array, 0) }.unwrap();
        assert!(empty.is_empty() && empty.iter().next().is_none());

        let (array, len) = StrListFree::copy(&[c"a", c"b", c"c"])
            .unwrap()
            .into_raw_parts();
        // SAFETY: the array holds three slots.
        let second = unsafe { array.add(1) };
        // SAFETY: as above.
        let third = unsafe { array.add(2) };
        // SAFETY: the second slot holds a live string of the list, given up.
        unsafe { null_out(second) };
        // SAFETY: an array of libgit2's allocator holding a string, NULL and a string; it is
        // refused, so the caller keeps it.
        assert!(unsafe { StrList::from_raw_parts(array, len) }.is_none());
        // The caller releases the last string and hands over the first alone, with a count
        // that stops before the NULL one.
        // SAFETY: the third slot holds a live string of libgit2's allocator, released once.
        unsafe { null_out(third) };
        // SAFETY: the array of libgit2's allocator, its first slot a live string of it.
        let first = unsafe { StrList::from_raw_parts(array, 1) }.unwrap();
        assert_eq!(strs(&first), [c"a"]);
    }
}

mod null_str_list_from_raw {
    use super::*;

    /// A list given up and adopted again keeps its strings up to the terminator.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopts_what_a_list_gave_up() {
        let mut argv = NullStrListFree::copy(&[c"/bin/sh", c"-c", c"true"]).unwrap();
        argv[2] = StrFree::dup(c"false").unwrap();
        let array = argv.into_raw();
        // SAFETY: the NULL-terminated array and strings a list gave up, handed back.
        let back = unsafe { NullStrList::from_raw(array) }.unwrap();
        assert_eq!(strs(&back), [c"/bin/sh", c"-c", c"false"]);
        assert_eq!(back.as_ptr(), array.cast_const().cast());
    }

    /// The array `git_strlist_copy_with_null` makes, adopted as C hands it out; an empty one
    /// holds the terminator alone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopts_what_git_strlist_copy_with_null_made() {
        drop(NullStrListFree::copy(&[c"init"]).unwrap());
        for strings in [&[c"HOME=/", c"TERM=dumb"][..], &[]] {
            let source: Vec<*const c_char> = strings.iter().map(|s| s.as_ptr()).collect();
            let mut out: *mut *mut c_char = ptr::null_mut();
            // SAFETY: live strings borrowed for the call (none read when empty); `out` is a
            // writable slot.
            let rc = unsafe {
                ffi::git_strlist_copy_with_null(
                    &raw mut out,
                    source.as_ptr().cast_mut(),
                    source.len(),
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: `out` is the copy's NULL-terminated array, given up to the list.
            let list = unsafe { NullStrList::from_raw(out) }.unwrap();
            assert_eq!(strs(&list), strings);
        }
    }

    /// A NULL array is refused; a NULL element ends the list there, and the strings after
    /// it stay the caller's.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn stops_at_the_first_null() {
        // SAFETY: a NULL array is refused before anything is read.
        assert!(unsafe { NullStrList::from_raw(ptr::null_mut()) }.is_none());

        let array = NullStrListFree::copy(&[c"a", c"b", c"c"])
            .unwrap()
            .into_raw();
        // SAFETY: the array holds four slots.
        let second = unsafe { array.add(1) };
        // SAFETY: as above.
        let third = unsafe { array.add(2) };
        // SAFETY: the second slot holds a live string of the list, given up.
        unsafe { null_out(second) };
        // SAFETY: the third slot is initialised; its string stays the caller's, since the
        // list ends before it.
        let third = unsafe { third.read() };
        // SAFETY: a NULL-terminated array of libgit2's allocator whose first string is
        // live, handed over.
        let list = unsafe { NullStrList::from_raw(array) }.unwrap();
        assert_eq!(strs(&list), [c"a"]);
        drop(list);
        // SAFETY: the string the list did not adopt, released once.
        drop(unsafe { AllocStr::from_raw(third) });
    }
}
