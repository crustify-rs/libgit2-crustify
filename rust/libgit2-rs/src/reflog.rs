//! Port of `src/libgit2/reflog.c` (and the reflog file format of
//! `refdb_fs.c`): reflogs (`git_reflog`) and their entries
//! (`git_reflog_entry`).
//!
//! Both are mirrored (kind 3): `refdb_reftable.c` allocates them with
//! `git__calloc` and fills the fields, `transaction.c` builds a reflog in
//! a pool and hands it to a backend's `reflog_write`, and the tests read
//! `entry->oid_old` / `oid_cur` / `msg`. The Rust allocates them as C did
//! (`git__allocator`), and frees them as `git_reflog_free` does, whoever
//! made them.
//!
//! The entries are kept oldest first, as in the file; index 0 of the API
//! is the newest (`reflog_inverse_index`).

use crate::util::cabi::{CStrArg, Out};
use core::ffi::{CStr, c_char, c_int};
use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::git_error_set;
use crate::oid::{Oid, OidType};
use crate::parse::GitParseCtx;
use crate::refdb::{self, GitRefdb, RefdbPtr};
use crate::refdb_fs::oid_clear;
use crate::refs::deps::{self as p, RepoPtr, SigRef, git_signature};
use crate::util::alloc;
use crate::util::errors::{ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::vector::GitVector;

/// `struct git_reflog_entry`, mirrored.
#[repr(C)]
pub struct GitReflogEntry {
    /// The previous value.
    pub oid_old: Oid,
    /// The new value.
    pub oid_cur: Oid,
    /// Who made the change (owned).
    pub committer: *mut git_signature,
    /// The message (owned), or NULL.
    pub msg: *mut c_char,
}

/// `struct git_reflog`, mirrored.
#[repr(C)]
pub struct GitReflog {
    /// The database, with a count the reflog holds (or NULL).
    pub db: *mut GitRefdb,
    /// The reference's name (owned).
    pub ref_name: *mut c_char,
    /// `git_oid_t` of the ids.
    pub oid_type: c_int,
    /// The entries, `git_reflog_entry *`, oldest first.
    pub entries: GitVector,
}

/// `git_reflog_entry__free`.
///
/// # Safety
///
/// `entry` is a live entry from libgit2's allocator (its `committer` NULL
/// or a signature from `git_signature_*`, its `msg` NULL or a
/// `git__allocator` string) that the caller gives up.
pub unsafe fn entry_free(entry: *mut GitReflogEntry) {
    // SAFETY: a live, aligned entry the caller gives up (the contract), so
    // nothing else writes it; read before the frees.
    let committer = unsafe { (*entry).committer };
    // SAFETY: as above.
    let msg = unsafe { (*entry).msg };
    // SAFETY: the entry's own signature (or NULL), freed once.
    unsafe { p::git_signature_free(committer) };
    // SAFETY: the entry's own message (or NULL), freed once.
    unsafe { alloc::free(msg.cast()) };
    // SAFETY: the entry's block, from `git__allocator`, freed once.
    unsafe { alloc::free(entry.cast()) };
}

/// `reflog_inverse_index`.
fn inverse(idx: usize, total: usize) -> usize {
    (total - 1) - idx
}

/// An entry of a valid reflog, borrowed for `'a`: its `committer` is NULL
/// or a valid signature, its `msg` NULL or a NUL-terminated string, and
/// nothing writes or frees them during `'a`. Made only by [`ReflogRef`].
#[derive(Clone, Copy)]
pub struct EntryRef<'a>(&'a GitReflogEntry);

impl<'a> EntryRef<'a> {
    /// The entry's fields.
    pub fn get(self) -> &'a GitReflogEntry {
        self.0
    }

    /// `entry->committer`.
    pub fn committer(self) -> Option<SigRef<'a>> {
        // SAFETY: NULL or the entry's valid signature, unwritten and not
        // freed during `'a` (the type's invariant).
        unsafe { SigRef::from_raw(self.0.committer) }
    }

    /// `entry->msg`.
    pub fn msg(self) -> Option<&'a CStr> {
        let m = self.0.msg;
        if m.is_null() {
            return None;
        }
        // SAFETY: a non-null message is a NUL-terminated string the entry
        // holds, unwritten during `'a` (the type's invariant).
        Some(unsafe { CStr::from_ptr(m) })
    }

    /// The entry's address (as C hands it out).
    pub fn as_ptr(self) -> *const GitReflogEntry {
        self.0
    }
}

/// A valid reflog, borrowed for `'a` and read only.
///
/// Invariant (from [`from_ptr`](Self::from_ptr), [`Reflog::as_ref`],
/// [`ReflogMut::as_ref`], or an export's parameter with the same contract):
/// `ptr` is a live, aligned `git_reflog` whose `ref_name` is NULL or a
/// NUL-terminated string, whose `entries` is a vector of live entries (as
/// [`EntryRef`] describes them), and which nothing writes or frees during
/// `'a`. `#[repr(transparent)]`, so `Option<ReflogRef<'_>>` has the ABI of
/// a `const git_reflog *`.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct ReflogRef<'a> {
    ptr: NonNull<GitReflog>,
    _life: PhantomData<&'a GitReflog>,
}

impl<'a> ReflogRef<'a> {
    /// The reflog at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` meets the type's invariant for `'a`.
    pub unsafe fn from_ptr(ptr: NonNull<GitReflog>) -> ReflogRef<'a> {
        ReflogRef {
            ptr,
            _life: PhantomData,
        }
    }

    fn raw(self) -> &'a GitReflog {
        // SAFETY: a live, aligned reflog that nothing writes during `'a`
        // (the type's invariant).
        unsafe { self.ptr.as_ref() }
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut GitReflog {
        self.ptr.as_ptr()
    }

    /// `reflog->ref_name` (`""` for NULL, which no constructor makes).
    pub fn name(self) -> &'a CStr {
        let name = self.raw().ref_name;
        if name.is_null() {
            return c"";
        }
        // SAFETY: a NUL-terminated string the reflog holds, unwritten
        // during `'a` (the type's invariant).
        unsafe { CStr::from_ptr(name) }
    }

    /// `reflog->oid_type`.
    pub fn oid_type(self) -> c_int {
        self.raw().oid_type
    }

    /// `reflog->db`.
    pub fn db(self) -> *mut GitRefdb {
        self.raw().db
    }

    /// `git_reflog_entrycount`.
    pub fn len(self) -> usize {
        self.raw().entries.len()
    }

    /// Whether there is no entry.
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The entry at vector position `i` (`None` past the end or for a NULL
    /// element).
    fn entry_at(self, i: usize) -> Option<EntryRef<'a>> {
        let p = self.raw().entries.get(i)?;
        // SAFETY: an element of this reflog's vector: NULL or a live,
        // aligned entry, unwritten during `'a` (the type's invariant).
        unsafe { p.cast::<GitReflogEntry>().as_ref() }.map(EntryRef)
    }

    /// The entries, oldest first.
    pub fn entries(self) -> impl Iterator<Item = EntryRef<'a>> {
        (0..self.len()).filter_map(move |i| self.entry_at(i))
    }

    /// The entry at API index `idx` (0 is the newest), or `None`
    /// (`git_reflog_entry_byindex`).
    pub fn entry_byindex(self, idx: usize) -> Option<EntryRef<'a>> {
        let n = self.len();
        if idx >= n {
            return None;
        }
        self.entry_at(inverse(idx, n))
    }

    /// The element at API index `idx` as the vector holds it (with the
    /// entry's own provenance, for writing or freeing it), or `None`.
    fn entry_raw(self, idx: usize) -> Option<*mut GitReflogEntry> {
        let v = &self.raw().entries;
        if idx >= v.len() {
            return None;
        }
        v.get(inverse(idx, v.len())).map(|e| e.cast())
    }
}

/// A valid reflog, borrowed exclusively for `'a`: the invariant of
/// [`ReflogRef`], and nothing else reads or writes it during `'a`.
/// `#[repr(transparent)]`, so `Option<ReflogMut<'_>>` has the ABI of a
/// `git_reflog *`.
#[repr(transparent)]
pub struct ReflogMut<'a> {
    ptr: NonNull<GitReflog>,
    _life: PhantomData<&'a mut GitReflog>,
}

impl<'a> ReflogMut<'a> {
    /// The reflog at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` meets [`ReflogRef`]'s invariant for `'a`, and nothing else
    /// uses the reflog during `'a`.
    pub unsafe fn from_ptr(ptr: NonNull<GitReflog>) -> ReflogMut<'a> {
        ReflogMut {
            ptr,
            _life: PhantomData,
        }
    }

    /// The reflog, read only.
    pub fn as_ref(&self) -> ReflogRef<'_> {
        ReflogRef {
            ptr: self.ptr,
            _life: PhantomData,
        }
    }

    /// The raw pointer.
    pub fn as_ptr(&self) -> *mut GitReflog {
        self.ptr.as_ptr()
    }

    fn raw_mut(&mut self) -> &mut GitReflog {
        // SAFETY: a live, aligned reflog that nothing else uses during `'a`
        // (the type's invariant), borrowed with `self`.
        unsafe { self.ptr.as_mut() }
    }

    /// Set `oid_old` of the entry at API index `idx`; `false` if there is
    /// none. (Only the oids may change: the pointers are the invariant's.)
    pub fn set_entry_old(&mut self, idx: usize, old: Oid) -> bool {
        let Some(p) = self.as_ref().entry_raw(idx).filter(|p| !p.is_null()) else {
            return false;
        };
        // SAFETY: a live, aligned entry of the reflog (the invariant), which
        // only the reflog references, and `&mut self` keeps every other
        // user away; `oid_old` is plain data.
        unsafe { (*p).oid_old = old };
        true
    }

    /// Insert an entry we own as the newest; on failure it is freed.
    fn insert(&mut self, e: EntryBox) -> Result<()> {
        let e = e.release();
        if let Err(err) = self.raw_mut().entries.insert(e.as_ptr().cast()) {
            // SAFETY: our live entry (every pointer NULL or owned), not
            // inserted: freed once.
            unsafe { entry_free(e.as_ptr()) };
            return Err(err);
        }
        Ok(())
    }

    /// `reflog_parse`: append the entries of a reflog file; malformed lines
    /// are skipped.
    pub fn parse(&mut self, buf: &[u8]) -> Result<()> {
        let t = OidType::from_raw(self.as_ref().oid_type() as u32);
        let mut parser = GitParseCtx::new(buf);
        while parser.remain_len() > 0 {
            let r = parse_line(&mut parser, t);
            parser.advance_line();
            match r {
                Ok(Some(entry)) => self.insert(entry)?,
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// `git_reflog_append`: append an entry to `new_oid`, whose old id is
    /// the newest entry's (or zero).
    pub fn append(
        &mut self,
        new_oid: &Oid,
        committer: SigRef<'_>,
        msg: Option<&[u8]>,
    ) -> Result<()> {
        let mut entry = EntryBox::new()?;
        let sig = p::SigBox::dup(committer)?;
        entry.get().committer = sig.into_raw();
        if let Some(msg) = msg {
            // `strndup` stops at a NUL in `msg` (C's `git__strdup` of a C
            // string sees no further): only the copy's own length is
            // rewritten (S-2: a message with a NUL from a Rust refdb backend
            // overflowed).
            let len = memchr::memchr(0, msg).unwrap_or(msg.len());
            let m = alloc::strndup(msg, len);
            if m.is_null() {
                return Err(ErrorCode::GENERIC);
            }
            // SAFETY: `m` is our fresh allocation of `len` bytes and a NUL
            // (`strndup` copied the bytes before the first NUL, which are
            // `len`), referenced by nothing else; `len <= msg.len()`, a
            // slice length, so `<= isize::MAX`.
            let bytes = unsafe { core::slice::from_raw_parts_mut(m.cast::<u8>(), len) };
            for c in bytes {
                if *c == b'\n' {
                    *c = b' ';
                }
            }
            entry.get().msg = m;
        }
        let log = self.as_ref();
        entry.get().oid_old = match log.entry_byindex(0) {
            None => oid_clear(log.oid_type()),
            Some(previous) => previous.get().oid_cur,
        };
        entry.get().oid_cur = *new_oid;
        self.insert(entry)
    }

    /// `git_reflog_drop`.
    pub fn drop_entry(&mut self, idx: usize, rewrite_previous_entry: bool) -> Result<()> {
        let entrycount = self.as_ref().len();
        let Some(entry) = self.as_ref().entry_raw(idx).filter(|p| !p.is_null()) else {
            let _ = git_error_set!(ErrorClass::REFERENCE, "no reflog entry at index {}", idx);
            return Err(ErrorCode::NOT_FOUND);
        };
        // SAFETY: an entry the reflog owns (a live `git__allocator` entry,
        // the invariant), removed from the vector next without being read;
        // `&mut self` keeps everyone else away.
        unsafe { entry_free(entry) };
        self.raw_mut().entries.remove(inverse(idx, entrycount))?;
        if !rewrite_previous_entry || idx == 0 || entrycount == 1 {
            return Ok(());
        }
        let t = self.as_ref().oid_type();
        let next_cur = if idx == entrycount - 1 {
            oid_clear(t)
        } else {
            // `idx` < the new count.
            match self.as_ref().entry_byindex(idx) {
                Some(previous) => previous.get().oid_cur,
                None => return Ok(()),
            }
        };
        self.set_entry_old(idx - 1, next_cur);
        Ok(())
    }
}

/// `serialize_reflog_entry`: `<old> <new> <signature>\t<message>\n`, the
/// message's newlines turned into spaces.
pub(crate) fn serialize_entry(
    buf: &mut GitStr,
    old: &Oid,
    new: &Oid,
    committer: SigRef<'_>,
    msg: Option<&[u8]>,
) -> Result<()> {
    buf.clear();
    let _ = buf.puts(old.hex().as_bytes());
    let _ = buf.putc(b' ');
    let _ = buf.puts(new.hex().as_bytes());
    // SAFETY: `buf` is a valid `git_str` (a `&mut`); `committer` a valid
    // signature (`SigRef`), not in `buf`'s buffer (`buf` is exclusively
    // borrowed, the signature only shared).
    unsafe { p::git_signature__writebuf(Some(buf), c" ".into(), Some(committer.get())) };
    buf.rtrim();
    if let Some(msg) = msg {
        let _ = buf.putc(b'\t');
        let _ = buf.puts(msg);
        if !buf.is_oom() {
            let n = buf.len().saturating_sub(2);
            for c in &mut buf.as_mut_bytes()[..n] {
                if *c == b'\n' {
                    *c = b' ';
                }
            }
        }
        buf.rtrim();
    }
    let _ = buf.putc(b'\n');
    if buf.is_oom() {
        // C returns `git_str_oom(buf)` (1), which its callers take for
        // success (see DEFECTS.md).
        return Err(ErrorCode::GENERIC);
    }
    Ok(())
}

/// An owned reflog (`git_reflog *`), freed on drop as `git_reflog_free`.
///
/// Invariant: the pointer meets [`ReflogRef`]'s invariant and the value
/// owns the reflog (one owner; C has given it up or never had it).
pub struct Reflog(NonNull<GitReflog>);

// SAFETY: the one field owns the reflog block, which holds `ref_name` and
// the entries (each with its signature and message), all `git__allocator`
// memory it owns (freed from any thread), the `GitVector` of entry
// pointers (`Send`), the plain `oid_type`, and `db`: NULL or a refdb on
// which the reflog holds a count, which is atomic (`GitRefcount`) and
// dropped on whichever thread drops the value; the reflog never follows
// `db->repo`. Nothing ties any of it to a thread.
unsafe impl Send for Reflog {}
// SAFETY: `&Reflog` only gives `as_ref()` (reads); writes need `&mut`.
unsafe impl Sync for Reflog {}

impl Reflog {
    /// `reflog_alloc(name, oid_type)`: an empty reflog.
    pub fn alloc(name: &CStr, oid_type: c_int) -> Result<Reflog> {
        // `git__allocator` returns `malloc`-aligned memory.
        const _: () = assert!(core::mem::align_of::<GitReflog>() <= 16);
        let log =
            NonNull::new(alloc::calloc(1, core::mem::size_of::<GitReflog>()).cast::<GitReflog>())
                .ok_or(ErrorCode::GENERIC)?;
        let n = alloc::strdup(name);
        if n.is_null() {
            // SAFETY: our fresh block, from `git__calloc`, freed once.
            unsafe { alloc::free(log.as_ptr().cast()) };
            return Err(ErrorCode::GENERIC);
        }
        let l = log.as_ptr();
        // SAFETY: our fresh, zeroed, aligned block, which nothing else
        // references; `db` stays NULL.
        unsafe { (*l).ref_name = n };
        // SAFETY: as above.
        unsafe { (*l).oid_type = oid_type };
        let entries = l
            .wrapping_byte_add(core::mem::offset_of!(GitReflog, entries))
            .cast::<GitVector>();
        // SAFETY: as above; the zeroed vector is written over, not dropped.
        unsafe { entries.write(GitVector::new()) };
        Ok(Reflog(log))
    }

    /// Adopt a reflog.
    ///
    /// # Safety
    ///
    /// `p` is NULL or a reflog that meets [`ReflogRef`]'s invariant, from
    /// `git__allocator` (as `git_reflog_free` frees it), that the caller
    /// gives up.
    pub unsafe fn from_raw(p: *mut GitReflog) -> Option<Reflog> {
        NonNull::new(p).map(Reflog)
    }

    /// Give the reflog to C.
    pub fn into_raw(self) -> NonNull<GitReflog> {
        let p = self.0;
        core::mem::forget(self);
        p
    }

    /// The raw pointer.
    pub fn as_ptr(&self) -> *mut GitReflog {
        self.0.as_ptr()
    }

    /// The reflog, read only.
    pub fn as_ref(&self) -> ReflogRef<'_> {
        ReflogRef {
            ptr: self.0,
            _life: PhantomData,
        }
    }

    /// The reflog, to change.
    pub fn as_mut(&mut self) -> ReflogMut<'_> {
        ReflogMut {
            ptr: self.0,
            _life: PhantomData,
        }
    }

    /// The reference's name.
    pub fn name(&self) -> &CStr {
        self.as_ref().name()
    }

    /// `reflog_parse`: append the entries of a reflog file.
    pub fn parse(&mut self, buf: &[u8]) -> Result<()> {
        self.as_mut().parse(buf)
    }
}

/// An entry being built, freed unless handed over.
struct EntryBox(NonNull<GitReflogEntry>);

impl EntryBox {
    fn new() -> Result<EntryBox> {
        // `git__allocator` returns `malloc`-aligned memory.
        const _: () = assert!(core::mem::align_of::<GitReflogEntry>() <= 16);
        let e = NonNull::new(
            alloc::calloc(1, core::mem::size_of::<GitReflogEntry>()).cast::<GitReflogEntry>(),
        )
        .ok_or(ErrorCode::GENERIC)?;
        Ok(EntryBox(e))
    }

    fn get(&mut self) -> &mut GitReflogEntry {
        // SAFETY: our zeroed (valid: oids, NULL pointers), aligned entry,
        // referenced by nothing else; borrowed with `self`.
        unsafe { self.0.as_mut() }
    }

    fn release(self) -> NonNull<GitReflogEntry> {
        let p = self.0;
        core::mem::forget(self);
        p
    }
}

impl Drop for EntryBox {
    fn drop(&mut self) {
        // SAFETY: our entry, whose pointers are NULL or owned, freed once.
        unsafe { entry_free(self.0.as_ptr()) };
    }
}

/// One line of `reflog_parse`: the entry, `None` for a malformed line,
/// or an allocation error.
fn parse_line(parser: &mut GitParseCtx<'_>, t: Option<OidType>) -> Result<Option<EntryBox>> {
    let mut entry = EntryBox::new()?;
    let sig = alloc::calloc(1, core::mem::size_of::<git_signature>()).cast::<git_signature>();
    if sig.is_null() {
        return Err(ErrorCode::GENERIC);
    }
    entry.get().committer = sig;
    let Some(t) = t else { return Ok(None) };
    let Ok(old) = parser.advance_oid(t) else {
        return Ok(None);
    };
    entry.get().oid_old = old;
    if parser.advance_expected(b" ").is_err() {
        return Ok(None);
    }
    let Ok(cur) = parser.advance_oid(t) else {
        return Ok(None);
    };
    entry.get().oid_cur = cur;
    let line = parser.line();
    let mut c = 0u8;
    while let Some(ch) = parser.peek(false) {
        c = ch;
        if c == b'\t' || c == b'\n' {
            break;
        }
        parser.advance_chars(1);
    }
    let sig_len = line.len() - parser.line().len();
    let mut start = line.as_ptr().cast::<c_char>();
    // SAFETY: `sig` is the signature allocated above (`calloc`: aligned,
    // writable), which nothing references while the parse writes it.
    let sig_out = unsafe { Out::from_ptr(sig) };
    // SAFETY: `start..start + sig_len` is within the parsed buffer (a live
    // slice); the C parser reads only that range and moves `start`, our
    // local, within it.
    let r = unsafe {
        p::git_signature__parse(
            sig_out,
            Some(&mut start),
            line.as_ptr().wrapping_add(sig_len).cast(),
            CStrArg::NULL,
            0,
        )
    };
    if r < 0 {
        return Ok(None);
    }
    if c == b'\t' {
        parser.advance_chars(1);
        let msg = parser.line();
        let mut len = msg.len();
        if len > 0 && msg[len - 1] == b'\n' {
            len -= 1;
        }
        let m = alloc::strndup(msg, len);
        if m.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        entry.get().msg = m;
    }
    Ok(Some(entry))
}

impl Drop for Reflog {
    fn drop(&mut self) {
        // SAFETY: we own the valid reflog (the type's invariant), given up
        // here.
        unsafe { free_raw(self.0.as_ptr()) };
    }
}

/// `git_reflog_free`.
///
/// # Safety
///
/// `log` is NULL, or a reflog meeting [`ReflogRef`]'s invariant, from
/// `git__allocator`, that the caller gives up.
pub unsafe fn free_raw(log: *mut GitReflog) {
    if log.is_null() {
        return;
    }
    // SAFETY: a live, aligned reflog the caller gives up (the contract), so
    // this is the only reference to it; it ends before the block is freed.
    let l = unsafe { &mut *log };
    if !l.db.is_null() {
        // SAFETY: the reflog holds a count on its live database, given up.
        unsafe { refdb::decref(l.db) };
    }
    for e in l.entries.iter() {
        // SAFETY: each element is a live entry the reflog owns (the
        // contract), freed once (the vector is disposed of next).
        unsafe { entry_free(e.cast()) };
    }
    l.entries.dispose();
    // SAFETY: the reflog's own name (or NULL), freed once.
    unsafe { alloc::free(l.ref_name.cast()) };
    // SAFETY: the reflog's block, from `git__allocator`, freed once (`l`
    // is not used after).
    unsafe { alloc::free(log.cast()) };
}

/// The refdb of `repo`, for the reflog functions.
fn refdb_of(repo: RepoPtr) -> Result<RefdbPtr> {
    RefdbPtr::of_repo(repo).map_err(|_| ErrorCode::GENERIC)
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `reflog.c`.
///
/// Every `git_reflog *` is NULL (rejected as an invalid argument) or a
/// valid reflog ([`ReflogRef`]'s invariant: from `git_reflog_read`, a
/// backend, or `transaction.c`) used by one thread at a time; every
/// `git_repository *` NULL or a live repository whose refdb and backend
/// are not replaced during the call.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::export::{barrier, status};

    /// `git_reflog_entry__free`.
    ///
    /// # Safety
    ///
    /// As [`entry_free`]: `entry` is a live entry (from libgit2's
    /// allocator) that the caller gives up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry__free(entry: *mut GitReflogEntry) {
        // SAFETY: this export's contract is `entry_free`'s.
        barrier(|| unsafe { entry_free(entry) })
    }

    /// `git_reflog_free`.
    ///
    /// # Safety
    ///
    /// As [`free_raw`]: `reflog` is NULL or a valid reflog that the caller
    /// gives up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_free(reflog: *mut GitReflog) {
        // SAFETY: this export's contract is `free_raw`'s.
        barrier(|| unsafe { free_raw(reflog) })
    }

    /// `git_reflog_read`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or writable; `repo` NULL or a live repository (see
    /// the module); `name` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_read(
        mut reflog: Out<'_, *mut GitReflog>,
        repo: Option<RepoPtr>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", -1);
            let repo = git_arg!(repo, "repo", -1);
            let name = git_arg!(name.get(), "name", -1);
            let db = match RefdbPtr::of_repo(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            match db.reflog_read(name) {
                Ok(log) => {
                    reflog.write(log.as_ptr());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reflog_write`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or a valid reflog used by nobody else during the
    /// call (see the module), whose `db` is NULL or a live database.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_write(reflog: Option<ReflogMut<'_>>) -> c_int {
        barrier(|| {
            let log = git_arg!(reflog, "reflog", -1);
            let db = git_arg!(NonNull::new(log.as_ref().db()), "reflog->db", -1);
            // SAFETY: the reflog holds a count on its database, live for the
            // call with its backend and repository (this export's contract).
            let d = unsafe { RefdbPtr::new(db) };
            status(d.reflog_write(log))
        })
    }

    /// `git_reflog_append`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or a valid reflog used by nobody else during the
    /// call (see the module); `new_oid` NULL or a valid `git_oid`;
    /// `committer` NULL or a valid signature (it may be an entry's of this
    /// reflog); `msg` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_append(
        reflog: Option<ReflogMut<'_>>,
        new_oid: Option<&Oid>,
        committer: *const git_signature,
        msg: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let mut log = git_arg!(reflog, "reflog", -1);
            let id = git_arg!(new_oid, "new_oid", -1);
            // SAFETY: NULL or a valid signature that nothing writes or frees
            // during the call (this export's contract).
            let committer = git_arg!(unsafe { SigRef::from_raw(committer) }, "committer", -1);
            match log.append(id, committer, msg.get().map(CStr::to_bytes)) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    /// `git_reflog_rename`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); the names
    /// what the backend's `reflog_rename` accepts.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_rename(
        repo: Option<RepoPtr>,
        old_name: *const c_char,
        new_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let db = match refdb_of(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            let (b, f) = match backend_slot(db, |t| t.reflog_rename) {
                Ok(x) => x,
                Err(e) => return e,
            };
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(b, old_name, new_name) }
        })
    }

    /// `git_reflog_delete`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `name` what
    /// the backend's `reflog_delete` accepts.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_delete(
        repo: Option<RepoPtr>,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let db = match refdb_of(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            let (b, f) = match backend_slot(db, |t| t.reflog_delete) {
                Ok(x) => x,
                Err(e) => return e,
            };
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(b, name) }
        })
    }

    /// One load of the repository database's backend and its slot `f`. C
    /// dereferences a NULL backend and calls a NULL slot: both panic here
    /// (an error through the barrier).
    fn backend_slot<T: Copy>(
        db: RefdbPtr,
        f: impl FnOnce(&crate::refdb::git_refdb_backend) -> Option<T>,
    ) -> core::result::Result<(*mut crate::refdb::git_refdb_backend, T), c_int> {
        let b = db.backend();
        assert!(!b.is_null(), "NULL refdb backend");
        // SAFETY: `b` is the non-null backend of a live database (the
        // exports' contract), whose table nothing writes while it is used;
        // the reference lives only for `f`.
        let s = f(unsafe { &*b }).expect("refdb backend slot is NULL");
        Ok((b, s))
    }

    /// `git_reflog_entrycount`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or a valid reflog, unwritten during the call (see
    /// the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entrycount(reflog: Option<ReflogRef<'_>>) -> usize {
        barrier(|| git_arg!(reflog, "reflog", 0).len())
    }

    /// `git_reflog_entry_byindex`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entrycount`]; the result lives until the entry is
    /// dropped or the reflog freed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_byindex(
        reflog: Option<ReflogRef<'_>>,
        idx: usize,
    ) -> *const GitReflogEntry {
        barrier(|| {
            let log = git_arg!(reflog, "reflog", core::ptr::null());
            log.entry_raw(idx)
                .map_or(core::ptr::null(), <*mut GitReflogEntry>::cast_const)
        })
    }

    /// `git_reflog_entry_id_old`.
    ///
    /// # Safety
    ///
    /// `entry` is NULL or a live entry, unwritten during the call; the
    /// result lives as long as the entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_id_old(entry: Option<&GitReflogEntry>) -> *const Oid {
        barrier(|| &raw const git_arg!(entry, "entry", core::ptr::null()).oid_old)
    }

    /// `git_reflog_entry_id_new`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_id_new(entry: Option<&GitReflogEntry>) -> *const Oid {
        barrier(|| &raw const git_arg!(entry, "entry", core::ptr::null()).oid_cur)
    }

    /// `git_reflog_entry_committer`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_committer(
        entry: Option<&GitReflogEntry>,
    ) -> *const git_signature {
        barrier(|| {
            git_arg!(entry, "entry", core::ptr::null())
                .committer
                .cast_const()
        })
    }

    /// `git_reflog_entry_message`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_message(
        entry: Option<&GitReflogEntry>,
    ) -> *const c_char {
        barrier(|| git_arg!(entry, "entry", core::ptr::null()).msg.cast_const())
    }

    /// `git_reflog_drop`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or a valid reflog used by nobody else during the
    /// call (see the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_drop(
        reflog: Option<ReflogMut<'_>>,
        idx: usize,
        rewrite_previous_entry: c_int,
    ) -> c_int {
        barrier(|| {
            let mut log = git_arg!(reflog, "reflog", -1);
            status(log.drop_entry(idx, rewrite_previous_entry != 0))
        })
    }
}

crate::c_layout_test! {
    headers: ["reflog.h"],
    types: [
        GitReflogEntry => "git_reflog_entry" { oid_old, oid_cur, committer, msg },
        GitReflog => "git_reflog" { db, ref_name, oid_type, entries },
    ],
}
