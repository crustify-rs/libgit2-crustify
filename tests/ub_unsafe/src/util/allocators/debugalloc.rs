//! UB tests of `Allocator::debug` (`src/util/allocators/debugalloc.c`) installed through the
//! unsafe `set_allocator`, within its contract: libgit2 then asks the family for the sizes it
//! reads from a repository, which the value's guards refuse where C would wrap them.
//!
//! Installing an allocator is process-global and must precede libgit2's initialisation, so
//! each test reruns alone in a child process.

use std::ffi::CString;
use std::fs;
use std::process::Command;

use libgit2::libgit2::src::libgit2::odb::Odb;
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::Allocator;
use libgit2::libgit2::src::util::errors::ErrorClass;

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2 and no
/// other thread calls it.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

/// A loose object in the old pack-like format: a blob whose header names `size` bytes, in
/// 4 low bits and then 7-bit groups, followed by one byte of body.
fn packlike_object(size: u64) -> Vec<u8> {
    let low = u8::try_from(size & 0xF).unwrap();
    let mut rest = size >> 4;
    let mut object = vec![(3 << 4) | low | if rest == 0 { 0 } else { 0x80 }];
    while rest != 0 {
        let group = u8::try_from(rest & 0x7F).unwrap();
        rest >>= 7;
        object.push(group | if rest == 0 { 0 } else { 0x80 });
    }
    object.push(b'x');
    object
}

/// The regression test of `debug-size-obligation-unenforceable` (review wave-183/batch-1):
/// a header naming `2^64 - 16` bytes has `git_str_init` ask for `2^64 - 8`, which C's
/// `debugalloc__realloc` wrapped to a 0-byte block and wrote its length word past. The
/// guard refuses it, so the read fails as out of memory, as under the standard allocator.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn installed_debug_refuses_a_wrapping_object_size() {
    isolated(
        concat!(
            module_path!(),
            "::installed_debug_refuses_a_wrapping_object_size"
        ),
        || {
            let debug = Allocator::debug();
            // SAFETY: libgit2 is not initialised and holds no allocation, so every
            // allocation it makes is this family's, which meets `Alloc`'s contract; this
            // child runs one test on one thread.
            unsafe { set_allocator(Some(debug.as_ref())) };

            let root = std::env::temp_dir().join(format!(
                "crustify-ub-unsafe-debugalloc-{}",
                std::process::id()
            ));
            let objects = root.join("objects");
            let fanout = objects.join(&HEX[..2]);
            fs::create_dir_all(&fanout).unwrap();
            let odb = Odb::open(&CString::new(objects.to_str().unwrap()).unwrap()).unwrap();
            let id = Oid::from_string(&CString::new(HEX).unwrap(), OidType::Sha1).unwrap();
            // Sizes whose request rounds to within a word of `usize::MAX`, and the largest
            // a header holds; each read fails as out of memory, before any block is written.
            for size in [u64::MAX - 15, u64::MAX - 8, u64::MAX - 1, 1 << 63] {
                fs::write(fanout.join(&HEX[2..]), packlike_object(size)).unwrap();
                let err = odb.as_ref().read(id.as_ref()).unwrap_err();
                assert_eq!(
                    err.class(),
                    Some(ErrorClass::NoMemory),
                    "{size:#x}: {err:?}"
                );
            }
            // A size the family serves still reads, up to the body's end.
            fs::write(fanout.join(&HEX[2..]), packlike_object(1)).unwrap();
            let read = odb.as_ref().read(id.as_ref());
            drop(odb);
            fs::remove_dir_all(&root).unwrap();
            // The one-byte body is not zlib data, so the inflate fails; the allocation and
            // the read of the header did not.
            if let Err(err) = read {
                assert_ne!(err.class(), Some(ErrorClass::NoMemory), "{err:?}");
            }
        },
    );
}
