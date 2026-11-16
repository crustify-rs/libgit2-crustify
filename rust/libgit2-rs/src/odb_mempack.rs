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
    unsafe impl CVtable<MempackBackend> for BackendTable {}
    base = backend_base();
    /// `impl__read`.
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &MempackBackend| {
            // SAFETY: C's contract: a readable id.
            let r = s.0.read_with(unsafe { &*oid }, |data, t| {
                let p = crate::util::alloc::malloc(data.len()).cast::<u8>();
                if p.is_null() {
                    return -1;
                }
                // SAFETY: a fresh allocation of `data.len()` bytes; C's
                // out-parameters are writable (its contract).
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
                // SAFETY: as above.
                unsafe { len_p.write(data.len()) };
                // SAFETY: as above.
                unsafe { type_p.write(t) };
                // SAFETY: as above.
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
            // SAFETY: C's contract: a readable id.
            match s.0.read_header(unsafe { &*oid }) {
                Ok((len, t)) => {
                    // SAFETY: C's contract: writable out-parameters.
                    unsafe { len_p.write(len) };
                    // SAFETY: as above.
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
            // SAFETY: C's contract: a readable id.
            let oid = unsafe { &*oid };
            // SAFETY: C's contract: `len` readable bytes.
            let data = unsafe { crate::util::export::bytes(data.cast(), len) };
            crate::util::export::status(s.0.write(oid, data, type_))
        },
    }
    /// `impl__exists`.
    fn exists(@backend, oid: *const Oid) -> c_int {
        on panic: 0,
        on busy: 0,
        // SAFETY: C's contract: a readable id.
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
/// `backend` is a live table [`new_backend`] (`git_mempack_new`) made.
unsafe fn with_state<R>(
    backend: *mut GitOdbBackend,
    f: impl FnOnce(&MempackState) -> R,
) -> Option<R> {
    // SAFETY: the caller's contract; the body takes `&S` (shared), so it
    // runs beside the table's slots.
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
/// `repo` is a live `git_repository *`.
pub(crate) unsafe fn dump(
    pack: &mut crate::util::str::GitStr,
    repo: *mut c_void,
    state: &MempackState,
) -> Result<()> {
    let commits = state.commits()?;
    // SAFETY: the caller's contract.
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
    use crate::util::export::barrier;
    use crate::util::str::GitStr;

    /// `git_mempack_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_new(out: *mut *mut GitOdbBackend) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let b = new_backend(Arc::new(MempackState::default()));
            // SAFETY: the caller's contract.
            unsafe { out.write(b.as_ptr()) };
            0
        })
    }

    /// `git_mempack_write_thin_pack`: insert every object into `pb`.
    ///
    /// # Safety
    ///
    /// `backend` is a mempack table; `pb` a live `git_packbuilder *`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_write_thin_pack(
        backend: *mut GitOdbBackend,
        pb: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let ids = unsafe { with_state(backend, MempackState::ids) }
                .unwrap_or(Err(ErrorCode::GENERIC));
            let ids = match ids {
                Ok(i) => i,
                Err(e) => return e.raw(),
            };
            for id in &ids {
                // SAFETY: the caller's packbuilder.
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
    /// `pack` is a writable `git_buf`; `repo` a live repository; `backend`
    /// a mempack table.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_dump(
        pack: *mut GitBuf,
        repo: *mut c_void,
        backend: *mut GitOdbBackend,
    ) -> c_int {
        barrier(|| {
            // GIT_BUF_WRAP_PRIVATE(pack, git_mempack__dump, repo, backend)
            let mut s = GitStr::new();
            // SAFETY: the caller's `git_buf`; `s` a local string.
            let mut error = unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, pack) };
            if error == 0 {
                let run = |st: &MempackState| {
                    // SAFETY: the caller's repository.
                    crate::util::export::status(unsafe { dump(&mut s, repo, st) })
                };
                // SAFETY: the caller's contract: a mempack table.
                error = unsafe { with_state(backend, run) }.unwrap_or(-1);
                if error == 0 {
                    // SAFETY: as above.
                    error = unsafe { crate::buf::ffi::git_buf_fromstr(pack, &raw mut s) };
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
    /// `backend` is a mempack table.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_reset(backend: *mut GitOdbBackend) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe { with_state(backend, MempackState::reset) }.map_or(-1, |()| 0)
        })
    }

    /// `git_mempack_object_count`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `backend` NULL or a mempack table.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_mempack_object_count(
        out: *mut usize,
        backend: *mut GitOdbBackend,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!backend.is_null(), "_backend", -1);
            // SAFETY: the caller's contract.
            let n = unsafe { with_state(backend, MempackState::object_count) };
            match n {
                Some(n) => {
                    // SAFETY: the caller's contract.
                    unsafe { out.write(n) };
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
