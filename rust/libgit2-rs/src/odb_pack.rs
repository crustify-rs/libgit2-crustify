//! Port of `src/libgit2/odb_pack.c`: the object database backend over a
//! repository's packs (and its `multi-pack-index`).
//!
//! The backend is a `git_odb_backend` table (public: `git2/sys/odb_backend.h`)
//! implemented in Rust: [`PackBackend`] is its state, behind ffibox's
//! [`CTable`], whose slots `impl_cvtable!` mints. The same for the
//! `git_odb_writepack` that `writepack` returns, over the indexer (C).
//!
//! # Threads and re-entry
//!
//! Every slot takes the state shared (`&PackBackend`): the odb calls them
//! from several threads, and `foreach`'s callback may call back into the
//! backend (to read the objects it is given). The mutable state (the packs,
//! the multi-pack-index, the last pack an object was found in) is behind a
//! mutex, held for a whole lookup or read (the odb serializes those with its
//! own lock anyway) but never across a callback: `foreach` iterates a
//! snapshot (references to the packs, and to the multi-pack-index).
//!
//! # Types
//!
//! | C | Rust | Kind |
//! |---|---|---|
//! | `git_odb_backend` | [`GitOdbBackend`] | mirrored (public) |
//! | `git_odb_writepack` | [`GitOdbWritepack`] | mirrored (public) |
//! | `git_odb_backend_pack_options` | [`GitOdbBackendPackOptions`] | mirrored (public) |
//! | `struct pack_backend`, `struct pack_writepack` | [`PackBackend`], [`Writepack`] behind [`CTable`] | the C table, then Rust state |

use core::ffi::{CStr, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ffibox::define_ctype;
use ffibox::vtable::CTable;

use crate::mwindow;
use crate::odb;
use crate::oid::{self, GIT_OID_MINPREFIXLEN, Oid, OidType, git_oid_t};
use crate::pack::{GitBytes, GitPackFile, Pack, PackObject, PackObjectType};
use crate::sys::indexer::{GitIndexerOptions, GitIndexerProgress, Indexer, IndexerProgressCb};
use crate::sys::midx::{Midx, MidxWriter, OdbForeachCb};
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::{fs_path, futils, posix};
use crate::{git_assert_arg, git_error_set};

/// `FRESHEN_FREQUENCY`: re-freshen pack files no more than every 2 seconds.
const FRESHEN_FREQUENCY: i64 = 2;

/// `GIT_ODB_BACKEND_VERSION`.
pub const GIT_ODB_BACKEND_VERSION: c_uint = 1;
/// `GIT_ODB_BACKEND_PACK_OPTIONS_VERSION`.
pub const GIT_ODB_BACKEND_PACK_OPTIONS_VERSION: c_uint = 1;

use crate::odb::{ForeachCb as OdbForeachCbOpt, GitOdb};
pub use crate::odb::{GitOdbBackend, GitOdbBackendPackOptions, GitOdbWritepack};

/// A `git_odb_backend` of `GIT_ODB_BACKEND_VERSION` with no slot set.
fn empty_backend() -> GitOdbBackend {
    GitOdbBackend {
        version: GIT_ODB_BACKEND_VERSION,
        ..GitOdbBackend::default()
    }
}

define_ctype!(
    /// A `git_odb_backend` table.
    OdbBackend,
    OdbBackendRef,
    OdbBackendMut,
    GitOdbBackend
);
define_ctype!(
    /// A `git_odb_writepack` table.
    OdbWritepack,
    OdbWritepackRef,
    OdbWritepackMut,
    GitOdbWritepack
);

// ---------------------------------------------------------------------------
// The backend's state
// ---------------------------------------------------------------------------

/// What `refresh` and the lookups change, under [`PackBackend`]'s mutex.
///
/// Every pack, once added, stays in `midx_packs` or `packs` (moved between
/// them, never dropped) until the backend is freed, as in C.
struct BackendState {
    /// The packs the multi-pack-index names, in its order (`midx_packs`).
    midx_packs: Vec<Pack>,
    /// The other packs (`packs`), sorted (`packfile_sort__cb`) when
    /// `packs_sorted`, which follows `git_vector`'s sorted flag.
    packs: Vec<Pack>,
    packs_sorted: bool,
    midx: Option<Arc<Midx>>,
    /// The pack the last object was found in (`last_found`): one of the
    /// packs above.
    last_found: Option<NonNull<GitPackFile>>,
}

// SAFETY: `last_found` points to a pack the state holds a reference to (in
// `midx_packs` or `packs`), which is `Sync`; the rest is `Send`.
unsafe impl Send for BackendState {}

/// The pack backend (`struct pack_backend` after its table).
pub struct PackBackend {
    oid_type: git_oid_t,
    pack_folder: Option<CString>,
    state: Mutex<BackendState>,
}

/// An entry found by the backend (`struct git_pack_entry`, with the pack
/// borrowed from the state).
#[derive(Clone, Copy)]
struct Entry<'s> {
    offset: i64,
    id: Oid,
    p: &'s GitPackFile,
}

/// `packfile_byname_search_cmp == 0`: C's `strncmp(p->pack_name, prefix,
/// strlen(prefix))`, a prefix test.
fn pack_name_starts_with(p: &GitPackFile, prefix: &[u8]) -> bool {
    p.name().to_bytes().starts_with(prefix)
}

/// `packfile_sort__cb`: local packs first (all are), then the younger.
fn packfile_sort(a: &Pack, b: &Pack) -> core::cmp::Ordering {
    b.file().mtime().cmp(&a.file().mtime())
}

impl BackendState {
    fn insert_pack(&mut self, p: Pack) -> Result<()> {
        self.packs.try_reserve(1).map_err(|_| errors::oom())?;
        self.packs.push(p);
        self.packs_sorted = self.packs.len() <= 1;
        Ok(())
    }

    fn sort_packs(&mut self) {
        if !self.packs_sorted {
            // A stable sort, as `git_vector_sort`'s timsort.
            self.packs.sort_by(packfile_sort);
            self.packs_sorted = true;
        }
    }

    fn last_found(&self) -> Option<&GitPackFile> {
        // SAFETY: a pack the state holds (see `last_found`'s invariant),
        // alive while `self` is borrowed.
        self.last_found.map(|p| unsafe { p.as_ref() })
    }

    /// `remove_multi_pack_index`: forget the multi-pack-index, and move its
    /// packs to the others.
    fn remove_multi_pack_index(&mut self) -> Result<()> {
        self.packs
            .try_reserve(self.midx_packs.len())
            .map_err(|_| errors::oom())?;
        // `git_vector_set` past the end: the sorted flag is kept.
        self.packs.append(&mut self.midx_packs);
        self.midx = None;
        Ok(())
    }
}

/// The trampoline for a Rust closure as a `git_odb_foreach_cb`.
unsafe extern "C" fn foreach_trampoline(id: *const Oid, payload: *mut c_void) -> c_int {
    crate::util::export::barrier(|| {
        // SAFETY: `payload` is the `&mut dyn FnMut` `PackBackend::foreach`
        // passed for this call, used by this call alone.
        let f = unsafe { &mut *payload.cast::<&mut dyn FnMut(&Oid) -> c_int>() };
        // SAFETY: a valid oid for the call (the midx's contract).
        let id = unsafe { &*id };
        f(id)
    })
}

/// A `foreach` callback: C's, or a Rust closure.
pub(crate) enum ForeachCb<'a> {
    /// A C callback and its payload.
    C(OdbForeachCb, *mut c_void),
    /// A Rust closure.
    Rust(&'a mut dyn FnMut(&Oid) -> c_int),
}

impl ForeachCb<'_> {
    fn call(&mut self, id: &Oid) -> c_int {
        match self {
            // SAFETY: the contract of the C callback with its payload, which
            // its supplier (the export's caller) states.
            ForeachCb::C(cb, data) => unsafe { cb(id, *data) },
            ForeachCb::Rust(f) => f(id),
        }
    }
}

impl PackBackend {
    /// `pack_backend__alloc`.
    fn new(opts: Option<&GitOdbBackendPackOptions>) -> PackBackend {
        let mut oid_type = opts.map_or(0, |o| o.oid_type);
        if oid_type == 0 {
            oid_type = OidType::DEFAULT.raw();
        }
        PackBackend {
            oid_type,
            pack_folder: None,
            state: Mutex::new(BackendState {
                midx_packs: Vec::new(),
                packs: Vec::new(),
                packs_sorted: true,
                midx: None,
                last_found: None,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BackendState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn oid_hexsize(&self) -> usize {
        oid::oid_hexsize(self.oid_type)
    }

    /// `pack_entry_find`.
    fn entry_find<'s>(&self, st: &'s mut BackendState, id: &Oid) -> Result<Entry<'s>> {
        let hexsize = self.oid_hexsize();
        if let Some(midx) = &st.midx {
            let (r, e) = midx.entry_find(id, hexsize);
            if r == 0 && e.pack_index < st.midx_packs.len() {
                return Ok(Entry {
                    offset: e.offset,
                    id: e.sha1,
                    p: st.midx_packs[e.pack_index].file(),
                });
            }
        }
        let last = st.last_found;
        if let Some(p) = st.last_found()
            && let Ok((offset, found)) = p.entry_find(id, hexsize)
        {
            // SAFETY: as in `BackendState::last_found`, for `'s`.
            let p = unsafe { st.last_found.expect("set").as_ref() };
            return Ok(Entry {
                offset,
                id: found,
                p,
            });
        }
        let mut hit = None;
        for (i, p) in st.packs.iter().enumerate() {
            if Some(NonNull::from(p.file())) == last {
                continue;
            }
            if let Ok((offset, found)) = p.file().entry_find(id, hexsize) {
                hit = Some((i, offset, found));
                break;
            }
        }
        if let Some((i, offset, found)) = hit {
            st.last_found = Some(NonNull::from(st.packs[i].file()));
            return Ok(Entry {
                offset,
                id: found,
                p: st.packs[i].file(),
            });
        }
        Err(odb::error_notfound(
            b"failed to find pack entry",
            Some(id),
            hexsize,
        ))
    }

    /// `pack_entry_find_prefix`: the result, and the entry as C leaves it
    /// (`exists_prefix` copies its id whatever the result).
    fn entry_find_prefix<'s>(
        &self,
        st: &'s mut BackendState,
        short_oid: &Oid,
        len: usize,
    ) -> (Result<()>, Option<Entry<'s>>) {
        let mut e: Option<(i64, Oid, NonNull<GitPackFile>)> = None;
        let mut found_full = Oid::from_raw_parts(short_oid.raw_kind(), [0; oid::GIT_OID_MAX_SIZE]);
        let mut found = false;
        let last = st.last_found;

        let finish = |_: &'s BackendState, e: Option<(i64, Oid, NonNull<GitPackFile>)>| {
            e.map(|(offset, id, p)| Entry {
                offset,
                id,
                // SAFETY: the pack of an entry is one the state (borrowed
                // for `'s`) holds.
                p: unsafe { p.as_ref() },
            })
        };

        if let Some(midx) = &st.midx {
            let (r, me) = midx.entry_find(short_oid, len);
            if r == ErrorCode::AMBIGUOUS.raw() {
                return (Err(ErrorCode::AMBIGUOUS), finish(st, e));
            }
            if r == 0 && me.pack_index < st.midx_packs.len() {
                e = Some((
                    me.offset,
                    me.sha1,
                    NonNull::from(st.midx_packs[me.pack_index].file()),
                ));
                found_full = me.sha1;
                found = true;
            }
        }

        if let Some(p) = st.last_found() {
            match p.entry_find(short_oid, len) {
                Err(ErrorCode::AMBIGUOUS) => return (Err(ErrorCode::AMBIGUOUS), finish(st, e)),
                Ok((offset, id)) => {
                    e = Some((offset, id, last.expect("set")));
                    if found && id != found_full {
                        let r = odb::error_ambiguous(b"found multiple pack entries");
                        return (Err(r), finish(st, e));
                    }
                    found_full = id;
                    found = true;
                }
                Err(_) => {}
            }
        }

        let mut new_last = None;
        for p in &st.packs {
            let pf = p.file();
            if Some(NonNull::from(pf)) == last {
                continue;
            }
            match pf.entry_find(short_oid, len) {
                Err(ErrorCode::AMBIGUOUS) => {
                    if let Some(l) = new_last {
                        st.last_found = Some(l);
                    }
                    return (Err(ErrorCode::AMBIGUOUS), finish(st, e));
                }
                Ok((offset, id)) => {
                    e = Some((offset, id, NonNull::from(pf)));
                    if found && id != found_full {
                        if let Some(l) = new_last {
                            st.last_found = Some(l);
                        }
                        let r = odb::error_ambiguous(b"found multiple pack entries");
                        return (Err(r), finish(st, e));
                    }
                    found_full = id;
                    found = true;
                    new_last = Some(NonNull::from(pf));
                }
                Err(_) => {}
            }
        }
        if let Some(l) = new_last {
            st.last_found = Some(l);
        }

        if !found {
            let r = odb::error_notfound(b"no matching pack entry for prefix", Some(short_oid), len);
            return (Err(r), finish(st, e));
        }
        (Ok(()), finish(st, e))
    }

    /// `pack_backend__read`.
    pub(crate) fn read(&self, id: &Oid) -> Result<(GitBytes, c_int)> {
        let mut st = self.lock();
        let e = self.entry_find(&mut st, id)?;
        let mut off = e.offset;
        e.p.unpack(&mut off)
    }

    /// `pack_backend__read_prefix`: the full id, and the object.
    pub(crate) fn read_prefix(
        &self,
        short_oid: &Oid,
        len: usize,
    ) -> Result<(Oid, GitBytes, c_int)> {
        if len < GIT_OID_MINPREFIXLEN {
            return Err(odb::error_ambiguous(b"prefix length too short"));
        }
        if len >= self.oid_hexsize() {
            // The regular read will do.
            let (data, kind) = self.read(short_oid)?;
            return Ok((*short_oid, data, kind));
        }
        let mut st = self.lock();
        let (r, e) = self.entry_find_prefix(&mut st, short_oid, len);
        r?;
        let e = e.expect("found");
        let mut off = e.offset;
        let (data, kind) = e.p.unpack(&mut off)?;
        Ok((e.id, data, kind))
    }

    /// `pack_backend__read_header`.
    pub(crate) fn read_header(&self, id: &Oid) -> Result<(usize, c_int)> {
        let mut st = self.lock();
        let e = self.entry_find(&mut st, id)?;
        e.p.resolve_header(e.offset)
    }

    /// `pack_backend__exists`.
    pub(crate) fn exists(&self, id: &Oid) -> bool {
        let mut st = self.lock();
        self.entry_find(&mut st, id).is_ok()
    }

    /// `pack_backend__exists_prefix`: the result, and the id C copies out
    /// (the last entry found, or zero).
    pub(crate) fn exists_prefix(&self, short_id: &Oid, len: usize) -> (Result<()>, Oid) {
        let mut st = self.lock();
        let (r, e) = self.entry_find_prefix(&mut st, short_id, len);
        (r, e.map_or(Oid::NONE, |e| e.id))
    }

    /// `pack_backend__freshen`.
    pub(crate) fn freshen(&self, id: &Oid) -> Result<()> {
        let mut st = self.lock();
        let e = self.entry_find(&mut st, id)?;
        // SAFETY: `time(NULL)` has no preconditions.
        let now = unsafe { libc::time(ptr::null_mut()) };
        if e.p.last_freshen() > now - FRESHEN_FREQUENCY {
            return Ok(());
        }
        futils::touch(e.p.name(), Some(now))?;
        e.p.set_last_freshen(now);
        Ok(())
    }

    /// `process_multi_pack_index_pack`.
    fn process_multi_pack_index_pack(
        &self,
        st: &mut BackendState,
        packfile_name: &CStr,
    ) -> Result<()> {
        let folder = self.pack_folder.as_deref().map_or(&b""[..], CStr::to_bytes);
        let mut pack_path = GitStr::new();
        pack_path.joinpath(folder, packfile_name.to_bytes())?;
        let path = pack_path.as_bytes();
        // Ensured by midx_parse_packfile_name().
        if path.len() <= ".idx".len() || !path.ends_with(b".idx") {
            return Err(odb::error_notfound(
                b"midx file contained a non-index",
                None,
                0,
            ));
        }
        let prefix = &path[..path.len() - ".idx".len()];
        if let Some(pos) = st
            .packs
            .iter()
            .position(|p| pack_name_starts_with(p.file(), prefix))
        {
            // Found in the packs: move it to the midx packs.
            let p = st.packs.remove(pos);
            st.midx_packs.push(p);
            return Ok(());
        }
        let path = pack_path.as_c_str().ok_or(ErrorCode::GENERIC)?;
        let p = mwindow::get_pack(path, self.oid_type)?;
        // SAFETY: a reference `get_pack` gave us.
        st.midx_packs
            .push(unsafe { Pack::from_raw(NonNull::new(p).expect("a pack")) });
        Ok(())
    }

    /// `refresh_multi_pack_index`.
    fn refresh_multi_pack_index(&self, st: &mut BackendState) -> Result<()> {
        let folder = self.pack_folder.as_deref().map_or(&b""[..], CStr::to_bytes);
        let mut midx_path = GitStr::new();
        midx_path.joinpath(folder, b"multi-pack-index")?;
        let midx_path = midx_path.as_c_str().ok_or(ErrorCode::GENERIC)?;

        // If the multi-pack-index changed, close it and move its packs to
        // the others, so that none is lost if the new one fails.
        if let Some(midx) = &st.midx {
            if !midx.needs_refresh(midx_path) {
                return Ok(());
            }
            st.remove_multi_pack_index()?;
        }

        let midx = Arc::new(Midx::open(midx_path, self.oid_type)?);
        st.midx = Some(midx.clone());
        st.midx_packs
            .try_reserve(midx.packfile_count())
            .map_err(|_| errors::oom())?;
        for i in 0..midx.packfile_count() {
            let name = midx.packfile_name(i).expect("in range");
            if let Err(e) = self.process_multi_pack_index_pack(st, name) {
                // Back to the state without a multi-pack-index, keeping the
                // packs processed so far.
                let _ = st.remove_multi_pack_index();
                return Err(e);
            }
        }
        Ok(())
    }

    /// `packfile_load__cb`.
    fn packfile_load(&self, st: &mut BackendState, path: &GitStr) -> c_int {
        let p = path.as_bytes();
        if p.len() <= ".idx".len() || !p.ends_with(b".idx") {
            return 0; // not an index
        }
        let prefix = &p[..p.len() - ".idx".len()];
        if st
            .midx_packs
            .iter()
            .any(|x| pack_name_starts_with(x.file(), prefix))
            || st
                .packs
                .iter()
                .any(|x| pack_name_starts_with(x.file(), prefix))
        {
            return 0;
        }
        let Some(cpath) = path.as_c_str() else {
            return -1;
        };
        match mwindow::get_pack(cpath, self.oid_type) {
            // Ignore a missing .pack file, as git does.
            Err(ErrorCode::NOT_FOUND) => {
                errors::clear();
                0
            }
            Err(e) => e.raw(),
            Ok(pk) => {
                // SAFETY: a reference `get_pack` gave us.
                let pk = unsafe { Pack::from_raw(NonNull::new(pk).expect("a pack")) };
                match st.insert_pack(pk) {
                    Ok(()) => 0,
                    Err(e) => e.raw(),
                }
            }
        }
    }

    /// `pack_backend__refresh`.
    pub(crate) fn refresh(&self) -> Result<()> {
        let mut st = self.lock();
        self.refresh_locked(&mut st)
    }

    fn refresh_locked(&self, st: &mut BackendState) -> Result<()> {
        let Some(folder) = &self.pack_folder else {
            return Ok(());
        };
        match posix::stat(folder) {
            Ok(s) if posix::s_isdir(s.st_mode) => {}
            _ => {
                return Err(odb::error_notfound(b"failed to refresh packfiles", None, 0));
            }
        }
        if self.refresh_multi_pack_index(st).is_err() {
            // Fine: the multi-pack-index is just not used.
            errors::clear();
        }
        // Reload all packs.
        let mut path = GitStr::new();
        path.sets(folder.to_bytes())?;
        let r = fs_path::direach(&mut path, 0, &mut |path: &mut GitStr| {
            self.packfile_load(st, path)
        });
        st.sort_packs();
        r
    }

    /// `pack_backend__foreach`: `cb` for every object (of the
    /// multi-pack-index, then of each other pack), until it returns
    /// non-zero, which is returned.
    pub(crate) fn foreach(&self, mut cb: ForeachCb<'_>) -> c_int {
        // Make sure we know about the packfiles; then iterate a snapshot,
        // without the lock, as the callback may call back.
        let (midx, packs) = {
            let mut st = self.lock();
            if let Err(e) = self.refresh_locked(&mut st) {
                return e.raw();
            }
            (st.midx.clone(), st.packs.clone())
        };
        if let Some(midx) = midx {
            let error = match &mut cb {
                // SAFETY: C's callback with C's payload, which keep the
                // contract of a `git_odb_foreach_cb` (the export's caller).
                ForeachCb::C(f, data) => unsafe { midx.foreach_entry(*f, *data) },
                ForeachCb::Rust(f) => {
                    let mut f: &mut dyn FnMut(&Oid) -> c_int = &mut **f;
                    let payload = ptr::from_mut(&mut f).cast::<c_void>();
                    // SAFETY: the trampoline reads `payload` as the `&mut
                    // dyn FnMut` it is, for this call only.
                    unsafe { midx.foreach_entry(foreach_trampoline, payload) }
                }
            };
            if error != 0 {
                return error;
            }
        }
        for p in &packs {
            let error = p.file().foreach_entry(|id| cb.call(id));
            if error != 0 {
                return error;
            }
        }
        0
    }

    /// `get_idx_path`.
    fn idx_path(&self, p: &GitPackFile) -> Result<GitStr> {
        let mut idx_path = GitStr::new();
        let base = self.pack_folder.as_deref().map(CStr::to_bytes);
        fs_path::prettify(&mut idx_path, p.name(), base)?;
        let len = idx_path.len();
        if len <= ".pack".len() || !idx_path.as_bytes().ends_with(b".pack") {
            return Err(odb::error_notfound(
                b"packfile does not end in .pack",
                None,
                0,
            ));
        }
        idx_path.splice(len - ".pack".len(), ".pack".len(), b".idx")?;
        Ok(idx_path)
    }

    /// `pack_backend__writemidx`.
    pub(crate) fn writemidx(&self) -> Result<()> {
        let mut w = MidxWriter::new(self.pack_folder.as_deref(), self.oid_type)?;
        let mut st = self.lock();
        for p in st.midx_packs.iter().chain(st.packs.iter()) {
            let idx_path = self.idx_path(p.file())?;
            w.add(idx_path.as_c_str().ok_or(ErrorCode::GENERIC)?)?;
        }
        // Invalidate the previous midx before writing the new one.
        st.remove_multi_pack_index()?;
        w.commit()?;
        self.refresh_multi_pack_index(&mut st)
    }

    /// `pack_backend__writepack`: a writepack over an indexer writing into
    /// the pack directory.
    ///
    /// # Safety
    ///
    /// `odb` is NULL or a live odb that outlives the writepack;
    /// `progress_cb`, if set, may be called with `progress_payload`.
    unsafe fn writepack(
        &self,
        odb: *mut c_void,
        progress_cb: IndexerProgressCb,
        progress_payload: *mut c_void,
    ) -> Result<Writepack> {
        let mut opts = GitIndexerOptions {
            progress_cb,
            progress_cb_payload: progress_payload,
            odb,
            oid_type: self.oid_type,
            ..GitIndexerOptions::default()
        };
        // SAFETY: `odb` is NULL or a live odb that outlives the writepack,
        // and the callback may be called with its payload (this fn's
        // contract), as `Indexer::new` requires.
        let indexer = unsafe { Indexer::new(self.pack_folder.as_deref(), &mut opts) }
            .map_err(|_| ErrorCode::GENERIC)?;
        Ok(Writepack { indexer })
    }

    /// The packs, in the order of lookups (the multi-pack-index's, then the
    /// others).
    fn packs(&self) -> Vec<Pack> {
        let st = self.lock();
        st.midx_packs
            .iter()
            .chain(st.packs.iter())
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

#[cold]
fn busy() -> c_int {
    git_error_set!(ErrorClass::ODB, "the pack backend is being freed").raw()
}

#[cold]
fn panicked() -> c_int {
    errors::set_panic_error();
    ErrorCode::GENERIC.raw()
}

/// Write an object out to C's out-parameters.
///
/// # Safety
///
/// The pointers are aligned and writable, and no reference to their
/// pointees is live.
unsafe fn write_object(
    buffer_p: *mut *mut c_void,
    len_p: *mut usize,
    type_p: *mut c_int,
    data: GitBytes,
    kind: c_int,
) {
    let (data, len) = data.into_raw();
    // SAFETY: `buffer_p` is writable (this fn's contract); C frees `data`
    // with `git__free`.
    unsafe { buffer_p.write(data.cast()) };
    // SAFETY: `len_p` is writable (this fn's contract).
    unsafe { len_p.write(len) };
    // SAFETY: `type_p` is writable (this fn's contract).
    unsafe { type_p.write(kind) };
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2's odb passes each slot of a `git_odb_backend` the
    // backend in the argument marked `@` (`git2/sys/odb_backend.h`); `write`,
    // `writestream` and `readstream` may be NULL (the odb checks each slot
    // before it calls it), and every slot it calls unconditionally (`free`)
    // is set.
    unsafe impl CVtable<PackBackend> for OdbBackend {}
    base = empty_backend();
    /// `pack_backend__read`.
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut c_int, @backend, oid: *const Oid) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            // SAFETY: a valid oid (the odb's contract).
            match b.read(unsafe { &*oid }) {
                Ok((data, kind)) => {
                    // SAFETY: the out-parameters are writable.
                    unsafe { write_object(buffer_p, len_p, type_p, data, kind) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `pack_backend__read_prefix`.
    fn read_prefix(
        out_oid: *mut Oid,
        buffer_p: *mut *mut c_void,
        len_p: *mut usize,
        type_p: *mut c_int,
        @backend,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            // SAFETY: a valid oid (the odb's contract).
            match b.read_prefix(unsafe { &*short_oid }, len) {
                Ok((id, data, kind)) => {
                    // SAFETY: the out-parameters are writable.
                    unsafe { write_object(buffer_p, len_p, type_p, data, kind) };
                    // SAFETY: writable.
                    unsafe { out_oid.write(id) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `pack_backend__read_header`.
    fn read_header(len_p: *mut usize, type_p: *mut c_int, @backend, oid: *const Oid) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            git_assert_arg!(!len_p.is_null(), "len_p", -1);
            git_assert_arg!(!type_p.is_null(), "type_p", -1);
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: a valid oid (checked not NULL; the odb's contract).
            match b.read_header(unsafe { &*oid }) {
                Ok((len, kind)) => {
                    // SAFETY: writable (checked not NULL).
                    unsafe { len_p.write(len) };
                    // SAFETY: as above.
                    unsafe { type_p.write(kind) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `pack_backend__exists`.
    fn exists(@backend, oid: *const Oid) -> c_int {
        on panic: { errors::set_panic_error(); 0 },
        on busy: 0,
        // SAFETY: a valid oid (the odb's contract).
        |b: &PackBackend| c_int::from(b.exists(unsafe { &*oid })),
    }
    /// `pack_backend__exists_prefix`.
    fn exists_prefix(out: *mut Oid, @backend, short_id: *const Oid, len: usize) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            // SAFETY: a valid oid (the odb's contract).
            let (r, id) = b.exists_prefix(unsafe { &*short_id }, len);
            // SAFETY: writable (the odb's contract); C copies the entry's id
            // whatever the result.
            unsafe { out.write(id) };
            match r {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        },
    }
    /// `pack_backend__refresh`.
    fn refresh(@backend) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| crate::util::export::status(b.refresh()),
    }
    /// `pack_backend__foreach`.
    fn foreach(@backend, cb: OdbForeachCbOpt, data: *mut c_void) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            git_assert_arg!(cb.is_some(), "cb", -1);
            b.foreach(ForeachCb::C(cb.expect("checked"), data))
        },
    }
    /// `pack_backend__writepack`.
    fn writepack(
        out: *mut *mut GitOdbWritepack,
        @backend,
        odb: *mut GitOdb,
        progress_cb: IndexerProgressCb,
        progress_payload: *mut c_void,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| {
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: writable (checked not NULL).
            unsafe { out.write(ptr::null_mut()) };
            // SAFETY: the odb's contract: `odb` is the odb this backend is
            // in, which outlives the writepack, and the callback may be
            // called with its payload.
            match unsafe { b.writepack(odb.cast(), progress_cb, progress_payload) } {
                Ok(w) => {
                    let base = GitOdbWritepack {
                        backend,
                        append: None,
                        commit: None,
                        free: None,
                    };
                    // SAFETY: the base's slots are all NULL (`install` sets
                    // every one); its `backend` is the table C reads it as.
                    let t = unsafe { CTable::<OdbWritepack, Writepack>::with_base(base, w) };
                    // SAFETY: writable (checked not NULL); C frees it
                    // through its `free` slot.
                    unsafe { out.write(t.into_c().as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `pack_backend__writemidx`.
    fn writemidx(@backend) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |b: &PackBackend| crate::util::export::status(b.writemidx()),
    }
    /// `pack_backend__freshen`.
    fn freshen(@backend, oid: *const Oid) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: a valid oid (the odb's contract).
        |b: &PackBackend| crate::util::export::status(b.freshen(unsafe { &*oid })),
    }
    release free;
}

/// The writepack (`struct pack_writepack` after its table).
pub struct Writepack {
    indexer: Indexer,
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a `git_odb_writepack` the
    // writepack, first (`git2/odb_backend.h`), and every slot is set.
    unsafe impl CVtable<Writepack> for OdbWritepack {}
    base = GitOdbWritepack {
        backend: ptr::null_mut(),
        append: None,
        commit: None,
        free: None,
    };
    /// `pack_backend__writepack_append`.
    fn append(@writepack, data: *const c_void, size: usize, stats: *mut GitIndexerProgress) -> c_int {
        on panic: panicked(),
        on busy: git_error_set!(ErrorClass::INDEXER, "the writepack is in use").raw(),
        // SAFETY: the caller's buffer and progress (C's contract with
        // `git_indexer_append`).
        |w: &mut Writepack| unsafe { w.indexer.append_raw(data, size, stats) },
    }
    /// `pack_backend__writepack_commit`.
    fn commit(@writepack, stats: *mut GitIndexerProgress) -> c_int {
        on panic: panicked(),
        on busy: git_error_set!(ErrorClass::INDEXER, "the writepack is in use").raw(),
        // SAFETY: the caller's progress.
        |w: &mut Writepack| unsafe { w.indexer.commit(stats) },
    }
    release free;
}

/// Make a backend table, `refresh`ed if it has a pack directory.
fn new_table(backend: PackBackend) -> CTable<OdbBackend, PackBackend> {
    CTable::new(backend)
}

/// `git_odb_backend_pack`: the backend over `objects_dir/pack`.
pub fn backend_pack(
    objects_dir: &[u8],
    opts: Option<&GitOdbBackendPackOptions>,
) -> Result<CTable<OdbBackend, PackBackend>> {
    let mut backend = PackBackend::new(opts);
    let mut path = GitStr::new();
    path.joinpath(objects_dir, b"pack")?;
    if let Some(c) = path.as_c_str()
        && fs_path::isdir(c)
    {
        backend.pack_folder = Some(c.to_owned());
        backend.refresh()?;
    }
    Ok(new_table(backend))
}

/// `git_odb_backend_pack`, as the odb adds it: the table, which the odb
/// frees through its `free` slot.
pub(crate) fn backend_pack_raw(
    objects_dir: &CStr,
    opts: &GitOdbBackendPackOptions,
) -> Result<NonNull<GitOdbBackend>> {
    Ok(backend_pack(objects_dir.to_bytes(), Some(opts))?.into_c())
}

/// `git_odb_backend_one_pack`: the backend over the one pack of the index
/// `idx`.
pub fn backend_one_pack(
    idx: &CStr,
    opts: Option<&GitOdbBackendPackOptions>,
) -> Result<CTable<OdbBackend, PackBackend>> {
    let backend = PackBackend::new(opts);
    let oid_type = opts.map_or(0, |o| o.oid_type);
    let p = mwindow::get_pack(idx, oid_type).map_err(|_| ErrorCode::GENERIC)?;
    // SAFETY: a reference `get_pack` gave us.
    let p = unsafe { Pack::from_raw(NonNull::new(p).expect("a pack")) };
    backend
        .lock()
        .insert_pack(p)
        .map_err(|_| ErrorCode::GENERIC)?;
    Ok(new_table(backend))
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// The odb backend over a repository's packs: what `git_odb_backend_pack`
/// makes, used from Rust.
pub struct OdbPackBackend(CTable<OdbBackend, PackBackend>);

impl core::fmt::Debug for OdbPackBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbPackBackend").finish_non_exhaustive()
    }
}

// SAFETY: the wrapper reaches the state only shared (`CTable::state`), and
// `PackBackend` is `Sync` (its mutable state is behind a mutex); it reaches
// the table (raw pointers, which C reads) only to give it up in
// `into_c(self)`. Dropping it on another thread drops the state, which is
// `Send`, and frees the allocation.
unsafe impl Send for OdbPackBackend {}
// SAFETY: as above.
unsafe impl Sync for OdbPackBackend {}

type ApiResult<T> = core::result::Result<T, errors::Error>;

impl OdbPackBackend {
    /// The backend over the packs of `objects_dir/pack` (none if there is no
    /// such directory).
    pub fn open(objects_dir: &[u8], oid_type: OidType) -> ApiResult<OdbPackBackend> {
        let opts = GitOdbBackendPackOptions {
            version: GIT_ODB_BACKEND_PACK_OPTIONS_VERSION,
            oid_type: oid_type.raw(),
        };
        Ok(OdbPackBackend(backend_pack(objects_dir, Some(&opts))?))
    }

    /// The backend over the one pack of the index `idx_path`.
    pub fn open_one(idx_path: &[u8], oid_type: OidType) -> ApiResult<OdbPackBackend> {
        let opts = GitOdbBackendPackOptions {
            version: GIT_ODB_BACKEND_PACK_OPTIONS_VERSION,
            oid_type: oid_type.raw(),
        };
        let idx = CString::new(idx_path).map_err(|_| {
            errors::Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL"))
        })?;
        Ok(OdbPackBackend(backend_one_pack(&idx, Some(&opts))?))
    }

    fn b(&self) -> &PackBackend {
        // All the backend's slots are shared: the state is always lent.
        self.0.state().expect("no exclusive slot")
    }

    /// The object `id`.
    pub fn read(&self, id: &Oid) -> ApiResult<PackObject> {
        let (data, kind) = self.b().read(id)?;
        to_object(data, kind)
    }

    /// The object whose id starts with the first `len` hex digits of
    /// `short_id`, and its full id.
    pub fn read_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<(Oid, PackObject)> {
        let (id, data, kind) = self.b().read_prefix(short_id, len)?;
        Ok((id, to_object(data, kind)?))
    }

    /// The size and type of the object `id`.
    pub fn read_header(&self, id: &Oid) -> ApiResult<(usize, PackObjectType)> {
        let (size, kind) = self.b().read_header(id)?;
        let kind = PackObjectType::from_raw(kind).ok_or_else(invalid_type)?;
        Ok((size, kind))
    }

    /// Whether the object `id` is in a pack.
    pub fn exists(&self, id: &Oid) -> bool {
        self.b().exists(id)
    }

    /// The full id of the one object whose id starts with the first `len`
    /// hex digits of `short_id`.
    pub fn exists_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<Oid> {
        let (r, id) = self.b().exists_prefix(short_id, len);
        r?;
        Ok(id)
    }

    /// Look for packs added since the last refresh.
    pub fn refresh(&self) -> ApiResult<()> {
        Ok(self.b().refresh()?)
    }

    /// Call `f` with every object id, until it returns `false`. Returns
    /// whether every call returned `true`.
    pub fn foreach(&self, mut f: impl FnMut(&Oid) -> bool) -> ApiResult<bool> {
        let mut stopped = false;
        let mut g = |id: &Oid| {
            if f(id) {
                0
            } else {
                stopped = true;
                1
            }
        };
        let r = self.b().foreach(ForeachCb::Rust(&mut g));
        if stopped {
            errors::clear();
            return Ok(false);
        }
        match ErrorCode::from_raw(r) {
            None => Ok(true),
            Some(e) => Err(e.into()),
        }
    }

    /// Update the modification time of the pack holding `id` (at most every
    /// two seconds).
    pub fn freshen(&self, id: &Oid) -> ApiResult<()> {
        Ok(self.b().freshen(id)?)
    }

    /// Write a `multi-pack-index` of the packs.
    pub fn write_multi_pack_index(&self) -> ApiResult<()> {
        Ok(self.b().writemidx()?)
    }

    /// The packs.
    pub fn packs(&self) -> Vec<Pack> {
        self.b().packs()
    }

    /// The table, for an odb (C's `git_odb_add_backend`), which frees it.
    #[allow(dead_code)] // for l-odb's `Odb`, at integration
    pub(crate) fn into_c(self) -> NonNull<GitOdbBackend> {
        self.0.into_c()
    }
}

fn invalid_type() -> errors::Error {
    git_error_set!(
        ErrorClass::ODB,
        "invalid pack file - invalid packfile type in header"
    )
    .into()
}

fn to_object(data: GitBytes, kind: c_int) -> ApiResult<PackObject> {
    let kind = PackObjectType::from_raw(kind).ok_or_else(invalid_type)?;
    Ok(PackObject::new(kind, data))
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// The exports of `odb_pack.c`.
pub mod ffi {
    use super::*;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::barrier;

    /// `git_odb_backend_pack_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb_backend_pack_options_init(
        mut opts: Out<'_, GitOdbBackendPackOptions>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            if version == 0 || version > GIT_ODB_BACKEND_PACK_OPTIONS_VERSION {
                return git_error_set!(
                    ErrorClass::INVALID,
                    "invalid version {} on git_odb_backend_pack_options",
                    version as c_int
                )
                .raw();
            }
            opts.write(GitOdbBackendPackOptions {
                version: GIT_ODB_BACKEND_PACK_OPTIONS_VERSION,
                oid_type: 0,
            });
            0
        })
    }

    /// `git_odb_backend_one_pack`.
    ///
    /// # Safety
    ///
    /// `backend_out` is NULL or writable; `idx` is NULL or NUL-terminated;
    /// `opts` is NULL or readable, unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb_backend_one_pack(
        mut backend_out: Out<'_, *mut GitOdbBackend>,
        idx: CStrArg<'_>,
        opts: Option<&GitOdbBackendPackOptions>,
    ) -> c_int {
        barrier(|| {
            // (C dereferences a NULL `idx`.)
            let idx = crate::git_arg!(idx.get(), "idx", -1);
            match backend_one_pack(idx, opts) {
                Ok(t) => {
                    backend_out.write(t.into_c().as_ptr());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_odb_backend_pack`.
    ///
    /// # Safety
    ///
    /// `backend_out` is NULL or writable; `objects_dir` is NULL or
    /// NUL-terminated; `opts` is NULL or readable, unwritten during the
    /// call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb_backend_pack(
        mut backend_out: Out<'_, *mut GitOdbBackend>,
        objects_dir: CStrArg<'_>,
        opts: Option<&GitOdbBackendPackOptions>,
    ) -> c_int {
        barrier(|| {
            // (C dereferences a NULL `objects_dir`.)
            let dir = crate::git_arg!(objects_dir.get(), "objects_dir", -1);
            let (out, r) = match backend_pack(dir.to_bytes(), opts) {
                Ok(t) => (t.into_c().as_ptr(), 0),
                Err(e) => (ptr::null_mut(), e.raw()),
            };
            backend_out.write(out);
            r
        })
    }
}

// The layouts of `git_odb_backend`, `git_odb_writepack` and
// `git_odb_backend_pack_options` are tested with `crate::odb`'s.
const _: () = assert!(GIT_ODB_BACKEND_VERSION == 1 && GIT_ODB_BACKEND_PACK_OPTIONS_VERSION == 1);
