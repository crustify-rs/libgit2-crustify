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
    // `git__allocator` blocks are aligned as `malloc`'s (at least 8): a
    // compile-time check, not a `debug_assert!`, since a release build
    // would write misaligned otherwise.
    const { assert!(align_of::<T>() <= 8 && size_of::<T>() > 0) };
    let p = NonNull::new(alloc::malloc(size_of::<T>()).cast::<T>()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a fresh, non-null block of `size_of::<T>()` bytes, aligned
    // for `T` (checked above), that nothing else references.
    unsafe { p.as_ptr().write(v) };
    Ok(p)
}

/// Drop and free a block of [`boxed`].
///
/// # Safety
///
/// `p` came from [`boxed`], is not freed elsewhere, and is not used
/// again (no reference to it is live).
unsafe fn unboxed<T>(p: NonNull<T>) {
    // SAFETY: `p` holds the initialized `T` that `boxed` wrote, dropped
    // once (this function's contract: it is not used again).
    unsafe {
        ptr::drop_in_place(p.as_ptr());
    }
    // SAFETY: `p` is the `git__allocator` block `boxed` made, freed once.
    unsafe { alloc::free(p.as_ptr().cast()) };
}

/// `git_grafts`: the grafts of a repository, from a file or in memory.
pub struct Grafts {
    /// Grafts by their oid; each key points at its value's `oid`, which
    /// lives (unmoved, unwritten) as long as the entry.
    commits: GitHashMap<OidPtr, NonNull<GitCommitGraft>>,
    /// `None` for an unknown type C gave (every graft line then fails).
    oid_type: Option<OidType>,
    /// The backing file (NULL for in-memory grafts).
    path: Option<std::ffi::CString>,
    path_checksum: [u8; SHA256_SIZE],
}

// SAFETY: the fields that are not `Send` by themselves are the map's raw
// pointers: each value is a graft (an oid and a `git__allocator` array of
// oids, plain data) in a `git__allocator` block that the grafts own alone,
// and each key points at its own graft's oid. Moving the grafts moves that
// ownership; `git__allocator` frees from any thread.
unsafe impl Send for Grafts {}
// SAFETY: through `&Grafts` the grafts are only read (`get`, `oids`,
// `len`); every write (`add`, `remove`, `clear`, `refresh`, `parse`) takes
// `&mut`.
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
        Grafts::new_raw(Some(oid_type))
    }

    fn new_raw(oid_type: Option<OidType>) -> Grafts {
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
        Grafts::open_raw(path, Some(oid_type))
    }

    fn open_raw(path: &CStr, oid_type: Option<OidType>) -> Result<Grafts> {
        let mut g = Grafts::new_raw(oid_type);
        g.path = Some(path.to_owned());
        g.refresh()?;
        Ok(g)
    }

    /// `git_grafts_clear`.
    pub fn clear(&mut self) {
        for g in self.commits.values() {
            // SAFETY: each value is a block of `boxed` that the map owns,
            // freed once here; the map is cleared just after, so no key or
            // value is used again.
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
            // An unknown type has no hex size: C's `git_oid_from_prefix`
            // fails, as here.
            let graft_oid = self.oid_type.map(|t| parser.advance_oid(t));
            let Some(Ok(graft_oid)) = graft_oid else {
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
                    .and_then(|()| parser.advance_oid(self.oid_type.unwrap_or(OidType::Sha1)));
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
                // SAFETY: the block `boxed` just made, not in the map, freed
                // once.
                unsafe { unboxed(graft) };
                return Err(e);
            }
        }
        // The graft's own oid: its first field (`#[repr(C)]`, offset 0, as
        // the layout test checks).
        let oid_ptr = graft.cast::<Oid>();
        // SAFETY: the key points at the graft's own oid, which lives in
        // its block (never moved or written) until the entry is removed.
        let key = unsafe { OidPtr::new(oid_ptr) };
        if let Err(e) = self.commits.put(key, graft) {
            // SAFETY: as above: the failed `put` did not keep it.
            unsafe { unboxed(graft) };
            return Err(e);
        }
        Ok(())
    }

    /// `git_grafts_remove`: `GIT_ENOTFOUND` (no message) if `oid` has no
    /// graft.
    pub fn remove(&mut self, oid: &Oid) -> Result<()> {
        // SAFETY: a key for the lookup only, which does not outlive the
        // borrowed `oid`.
        let key = unsafe { OidPtr::new(NonNull::from(oid)) };
        match self.commits.remove(&key) {
            Some((_, g)) => {
                // SAFETY: the removed graft, a block of `boxed` that the map
                // no longer holds, freed once.
                unsafe { unboxed(g) };
                Ok(())
            }
            None => Err(ErrorCode::NOT_FOUND),
        }
    }

    /// `git_grafts_get`.
    pub fn get(&self, oid: &Oid) -> Option<&GitCommitGraft> {
        // SAFETY: a key for the lookup only, which does not outlive the
        // borrowed `oid`.
        let key = unsafe { OidPtr::new(NonNull::from(oid)) };
        // SAFETY: a graft of the map: a live block that is freed or written
        // only through `&mut self`, so not while the result borrows `self`.
        self.commits.get(&key).map(|g| unsafe { g.as_ref() })
    }

    /// The parents grafted onto `oid`.
    pub fn parents(&self, oid: &Oid) -> Option<&[Oid]> {
        self.get(oid).map(|g| g.parents.as_slice())
    }

    /// `git_grafts_oids`: the grafted ids, in the map's order.
    pub fn oids(&self) -> Result<GitArray<Oid>> {
        let n = self.len();
        let mut a = if n > 0 {
            GitArray::with_capacity(n)?
        } else {
            GitArray::new()
        };
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
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, bytes, status};

    /// `git_grafts_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_new(
        mut out: Out<'_, *mut Grafts>,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null() && oid_type != 0, "out && oid_type", -1);
            match boxed(Grafts::new_raw(OidType::from_raw(oid_type))) {
                Ok(g) => {
                    out.write(g.as_ptr());
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
    pub unsafe extern "C" fn git_grafts_open(
        mut out: Out<'_, *mut Grafts>,
        path: CStrArg<'_>,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !path.is_null() && oid_type != 0,
                "out && path && oid_type",
                -1
            );
            let path = path.or_empty();
            match Grafts::open_raw(path, OidType::from_raw(oid_type)).and_then(boxed) {
                Ok(g) => {
                    out.write(g.as_ptr());
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
        out: Option<&mut *mut Grafts>,
        path: CStrArg<'_>,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                out.is_some() && !path.is_null() && oid_type != 0,
                "out && path && oid_type",
                -1
            );
            let Some(out) = out else { return -1 };
            let g = *out;
            if g.is_null() {
                // SAFETY: `out` is writable (a `&mut`), `path` a string
                // (this export's contract): `git_grafts_open`'s contract.
                unsafe { git_grafts_open(Out::from(out), path, oid_type) }
            } else {
                // SAFETY: a non-null `*out` is live grafts that nothing
                // else uses during the call (this export's contract), which
                // is `git_grafts_refresh`'s.
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
                // SAFETY: a non-null `grafts` came from `git_grafts_new` /
                // `_open` (blocks of `boxed`) and is not used again (this
                // export's contract).
                unsafe { unboxed(g) };
            }
        })
    }

    /// `git_grafts_clear` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `grafts` is NULL or live grafts, not otherwise accessed during the
    /// call (grafts C got from [`git_grafts_get`] are freed).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_grafts_clear(grafts: Option<&mut Grafts>) {
        barrier(|| {
            if let Some(g) = grafts {
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
            // SAFETY: `grafts` is NULL or live grafts that nothing else
            // accesses during the call (this export's contract). (Raw: the
            // repository's callers pass the pointer it holds.)
            let Some(g) = (unsafe { grafts.as_mut() }) else {
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
    pub unsafe extern "C" fn git_grafts_parse(
        grafts: Option<&mut Grafts>,
        buf: *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // (C does not check for NULL.)
            let Some(g) = grafts else {
                return crate::util::errors::assert_arg_failed("grafts").raw();
            };
            // SAFETY: `buf` is readable, and unwritten, for `len` bytes
            // during the call (this export's contract).
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
            // SAFETY: `grafts` is non-null, so live grafts that nothing else
            // accesses during the call (this export's contract).
            let g = unsafe { &mut *grafts };
            // SAFETY: `oid` is non-null, so a valid, aligned oid (this
            // export's contract). It is copied before any change: it may be
            // a graft's own oid, which `add` frees (so it stays raw).
            let oid = unsafe { oid.read() };
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
            // SAFETY: as in `git_grafts_add`.
            let g = unsafe { &mut *grafts };
            // SAFETY: as in `git_grafts_add`: copied first, as `remove` may
            // free the graft it points into.
            let oid = unsafe { oid.read() };
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
        mut out: Out<'_, *mut GitCommitGraft>,
        grafts: *mut Grafts,
        oid: Option<&Oid>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !grafts.is_null() && oid.is_some(),
                "out && grafts && oid",
                -1
            );
            let Some(oid) = oid else { return -1 };
            // SAFETY: `grafts` is non-null, so live grafts that nothing
            // changes during the call (this export's contract). (Raw: the
            // commit module passes the pointer the repository holds.)
            let g = unsafe { &*grafts };
            match g.get(oid) {
                Some(graft) => {
                    // The graft's block, which C reads.
                    out.write(ptr::from_ref(graft).cast_mut());
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
        mut out: Out<'_, *mut Oid>,
        mut out_len: Out<'_, usize>,
        grafts: *mut Grafts,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null() && !grafts.is_null(), "out && grafts", -1);
            // SAFETY: `grafts` is non-null, so live grafts that nothing
            // changes during the call (this export's contract). (Raw: the
            // repository passes the pointer it holds.)
            match unsafe { &*grafts }.oids() {
                Ok(a) => {
                    let (p, n, _) = a.into_raw_parts();
                    out.write(p);
                    out_len.write(n);
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
    pub unsafe extern "C" fn git_grafts_size(grafts: Option<&Grafts>) -> usize {
        grafts.map_or(0, Grafts::len)
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
        Oid::parse_hex(s.as_bytes(), OidType::Sha1).unwrap()
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
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid graft OID at line 1"
        );
        assert!(g.parse(format!("{A}\n{B} zz\n").as_bytes()).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid parent OID at line 2"
        );
        assert!(g.parse(format!("{A}x\n").as_bytes()).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid parent OID at line 1"
        );
    }
}
