//! `Allocator::debug()` installed with `set_allocator` before libgit2 initialises, as its
//! documentation invites, then a loose object read from an `objects` directory.
//!
//! `Allocator::debug`'s members carry one obligation of their own: the size asked for is at
//! most `usize::MAX - size_of::<usize>()`, which `debugalloc.c` adds to unchecked. Once the
//! family is installed, its caller is libgit2, which takes sizes from the repository's
//! bytes. A loose object in the old pack-like format whose header names 2^64 - 16 bytes
//! makes `read_loose_packlike` ask `git_str_init` for 2^64 - 15, which rounds it to
//! 2^64 - 8 and calls `git__realloc(NULL, 2^64 - 8)`: `debugalloc__realloc` adds 8, wraps
//! to `malloc(0)`, and writes the length word past the empty block. No caller of
//! `set_allocator` can vouch that libgit2 never asks such a size.
//!
//! Run under the `asan-ubsan` variant: ASan reports a heap-buffer-overflow WRITE of size 8
//! in `debugalloc__realloc`.

use std::ffi::CString;
use std::fs;

use libgit2::libgit2::src::libgit2::odb::Odb;
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::Allocator;

const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

fn main() {
    let debug = Allocator::debug();
    // SAFETY: libgit2 is not initialised yet and holds no allocation, so every allocation it
    // makes is this family's; its storage is aligned to a word, which libgit2 and this crate
    // need at most; nothing in this program asks the family for a size itself; one thread.
    unsafe { set_allocator(Some(debug.as_ref())) };

    let root = std::env::temp_dir().join(format!("debugalloc-repro-{}", std::process::id()));
    let objects = root.join("objects");
    fs::create_dir_all(objects.join(&HEX[..2])).unwrap();
    // The pack-like header: a blob (type 3), its size in 7-bit groups after 4 low bits:
    // 0 in the low 4 bits, then every bit from 4 to 63 set, 0xffff_ffff_ffff_fff0.
    let mut object = vec![0x80 | (3 << 4)];
    object.extend([0xff; 8]);
    object.push(0x0f);
    object.extend(b"x");
    fs::write(objects.join(&HEX[..2]).join(&HEX[2..]), &object).unwrap();

    let dir = CString::new(objects.to_str().unwrap()).unwrap();
    let odb = Odb::open(&dir).unwrap();
    let id = Oid::from_string(&CString::new(HEX).unwrap(), OidType::Sha1).unwrap();
    let read = odb.as_ref().read(id.as_ref());
    let _ = fs::remove_dir_all(&root);
    // Under the standard allocator the read fails as out of memory; under this family C
    // has already written past its block.
    match read {
        Ok(_) => panic!("a 2^64-byte object was read"),
        Err(e) => println!("read failed: {e:?}"),
    }
}
