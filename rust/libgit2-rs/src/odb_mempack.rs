//! Port of `src/libgit2/odb_mempack.c`: an in-memory odb backend
//! (`git_mempack_new`), which can be dumped as a pack.
//!
//! The backend is a [`CTable`] over `git_odb_backend`; its state is an
//! `Arc` of the objects under a mutex, shared with the safe API's
//! [`Mempack`] handle, so that a mempack added to an odb can still be
//! counted, reset and dumped. `git_mempack_*` take the table pointer, as
//! C's did, and reach the state through it.
//!
//! The objects map is a `GitHashMap` with C's hash and growth, so that the
//! order in which `git_mempack_write_thin_pack` inserts objects is C's.
//! The lock is never held while C (the packbuilder, which reads objects
//! back through the odb) runs.

use core::ffi::{c_int, c_void};
use core::ptr::NonNull;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use ffibox::CTable;

use crate::git_assert_arg;
use crate::odb::{
    BackendTable, GIT_OBJECT_COMMIT, GitOdbBackend, backend_base, busy, git_object_t,
};
use crate::oid::Oid;
use crate::util::alloc::{GIT_ALLOC, GitAlloc};
use crate::util::errors::{self, ErrorCode, Result};
use crate::util::hashmap::{GitHashMap, HashMapIter};

/// `struct memobject`: an object's type and data (`git__allocator`).
#[derive(Debug)]
struct MemObject {
    type_: git_object_t,
    data: Vec<u8, GitAlloc>,
}

/// The objects, and the commits in the order they were written.
#[derive(Debug, Default)]
struct Objects {
    objects: GitHashMap<Oid, MemObject>,
    commits: Vec<Oid>,
}

/// The state of a mempack backend.
#[derive(Debug, Default)]
pub struct MempackState {
    /// Readers share it (a read copies the object out under the read
    /// lock), writers take it alone.
    inner: RwLock<Objects>,
}

impl MempackState {
    fn lock(&self) -> RwLockWriteGuard<'_, Objects> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self) -> RwLockReadGuard<'_, Objects> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// `impl__write`.
    pub fn write(&self, oid: &Oid, data: &[u8], type_: git_object_t) -> Result<()> {
        let mut o = self.lock();
        if o.objects.contains(oid) {
            return Ok(());
        }
        let mut v = Vec::new_in(GIT_ALLOC);
        v.try_reserve_exact(data.len()).map_err(|_| errors::oom())?;
        v.extend_from_slice(data);
        o.objects.put(*oid, MemObject { type_, data: v })?;
        if type_ == GIT_OBJECT_COMMIT {
            o.commits.try_reserve(1).map_err(|_| errors::oom())?;
            o.commits.push(*oid);
        }
        Ok(())
    }

    /// `impl__exists`.
    pub fn exists(&self, oid: &Oid) -> bool {
        self.read().objects.contains(oid)
    }

    /// `impl__read_header`: the size and type.
    pub fn read_header(&self, oid: &Oid) -> Result<(usize, git_object_t)> {
        let o = self.read();
        let obj = o.objects.get(oid).ok_or(ErrorCode::NOT_FOUND)?;
        Ok((obj.data.len(), obj.type_))
    }

    /// `impl__read`, with the data given to `f` (under the lock).
    pub fn read_with<R>(&self, oid: &Oid, f: impl FnOnce(&[u8], git_object_t) -> R) -> Result<R> {
        let o = self.read();
        let obj = o.objects.get(oid).ok_or(ErrorCode::NOT_FOUND)?;
        Ok(f(&obj.data, obj.type_))
    }

    /// `git_mempack_object_count`.
    pub fn object_count(&self) -> usize {
        self.read().objects.len()
    }

    /// `git_mempack_reset`.
    pub fn reset(&self) {
        let mut o = self.lock();
        // `git_array_clear` frees the array; the map keeps its buckets.
        o.commits = Vec::new();
        o.objects.clear();
    }

    /// The commits written, in order.
    fn commits(&self) -> Result<Vec<Oid>> {
        let o = self.read();
        let mut v = Vec::new();
        v.try_reserve_exact(o.commits.len())
            .map_err(|_| errors::oom())?;
        v.extend_from_slice(&o.commits);
        Ok(v)
    }

    /// The ids, in the map's (C's) order.
    pub fn ids(&self) -> Result<Vec<Oid>> {
        let o = self.read();
        let mut v = Vec::new();
        v.try_reserve_exact(o.objects.len())
            .map_err(|_| errors::oom())?;
        let mut it: HashMapIter = 0;
        while let Some((k, _)) = o.objects.iterate(&mut it) {
            v.push(*k);
        }
        Ok(v)
    }
}

/// The table's state: the shared objects.
#[derive(Debug)]
pub struct MempackBackend(pub(crate) Arc<MempackState>);

impl Drop for MempackBackend {
    fn drop(&mut self) {
        // The table is being freed: the safe API no longer finds it.
        crate::odb::api::unregister_mempack(&self.0);
    }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each of a backend's slots the backend in the
    // place marked `@`, and calls `free` once; the slots left NULL are
    // optional in C's contract (C's mempack set the same ones).
    //
    // Slot arguments (cited as such below): the odb (`odb.rs`: `read_1`,
    // `read_header`, `exists_1`, `write`) passes its `&Oid` as `oid`, its
    // locals as out-parameters and a slice's pointer and length as
    // `data, len`, all valid and aligned for the call and reached by
    // nothing else meanwhile; a C caller of the slots of
    // `git2/sys/odb_backend.h` must pass the same. An `&Oid` made from
    // `oid` lives only inside the slot.
    unsafe impl CVtable<MempackBackend> for BackendTable {}
    base = backend_base();
    /// `impl__read`.
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &MempackBackend| {
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            let r = s.0.read_with(unsafe { &*oid }, |data, t| {
                let p = crate::util::alloc::malloc(data.len()).cast::<u8>();
                if p.is_null() {
                    return -1;
                }
                // SAFETY: `p` is a fresh allocation of `data.len()` bytes
                // (not NULL, checked), so writable for them and disjoint
                // from `data`, the map's vector; `u8` needs no alignment.
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
                // SAFETY: `len_p` is a valid, aligned `size_t *` for the
                // call (slot arguments).
                unsafe { len_p.write(data.len()) };
                // SAFETY: `type_p` likewise (slot arguments).
                unsafe { type_p.write(t) };
                // SAFETY: `buffer_p` likewise (slot arguments); the buffer
                // passes to the caller, which frees it with `git__free`.
                unsafe { buffer_p.write(p.cast()) };
                0
            });
            match r {
                Ok(c) => c,
                Err(e) => e.raw(),
            }
        },
    }
    /// `impl__read_header`.
    fn read_header(len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &MempackBackend| {
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            match s.0.read_header(unsafe { &*oid }) {
                Ok((len, t)) => {
                    // SAFETY: `len_p` is a valid, aligned `size_t *` for the
                    // call (slot arguments).
                    unsafe { len_p.write(len) };
                    // SAFETY: `type_p` likewise (slot arguments).
                    unsafe { type_p.write(t) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `impl__write`.
    fn write(@backend, oid: *const Oid, data: *const c_void, len: usize, type_: git_object_t) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &MempackBackend| {
            if data.is_null() && len != 0 {
                // C read through NULL (rust/DEFECTS.md).
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            let oid = unsafe { &*oid };
            // SAFETY: `data` is readable for `len` initialized bytes that
            // nothing writes during the call (slot arguments: the odb
            // passes a slice); NULL with 0 is the empty slice, and `bytes`
            // panics (into `on panic`) for `len > isize::MAX`.
            let data = unsafe { crate::util::export::bytes(data.cast(), len) };
            crate::util::export::status(s.0.write(oid, data, type_))
        },
    }
    /// `impl__exists`.
    fn exists(@backend, oid: *const Oid) -> c_int {
        on panic: 0,
        on busy: 0,
        // SAFETY: `oid` is a valid id, unwritten for the call (slot
        // arguments, above).
        |s: &MempackBackend| c_int::from(s.0.exists(unsafe { &*oid })),
    }
    release free;
}

/// `git_mempack_new`'s table, over `state`.
pub fn new_backend(state: Arc<MempackState>) -> NonNull<GitOdbBackend> {
    let table = CTable::<BackendTable, MempackBackend>::new(MempackBackend(state.clone())).into_c();
    crate::odb::api::register_mempack(table, &state);
    table
}

/// Run `f` on the state of the mempack table at `backend`; `None` (the
/// error set) if it panicked or the table is being freed.
///
/// # Safety
///
/// `backend` is a table [`new_backend`] (`git_mempack_new`) made (not any
/// other backend's: C casts it alike) that no thread frees during the
/// call.
unsafe fn with_state<R>(
    backend: *mut GitOdbBackend,
    f: impl FnOnce(&MempackState) -> R,
) -> Option<R> {
    // SAFETY: `backend` is the table of a live `CTable<BackendTable,
    // MempackBackend>` (this fn's contract), not freed during the call;
    // the body takes `&S` (shared), so it may run beside the
    // table's slots.
    unsafe {
        CTable::<BackendTable, MempackBackend>::enter(
            backend,
            || {
                errors::set_panic_error();
                None
            },
            || {
                let _ = busy();
                None
            },
            |s: &MempackBackend| Some(f(&s.0)),
        )
    }
}

/// `git_mempack__dump`: a pack of the commits written (and what they
/// reach), from `repo`'s odb.
///
/// # Safety
///
/// `repo` is a live `git_repository *` that no thread frees during the
/// call (the packbuilder made on it is freed before the return).
pub(crate) unsafe fn dump(
    pack: &mut crate::util::str::GitStr,
    repo: *mut c_void,
    state: &MempackState,
) -> Result<()> {
    let commits = state.commits()?;
    // SAFETY: `repo` is a live repository not freed during the call (this
    // fn's contract); the packbuilder, which uses it, is dropped before
    // the return, so it outlives the builder as `new` requires.
    let mut pb = unsafe { crate::sys::pack_objects::Packbuilder::new(repo) }
        .map_err(|_| ErrorCode::GENERIC)?;
    pb.set_threads(0);
    for c in &commits {
        pb.insert_commit(c)?;
    }
    pb.write_buf(pack)
}

/// The C ABI of `odb_mempack.c`.
pub mod ffi {
    use super::*;
    use crate::buf::GitBuf;
    use crate::util::cabi::Out;
    use crate::util::export::barrier;
    use crate::util::str::GitStr;

    /// `git_mempack_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_new(mut out: Out<'_, *mut GitOdbBackend>) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let b = new_backend(Arc::new(MempackState::default()));
            out.write(b.as_ptr());
            0
        })
    }

    /// `git_mempack_write_thin_pack`: insert every object into `pb`.
    ///
    /// # Safety
    ///
    /// `backend` is a mempack table (`git_mempack_new`'s) and `pb` a live
    /// `git_packbuilder *`, neither freed during the call; no other thread
    /// uses `pb` meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_write_thin_pack(
        backend: *mut GitOdbBackend,
        pb: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `backend` is a mempack table not freed during the
            // call (this export's contract), as `with_state` requires.
            let ids = unsafe { with_state(backend, MempackState::ids) }
                .unwrap_or(Err(ErrorCode::GENERIC));
            let ids = match ids {
                Ok(i) => i,
                Err(e) => return e.raw(),
            };
            for id in &ids {
                // SAFETY: `pb` is a live packbuilder that no other thread
                // uses during the call (this export's contract), as `insert`
                // requires; `id` is a local copy.
                if let Err(e) = unsafe { crate::sys::pack_objects::insert(pb, id) } {
                    return e.raw();
                }
            }
            0
        })
    }

    /// `git_mempack_dump`.
    ///
    /// # Safety
    ///
    /// `pack` is NULL or a valid `git_buf`, `repo` a live repository and
    /// `backend` a mempack table, none freed during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_dump(
        pack: *mut GitBuf,
        repo: *mut c_void,
        backend: *mut GitOdbBackend,
    ) -> c_int {
        barrier(|| {
            // GIT_BUF_WRAP_PRIVATE(pack, git_mempack__dump, repo, backend)
            let mut s = GitStr::new();
            // SAFETY: `pack` is NULL or a valid `git_buf` (this export's
            // contract); `s` a writable local string (overwritten, holding
            // nothing to free).
            let mut error = unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, pack) };
            if error == 0 {
                let run = |st: &MempackState| {
                    // SAFETY: `repo` is a live repository not freed during
                    // the call (this export's contract), as `dump` requires.
                    crate::util::export::status(unsafe { dump(&mut s, repo, st) })
                };
                // SAFETY: as in `git_mempack_write_thin_pack`.
                error = unsafe { with_state(backend, run) }.unwrap_or(-1);
                if error == 0 {
                    // SAFETY: `pack` is a valid `git_buf` (`git_buf_tostr`
                    // accepted it), `s` a valid string.
                    error = unsafe { crate::buf::ffi::git_buf_fromstr(pack, Some(&mut s)) };
                }
            }
            drop(s);
            error
        })
    }

    /// `git_mempack_reset`.
    ///
    /// # Safety
    ///
    /// `backend` is a mempack table not freed during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_reset(backend: *mut GitOdbBackend) -> c_int {
        barrier(|| {
            // SAFETY: as in `git_mempack_write_thin_pack`.
            unsafe { with_state(backend, MempackState::reset) }.map_or(-1, |()| 0)
        })
    }

    /// `git_mempack_object_count`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `backend` NULL or a mempack table not
    /// freed during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_object_count(
        mut out: Out<'_, usize>,
        backend: *mut GitOdbBackend,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!backend.is_null(), "_backend", -1);
            // SAFETY: `backend` is a mempack table not freed during the call
            // (this export's contract; NULL is rejected above).
            let n = unsafe { with_state(backend, MempackState::object_count) };
            match n {
                Some(n) => {
                    out.write(n);
                    0
                }
                None => -1,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid::OidType;

    #[test]
    fn write_read_reset() {
        let _g = crate::util::test_support::init();
        let s = MempackState::default();
        let a = Oid::parse_prefix(b"abcd", OidType::Sha1).unwrap();
        let b = Oid::parse_prefix(b"1234", OidType::Sha1).unwrap();
        s.write(&a, b"hello", 3).unwrap();
        s.write(&a, b"other", 3).unwrap();
        s.write(&b, b"tree", GIT_OBJECT_COMMIT).unwrap();
        assert_eq!(s.object_count(), 2);
        assert_eq!(s.read_header(&a).unwrap(), (5, 3));
        assert_eq!(s.read_with(&a, |d, _| d.to_vec()).unwrap(), b"hello");
        assert_eq!(s.commits().unwrap(), vec![b]);
        assert!(s.exists(&b));
        assert_eq!(s.read_header(&Oid::NONE), Err(ErrorCode::NOT_FOUND));
        s.reset();
        assert_eq!(s.object_count(), 0);
        assert!(!s.exists(&a));
    }
}
