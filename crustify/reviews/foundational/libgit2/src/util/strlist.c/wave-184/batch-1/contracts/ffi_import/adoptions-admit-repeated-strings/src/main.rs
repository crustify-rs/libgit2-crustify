//! `StrList::from_raw_parts`'s `# Safety` asks for an array of libgit2's allocator "whose
//! first `len` elements are initialised, each NULL or a NUL-terminated string allocated by
//! the same allocator", and for the caller to "transfer unique ownership of the array and of
//! those strings". Nothing asks the elements to be distinct strings: an array that holds one
//! string twice honours every word of it, and the list's release, `git_strlist_free`, then
//! frees that string twice. `NullStrList::from_raw`'s obligation reads the same.
//!
//! Run under the `asan-ubsan` variant: AddressSanitizer reports the double free.

use std::ffi::c_char;

use libgit2::libgit2::src::util::alloc::AllocStr;
use libgit2::libgit2::src::util::strlist::{NullStrList, NullStrListFree, StrList, StrListFree};

/// Makes the second slot of `array` hold the first slot's string, releasing its own.
///
/// # Safety
///
/// `array` holds two live strings of libgit2's allocator, which the caller owns.
unsafe fn repeat_first(array: *mut *mut c_char) {
    // SAFETY: per this function's contract, the array holds two slots.
    let second = unsafe { array.add(1) };
    // SAFETY: as above, the slot holds a live string the caller owns, released once here.
    drop(unsafe { AllocStr::from_raw(second.read()) });
    // SAFETY: both slots are initialised and writable.
    unsafe { second.write(array.read()) };
}

fn main() {
    let null_terminated = std::env::args().nth(1).as_deref() == Some("null");
    if null_terminated {
        let array = NullStrListFree::copy(&[c"a", c"b"]).unwrap().into_raw();
        // SAFETY: a list's array, given up, of two strings.
        unsafe { repeat_first(array) };
        // SAFETY: as `from_raw`'s contract reads: a NULL-terminated array of libgit2's
        // allocator, aligned for a pointer, holding NUL-terminated strings of the same
        // allocator (one string, twice), whose unique ownership is transferred.
        let list = unsafe { NullStrList::from_raw(array) }.unwrap();
        println!("NullStrList::from_raw adopted {:?}", list);
        drop(list);
    } else {
        let (array, len) = StrListFree::copy(&[c"a", c"b"]).unwrap().into_raw_parts();
        // SAFETY: a list's array, given up, of two strings.
        unsafe { repeat_first(array) };
        // SAFETY: as `from_raw_parts`' contract reads: an array of libgit2's allocator,
        // aligned for a pointer, whose 2 elements are initialised, each a NUL-terminated
        // string of the same allocator (one string, twice), whose unique ownership is
        // transferred.
        let list = unsafe { StrList::from_raw_parts(array, len) }.unwrap();
        println!("StrList::from_raw_parts adopted {:?}", list);
        drop(list);
    }
    println!("released");
}
