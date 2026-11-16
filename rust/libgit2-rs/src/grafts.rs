//! Port of `src/libgit2/grafts.c`: commit grafts (`info/grafts`) and the
//! shallow roots (`shallow`): a map from a commit id to the parents that
//! replace its own, read from a file and refreshed when it changes.
//!
//! # Types
//!
//! - [`GitCommitGraft`] is `git_commit_graft` (kind 3, mirrored): C
//!   (`commit.c`, the tests) reads `graft->oid` and `graft->parents` (a
//!   `git_array_oid_t`, [`GitArray<Oid>`]). Grafts live in `git__allocator`
//!   blocks the map owns; [`ffi::git_grafts_get`] lends them to C.
//! - [`Grafts`] is `git_grafts` (kind 1, opaque: `grafts.h` has only the
//!   typedef): the map is a [`GitHashMap`] keyed by the graft's own oid
//!   (`git_grafts_oidmap`'s khash), so [`Grafts::oids`] gives the C's order
//!   (the shallow roots' order).
//!
//! Parsing ([`Grafts::parse`]) is safe Rust over [`GitParseCtx`], with the
//! C's messages.

use core::ffi::{CStr, c_char, c_int};
use core::ptr::{self, NonNull};

use crate::hashmap_oid::OidPtr;
use crate::oid::{Oid, OidType};
use crate::parse::GitParseCtx;
use crate::util::alloc;
use crate::util::array::GitArray;
use crate::util::errors::{ErrorClass, ErrorCode, Result};
use crate::util::hash::SHA256_SIZE;
use crate::util::hashmap::GitHashMap;
use crate::util::str::GitStr;
use crate::{git_assert_arg, git_error_set};

/// `git_commit_graft`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitCommitGraft {
    /// The grafted commit.
    pub oid: Oid,
    /// Its replacement parents.
    pub parents: GitArray<Oid>,
}

/// Allocate `v` in a `git__allocator` block.
fn boxed<T>(v: T) -> Result<NonNull<T>> {
    let p = NonNull::new(alloc::malloc(size_of::<T>()).cast::<T>()).ok_or(ErrorCode::GENERIC)?;
    debug_assert!(align_of::<T>() <= 8);
    // SAFETY: a fresh block of `T`'s size, 8-aligned (`git__allocator`'s
    // memory), enough for `T`.
    unsafe { p.as_ptr().write(v) };
    Ok(p)
}

/// Drop and free a block of [`boxed`].
///
/// # Safety
///
/// `p` came from [`boxed`] and is not used again.
unsafe fn unboxed<T>(p: NonNull<T>) {
    // SAFETY: the caller's contract.
    unsafe {
        ptr::drop_in_place(p.as_ptr());
    }
    // SAFETY: as above.
    unsafe { alloc::free(p.as_ptr().cast()) };
}

/// `git_grafts`: the grafts of a repository, from a file or in memory.
pub struct Grafts {
    /// Grafts by their oid; each key points at its value's `oid`, which
    /// lives (unmoved, unwritten) as long as the entry.
    commits: GitHashMap<OidPtr, NonNull<GitCommitGraft>>,
    oid_type: OidType,
    /// The backing file (NULL for in-memory grafts).
    path: Option<std::ffi::CString>,
    path_checksum: [u8; SHA256_SIZE],
}

// SAFETY: the grafts own every graft in the map, plain data in
// `git__allocator` blocks, freed from any thread.
unsafe impl Send for Grafts {}
// SAFETY: shared access only reads.
unsafe impl Sync for Grafts {}

impl Drop for Grafts {
    fn drop(&mut self) {
        self.clear();
    }
}

impl core::fmt::Debug for Grafts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Grafts")
            .field("count", &self.len())
            .field("path", &self.path)
            .finish()
    }
}

impl Grafts {
    /// `git_grafts_new`: in-memory grafts.
    pub fn new(oid_type: OidType) -> Grafts {
        Grafts {
            commits: GitHashMap::new(),
            oid_type,
            path: None,
            path_checksum: [0; SHA256_SIZE],
        }
    }

    /// `git_grafts_open`: the grafts of the file at `path` (none if it does
    /// not exist).
    pub fn open(path: &CStr, oid_type: OidType) -> Result<Grafts> {
        let mut g = Grafts::new(oid_type);
        g.path = Some(path.to_owned());
        g.refresh()?;
        Ok(g)
    }

    /// `git_grafts_clear`.
    pub fn clear(&mut self) {
        for g in self.commits.values() {
            // SAFETY: each graft is the map's, freed once (the map is
            // cleared at once, so no key outlives its graft).
            unsafe { unboxed(*g) };
        }
        self.commits.clear();
    }

    /// `git_grafts_refresh`: re-read the file if it changed (cleared if it
    /// is gone).
    pub fn refresh(&mut self) -> Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let mut contents = GitStr::new();
        let mut updated: c_int = 0;
        match crate::util::futils::readbuffer_updated(
            &mut contents,
            path,
            Some(&mut self.path_checksum),
            Some(&mut updated),
        ) {
            Err(e) if e == ErrorCode::NOT_FOUND => {
                self.clear();
                return Ok(());
            }
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        if updated == 0 {
            return Ok(());
        }
        self.parse(contents.as_bytes())
    }

    /// `git_grafts_parse`: replace the grafts with those of `buf` (lines of
    /// `<oid>( <parent oid>)*`).
    pub fn parse(&mut self, buf: &[u8]) -> Result<()> {
        self.clear();
        let mut parser = GitParseCtx::new(buf);
        while parser.remain_len() > 0 {
            let Ok(graft_oid) = parser.advance_oid(self.oid_type) else {
                return Err(git_error_set!(
                    ErrorClass::GRAFTS,
                    "invalid graft OID at line {}",
                    parser.line_num()
                ));
            };
            let mut parents: GitArray<Oid> = GitArray::new();
            while parser.line_len() > 0 && parser.advance_expected(b"\n").is_err() {
                let r = parser
                    .advance_expected(b" ")
                    .and_then(|()| parser.advance_oid(self.oid_type));
                match r {
                    Ok(id) => {
                        parents.push(id)?;
                    }
                    Err(_) => {
                        return Err(git_error_set!(
                            ErrorClass::GRAFTS,
                            "invalid parent OID at line {}",
                            parser.line_num()
                        ));
                    }
                }
            }
            self.add(&graft_oid, parents.as_slice())?;
            parser.advance_line();
        }
        Ok(())
    }

    /// `git_grafts_add`: graft `parents` onto `oid` (replacing a graft of
    /// it).
    pub fn add(&mut self, oid: &Oid, parents: &[Oid]) -> Result<()> {
        let mut p = GitArray::with_capacity(parents.len())?;
        for id in parents {
            p.push(*id)?;
        }
        let graft = boxed(GitCommitGraft {
            oid: *oid,
            parents: p,
        })?;
        match self.remove(oid) {
            Ok(()) => {}
            Err(e) if e == ErrorCode::NOT_FOUND => {}
            Err(e) => {
                // SAFETY: the new graft, not in the map.
                unsafe { unboxed(graft) };
                return Err(e);
            }
        }
        // SAFETY: the key points at the graft's own oid, which lives in
        // its block (never moved or written) until the entry is removed.
        let key = unsafe { OidPtr::new(NonNull::from(&(*graft.as_ptr()).oid)) };
        if let Err(e) = self.commits.put(key, graft) {
            // SAFETY: the new graft, not in the map.
            unsafe { unboxed(graft) };
            return Err(e);
        }
        Ok(())
    }

    /// `git_grafts_remove`: `GIT_ENOTFOUND` (no message) if `oid` has no
    /// graft.
    pub fn remove(&mut self, oid: &Oid) -> Result<()> {
        // SAFETY: a key for the lookup only, alive during it.
        let key = unsafe { OidPtr::new(NonNull::from(oid)) };
        match self.commits.remove(&key) {
            Some((_, g)) => {
                // SAFETY: the removed graft, freed once.
                unsafe { unboxed(g) };
                Ok(())
            }
            None => Err(ErrorCode::NOT_FOUND),
        }
    }

    /// `git_grafts_get`.
    pub fn get(&self, oid: &Oid) -> Option<&GitCommitGraft> {
        // SAFETY: a key for the lookup only.
        let key = unsafe { OidPtr::new(NonNull::from(oid)) };
        // SAFETY: the map's graft, alive while `&self` is.
        self.commits.get(&key).map(|g| unsafe { g.as_ref() })
    }

    /// The parents grafted onto `oid`.
    pub fn parents(&self, oid: &Oid) -> Option<&[Oid]> {
        self.get(oid).map(|g| g.parents.as_slice())
    }

    /// `git_grafts_oids`: the grafted ids, in the map's order.
    pub fn oids(&self) -> Result<GitArray<Oid>> {
        let n = self.len();
        let mut a = if n > 0 { GitArray::with_capacity(n)? } else { GitArray::new() };
        for k in self.commits.keys() {
            a.push(*k.get())?;
        }
        Ok(a)
    }

    /// `git_grafts_size`.
    pub fn len(&self) -> usize {
        self.commits.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The exports of `grafts.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, bytes, status};

    /// `git_grafts_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_new(out: *mut *mut Grafts, oid_type: crate::oid::git_oid_t) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null() && oid_type != 0, "out && oid_type", -1);
            // (An unknown non-zero type: C keeps it, and every parse fails;
            // here it parses as SHA-1.)
            let t = OidType::from_raw(oid_type).unwrap_or(OidType::Sha1);
            match boxed(Grafts::new(t)) {
                Ok(g) => {
                    // SAFETY: writable.
                    unsafe { out.write(g.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_grafts_open`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_open(out: *mut *mut Grafts, path: *const c_char, oid_type: crate::oid::git_oid_t) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !path.is_null() && oid_type != 0,
                "out && path && oid_type",
                -1
            );
            let t = OidType::from_raw(oid_type).unwrap_or(OidType::Sha1);
            // SAFETY: NUL-terminated.
            let path = unsafe { CStr::from_ptr(path) };
            match Grafts::open(path, t).and_then(boxed) {
                Ok(g) => {
                    // SAFETY: writable.
                    unsafe { out.write(g.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_grafts_open_or_refresh`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a writable pointer to NULL or grafts; `path` NULL
    /// or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_open_or_refresh(
        out: *mut *mut Grafts,
        path: *const c_char,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !path.is_null() && oid_type != 0,
                "out && path && oid_type",
                -1
            );
            // SAFETY: readable.
            let g = unsafe { *out };
            if g.is_null() {
                // SAFETY: forwarded.
                unsafe { git_grafts_open(out, path, oid_type) }
            } else {
                // SAFETY: live grafts.
                unsafe { git_grafts_refresh(g) }
            }
        })
    }

    /// `git_grafts_free` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `grafts` is NULL or grafts of [`git_grafts_new`] / [`git_grafts_open`],
    /// not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_free(grafts: *mut Grafts) {
        barrier(|| {
            if let Some(g) = NonNull::new(grafts) {
                // SAFETY: the caller's contract.
                unsafe { unboxed(g) };
            }
        })
    }

    /// Borrow C's grafts for the call.
    ///
    /// # Safety
    ///
    /// `g` is NULL or live grafts, not otherwise accessed during the call.
    unsafe fn with<'a>(g: *mut Grafts) -> Option<&'a mut Grafts> {
        // SAFETY: the caller's contract.
        unsafe { g.as_mut() }
    }

    /// `git_grafts_clear` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `grafts` is NULL or live grafts, not otherwise accessed during the
    /// call (grafts C got from [`git_grafts_get`] are freed).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_clear(grafts: *mut Grafts) {
        barrier(|| {
            // SAFETY: the caller's contract.
            if let Some(g) = unsafe { with(grafts) } {
                g.clear();
            }
        })
    }

    /// `git_grafts_refresh`.
    ///
    /// # Safety
    ///
    /// As [`git_grafts_clear`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_refresh(grafts: *mut Grafts) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let Some(g) = (unsafe { with(grafts) }) else {
                return crate::util::errors::assert_arg_failed("grafts").raw();
            };
            status(g.refresh())
        })
    }

    /// `git_grafts_parse`.
    ///
    /// # Safety
    ///
    /// As [`git_grafts_clear`] (non-NULL); `buf` readable for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_parse(grafts: *mut Grafts, buf: *const c_char, len: usize) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract (C does not check for NULL).
            let Some(g) = (unsafe { with(grafts) }) else {
                return crate::util::errors::assert_arg_failed("grafts").raw();
            };
            // SAFETY: readable for `len` bytes.
            status(g.parse(unsafe { bytes(buf, len) }))
        })
    }

    /// `git_grafts_add` (`parents` is C's by value: not freed).
    ///
    /// # Safety
    ///
    /// As [`git_grafts_clear`]; `oid` NULL or valid; `parents` a valid
    /// `git_array_oid_t`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_add(
        grafts: *mut Grafts,
        oid: *const Oid,
        parents: GitArray<Oid>,
    ) -> c_int {
        let parents = core::mem::ManuallyDrop::new(parents);
        barrier(|| {
            git_assert_arg!(!grafts.is_null() && !oid.is_null(), "grafts && oid", -1);
            // SAFETY: the caller's contract.
            let (g, oid) = unsafe { (&mut *grafts, oid.read()) };
            status(g.add(&oid, parents.as_slice()))
        })
    }

    /// `git_grafts_remove`.
    ///
    /// # Safety
    ///
    /// As [`git_grafts_add`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_remove(grafts: *mut Grafts, oid: *const Oid) -> c_int {
        barrier(|| {
            git_assert_arg!(!grafts.is_null() && !oid.is_null(), "grafts && oid", -1);
            // SAFETY: the caller's contract.
            let (g, oid) = unsafe { (&mut *grafts, oid.read()) };
            status(g.remove(&oid))
        })
    }

    /// `git_grafts_get`: the graft of `oid` (valid until the grafts change),
    /// or `GIT_ENOTFOUND` (no message, `*out` untouched).
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `grafts` NULL or live grafts not modified
    /// during the call; `oid` NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_get(
        out: *mut *mut GitCommitGraft,
        grafts: *mut Grafts,
        oid: *const Oid,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !grafts.is_null() && !oid.is_null(),
                "out && grafts && oid",
                -1
            );
            // SAFETY: the caller's contract.
            let (g, oid) = unsafe { (&*grafts, &*oid) };
            match g.get(oid) {
                Some(graft) => {
                    // SAFETY: writable; the graft's block, which C reads.
                    unsafe { out.write(ptr::from_ref(graft).cast_mut()) };
                    0
                }
                None => ErrorCode::NOT_FOUND.raw(),
            }
        })
    }

    /// `git_grafts_oids`: the grafted ids, in a `git__allocator` array.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `out_len` writable; `grafts` NULL or live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_oids(
        out: *mut *mut Oid,
        out_len: *mut usize,
        grafts: *mut Grafts,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null() && !grafts.is_null(), "out && grafts", -1);
            // SAFETY: live grafts.
            match unsafe { &*grafts }.oids() {
                Ok(a) => {
                    let (p, n, _) = a.into_raw_parts();
                    // SAFETY: writable.
                    unsafe {
                        out.write(p);
                    }
                    // SAFETY: writable.
                    unsafe { out_len.write(n) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_grafts_size` (0 for NULL; C crashes).
    ///
    /// # Safety
    ///
    /// `grafts` is NULL or live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_size(grafts: *mut Grafts) -> usize {
        // SAFETY: NULL or live.
        unsafe { grafts.as_ref() }.map_or(0, Grafts::len)
    }
}

crate::c_layout_test! {
    headers: ["grafts.h"],
    types: [
        GitCommitGraft => "git_commit_graft" { oid, parents },
        GitArray<Oid> => "git_array_oid_t" {},
    ],
    asserts: [],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::errors;
    use crate::util::test_support;

    fn id(s: &str) -> Oid {
        Oid::from_str_hex(s, OidType::Sha1).unwrap()
    }

    const A: &str = "d7224d49d6d5aff6ade596ed74f4bcd4f77b29e2";
    const B: &str = "8cda5e8e1d9d2e10e1e10c3d9a2a3a4d5e6f7081";
    const C: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn parses_grafts() {
        let _g = test_support::init();
        let mut g = Grafts::new(OidType::Sha1);
        g.parse(format!("{A}\n{B} {A} {C}\n").as_bytes()).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g.parents(&id(A)).unwrap(), &[]);
        assert_eq!(g.parents(&id(B)).unwrap(), &[id(A), id(C)]);
        assert_eq!(g.oids().unwrap().len(), 2);
        g.add(&id(A), &[id(B)]).unwrap();
        assert_eq!(g.parents(&id(A)).unwrap(), &[id(B)]);
        g.remove(&id(A)).unwrap();
        assert_eq!(g.remove(&id(A)), Err(ErrorCode::NOT_FOUND));
        // Without a trailing newline.
        g.parse(A.as_bytes()).unwrap();
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn graft_errors() {
        let _g = test_support::init();
        let mut g = Grafts::new(OidType::Sha1);
        assert!(g.parse(b"xyz\n").is_err());
        assert_eq!(errors::last_message().unwrap(), b"invalid graft OID at line 1");
        assert!(g.parse(format!("{A}\n{B} zz\n").as_bytes()).is_err());
        assert_eq!(errors::last_message().unwrap(), b"invalid parent OID at line 2");
        assert!(g.parse(format!("{A}x\n").as_bytes()).is_err());
        assert_eq!(errors::last_message().unwrap(), b"invalid parent OID at line 1");
    }
}
