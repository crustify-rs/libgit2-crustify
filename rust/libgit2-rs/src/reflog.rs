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
use core::ptr::NonNull;

use crate::git_error_set;
use crate::oid::{Oid, OidType};
use crate::parse::GitParseCtx;
use crate::refdb::{self, GitRefdb, RefdbPtr};
use crate::refdb_fs::oid_clear;
use crate::refs::deps::{self as p, RepoPtr, git_repository, git_signature};
use crate::util::alloc;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
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
/// `entry` is a live entry from libgit2's allocator, given up.
pub unsafe fn entry_free(entry: *mut GitReflogEntry) {
    // SAFETY: per the contract: a live entry, ours.
    let e = unsafe { &mut *entry };
    // SAFETY: the entry's own signature (or NULL), freed once.
    unsafe { p::git_signature_free(e.committer) };
    // SAFETY: the entry's own message (or NULL), freed once.
    unsafe { alloc::free(e.msg.cast()) };
    // SAFETY: the entry's block, from `git__allocator`, freed once.
    unsafe { alloc::free(entry.cast()) };
}

/// `reflog->ref_name`.
///
/// # Safety
///
/// `log` is a live reflog that outlives the borrow and keeps its name.
pub unsafe fn name_of<'a>(log: *const GitReflog) -> &'a CStr {
    // SAFETY: per the contract: a live reflog.
    let name = unsafe { (*log).ref_name };
    // SAFETY: the name is a string the reflog holds.
    unsafe { CStr::from_ptr(name) }
}

/// The entries of `log`, oldest first.
///
/// # Safety
///
/// As [`name_of`], the entries unchanged during the borrow.
pub unsafe fn entries_of<'a>(
    log: *const GitReflog,
) -> impl Iterator<Item = *mut GitReflogEntry> + 'a {
    // SAFETY: per the contract: a live reflog whose vector is valid.
    let v: &'a GitVector = unsafe { &(*log).entries };
    v.iter().map(|e| e.cast::<GitReflogEntry>())
}

/// `reflog_inverse_index`.
fn inverse(idx: usize, total: usize) -> usize {
    (total - 1) - idx
}

/// The entry at API index `idx` (0 is the newest), or NULL
/// (`git_reflog_entry_byindex`).
///
/// # Safety
///
/// `log` is a live reflog.
pub unsafe fn entry_byindex(log: *const GitReflog, idx: usize) -> *mut GitReflogEntry {
    // SAFETY: per the contract.
    let v = unsafe { &(*log).entries };
    if idx >= v.len() {
        return core::ptr::null_mut();
    }
    v.get(inverse(idx, v.len()))
        .unwrap_or(core::ptr::null_mut())
        .cast()
}

/// `serialize_reflog_entry`: `<old> <new> <signature>\t<message>\n`, the
/// message's newlines turned into spaces.
pub(crate) fn serialize_entry(
    buf: &mut GitStr,
    old: &Oid,
    new: &Oid,
    committer: *const git_signature,
    msg: Option<&[u8]>,
) -> Result<()> {
    buf.clear();
    let _ = buf.puts(old.hex().as_bytes());
    let _ = buf.putc(b' ');
    let _ = buf.puts(new.hex().as_bytes());
    // SAFETY: `committer` is NULL or the caller's signature (C's contract
    // for the reflog writers: valid), only read.
    let committer = unsafe { committer.as_ref() };
    // SAFETY: `buf` is a valid `git_str`; `committer` NULL or valid.
    unsafe { p::git_signature__writebuf(Some(buf), c" ".into(), committer) };
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
pub struct Reflog(NonNull<GitReflog>);

// SAFETY: a reflog owns its entries and strings; its count on the refdb is
// atomic.
unsafe impl Send for Reflog {}
// SAFETY: shared access only reads.
unsafe impl Sync for Reflog {}

impl Reflog {
    /// `reflog_alloc(name, oid_type)`: an empty reflog.
    pub fn alloc(name: &CStr, oid_type: c_int) -> Result<Reflog> {
        let log =
            NonNull::new(alloc::calloc(1, core::mem::size_of::<GitReflog>()).cast::<GitReflog>())
                .ok_or(ErrorCode::GENERIC)?;
        let n = alloc::strdup(name);
        if n.is_null() {
            // SAFETY: our block.
            unsafe { alloc::free(log.as_ptr().cast()) };
            return Err(ErrorCode::GENERIC);
        }
        let l = log.as_ptr();
        // SAFETY: a fresh zeroed block: every field is written before use.
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
    /// `p` is NULL or a live reflog the caller gives up.
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

    fn raw(&self) -> &GitReflog {
        // SAFETY: we own the live reflog.
        unsafe { self.0.as_ref() }
    }

    fn raw_mut(&mut self) -> &mut GitReflog {
        // SAFETY: we own the live reflog, exclusively borrowed.
        unsafe { self.0.as_mut() }
    }

    /// The reference's name.
    pub fn name(&self) -> &CStr {
        // SAFETY: our reflog's name.
        unsafe { name_of(self.0.as_ptr()) }
    }

    /// `reflog_parse`: append the entries of a reflog file; malformed lines
    /// are skipped.
    pub fn parse(&mut self, buf: &[u8]) -> Result<()> {
        let oid_type = self.raw().oid_type;
        let t = OidType::from_raw(oid_type as u32);
        let mut parser = GitParseCtx::new(buf);
        while parser.remain_len() > 0 {
            let r = parse_line(&mut parser, t);
            parser.advance_line();
            match r {
                Ok(Some(entry)) => {
                    let e = entry.release();
                    if let Err(err) = self.raw_mut().entries.insert(e.as_ptr().cast()) {
                        // SAFETY: our entry, not inserted.
                        unsafe { entry_free(e.as_ptr()) };
                        return Err(err);
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// An entry being built, freed unless handed over.
struct EntryBox(NonNull<GitReflogEntry>);

impl EntryBox {
    fn new() -> Result<EntryBox> {
        let e = NonNull::new(
            alloc::calloc(1, core::mem::size_of::<GitReflogEntry>()).cast::<GitReflogEntry>(),
        )
        .ok_or(ErrorCode::GENERIC)?;
        Ok(EntryBox(e))
    }

    fn get(&mut self) -> &mut GitReflogEntry {
        // SAFETY: our zeroed (valid: oids, NULL pointers) entry.
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
        // SAFETY: our entry, whose pointers are NULL or owned.
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
    // writable, or NULL), which nothing references.
    let sig_out = unsafe { Out::from_ptr(sig) };
    // SAFETY: `start..start + sig_len` is within the parsed buffer; the
    // C parser reads only that range.
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
        // SAFETY: we own the reflog.
        unsafe { free_raw(self.0.as_ptr()) };
    }
}

/// `git_reflog_free`.
///
/// # Safety
///
/// `log` is NULL or a live reflog from libgit2's allocator, given up.
pub unsafe fn free_raw(log: *mut GitReflog) {
    if log.is_null() {
        return;
    }
    // SAFETY: per the contract: a live reflog, ours from now on.
    let l = unsafe { &mut *log };
    if !l.db.is_null() {
        // SAFETY: the reflog holds a count on its live database.
        unsafe { refdb::decref(l.db) };
    }
    for e in l.entries.iter() {
        // SAFETY: the reflog's entries are its own, freed once.
        unsafe { entry_free(e.cast()) };
    }
    l.entries.dispose();
    // SAFETY: the reflog's own name.
    unsafe { alloc::free(l.ref_name.cast()) };
    // SAFETY: the reflog's block, from `git__allocator`, freed once (`l`
    // is not used after).
    unsafe { alloc::free(log.cast()) };
}

/// `git_reflog_append`: append an entry to `new_oid`, whose old id is the
/// newest entry's (or zero).
///
/// # Safety
///
/// `log` is a live reflog used by nobody else during the call;
/// `committer` a valid signature.
pub unsafe fn append(
    log: *mut GitReflog,
    new_oid: &Oid,
    committer: *const git_signature,
    msg: Option<&[u8]>,
) -> Result<()> {
    let mut entry = EntryBox::new()?;
    // SAFETY: per the contract.
    let sig = unsafe { p::SigBox::dup(committer) }?;
    entry.get().committer = sig.into_raw();
    if let Some(msg) = msg {
        // `strndup` stops at a NUL in `msg` (C's `git__strdup` of a C
        // string sees no further): only the copy's own length is rewritten
        // (S-2: a message with a NUL from a Rust refdb backend overflowed).
        let len = memchr::memchr(0, msg).unwrap_or(msg.len());
        let m = alloc::strndup(msg, len);
        if m.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: `m` holds `len` bytes and a NUL (`strndup` copied the
        // bytes before the first NUL, which are `len`).
        let bytes = unsafe { core::slice::from_raw_parts_mut(m.cast::<u8>(), len) };
        for c in bytes {
            if *c == b'\n' {
                *c = b' ';
            }
        }
        entry.get().msg = m;
    }
    // SAFETY: per the contract.
    let previous = unsafe { entry_byindex(log, 0) };
    entry.get().oid_old = if previous.is_null() {
        // SAFETY: a live reflog.
        oid_clear(unsafe { (*log).oid_type })
    } else {
        // SAFETY: a live entry.
        unsafe { (*previous).oid_cur }
    };
    entry.get().oid_cur = *new_oid;
    let e = entry.release();
    // SAFETY: per the contract (exclusive use of the reflog).
    if let Err(err) = unsafe { (*log).entries.insert(e.as_ptr().cast()) } {
        // SAFETY: our entry, not inserted.
        unsafe { entry_free(e.as_ptr()) };
        return Err(err);
    }
    Ok(())
}

/// `git_reflog_drop`.
///
/// # Safety
///
/// As [`append`].
pub unsafe fn drop_entry(
    log: *mut GitReflog,
    idx: usize,
    rewrite_previous_entry: bool,
) -> Result<()> {
    // SAFETY: per the contract.
    let entrycount = unsafe { (*log).entries.len() };
    // SAFETY: per the contract.
    let entry = unsafe { entry_byindex(log, idx) };
    if entry.is_null() {
        let _ = git_error_set!(ErrorClass::REFERENCE, "no reflog entry at index {}", idx);
        return Err(ErrorCode::NOT_FOUND);
    }
    // SAFETY: an entry of the reflog, removed from it next.
    unsafe { entry_free(entry) };
    // SAFETY: per the contract.
    unsafe { (*log).entries.remove(inverse(idx, entrycount)) }?;
    if !rewrite_previous_entry || idx == 0 || entrycount == 1 {
        return Ok(());
    }
    // SAFETY: per the contract.
    let entry = unsafe { entry_byindex(log, idx - 1) };
    if idx == entrycount - 1 {
        // SAFETY: a live reflog.
        let t = unsafe { (*log).oid_type };
        // SAFETY: a live entry (`idx - 1` < the new count).
        unsafe { (*entry).oid_old = oid_clear(t) };
        return Ok(());
    }
    // SAFETY: a live reflog; `idx` < the new count.
    let previous = unsafe { entry_byindex(log, idx) };
    // SAFETY: a live entry of it.
    let cur = unsafe { (*previous).oid_cur };
    // SAFETY: as above.
    unsafe { (*entry).oid_old = cur };
    Ok(())
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
/// Every `git_reflog *` is NULL (where C checks it) or a live reflog used
/// by one thread at a time; every `git_repository *` a live repository.
pub mod ffi {
    use super::*;
    use crate::util::cabi::CStrArg;
    use crate::util::export::{barrier, status};

    /// `git_reflog_entry__free`.
    ///
    /// # Safety
    ///
    /// `entry` is a live entry, given up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry__free(entry: *mut GitReflogEntry) {
        // SAFETY: per the contract.
        barrier(|| unsafe { entry_free(entry) })
    }

    /// `git_reflog_free`.
    ///
    /// # Safety
    ///
    /// `reflog` is NULL or a live reflog, given up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_free(reflog: *mut GitReflog) {
        // SAFETY: per the contract.
        barrier(|| unsafe { free_raw(reflog) })
    }

    /// `git_reflog_read`.
    ///
    /// # Safety
    ///
    /// `reflog` writable; `repo` live; `name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_read(
        reflog: *mut *mut GitReflog,
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", -1);
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            crate::git_assert_arg!(!name.is_null(), "name", -1);
            // SAFETY: per the contract.
            let repo = unsafe { RepoPtr::from_raw(repo) };
            let db = match RefdbPtr::of_repo(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            // SAFETY: a string.
            match db.reflog_read(unsafe { CStr::from_ptr(name) }) {
                Ok(log) => {
                    // SAFETY: writable.
                    unsafe { *reflog = log.as_ptr() };
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
    /// See the module.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_write(reflog: *mut GitReflog) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", -1);
            // SAFETY: a live reflog.
            let db = unsafe { (*reflog).db };
            crate::git_assert_arg!(!db.is_null(), "reflog->db", -1);
            // SAFETY: the reflog's database, counted by it.
            let d = unsafe { RefdbPtr::from_raw(db) };
            // SAFETY: a live reflog.
            status(unsafe { d.reflog_write(reflog) })
        })
    }

    /// `git_reflog_append`.
    ///
    /// # Safety
    ///
    /// See the module; `new_oid` and `committer` valid; `msg` NULL or a
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_append(
        reflog: *mut GitReflog,
        new_oid: *const Oid,
        committer: *const git_signature,
        msg: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", -1);
            crate::git_assert_arg!(!new_oid.is_null(), "new_oid", -1);
            crate::git_assert_arg!(!committer.is_null(), "committer", -1);
            // SAFETY: a valid oid; NULL or a string.
            let (id, m) = (unsafe { &*new_oid }, msg.get());
            // SAFETY: a live reflog, exclusively used; a valid signature.
            let r = unsafe { append(reflog, id, committer, m.map(CStr::to_bytes)) };
            match r {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    /// `git_reflog_rename`.
    ///
    /// # Safety
    ///
    /// `repo` live; the names strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_rename(
        repo: *mut git_repository,
        old_name: *const c_char,
        new_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract (C dereferences NULL).
            let repo = unsafe { RepoPtr::from_raw(repo) };
            let db = match refdb_of(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            let b = db.backend();
            // the backend's slot, with C's arguments.
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).reflog_rename }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(b, old_name, new_name) }
            }
        })
    }

    /// `git_reflog_delete`.
    ///
    /// # Safety
    ///
    /// `repo` live; `name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_delete(
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let repo = unsafe { RepoPtr::from_raw(repo) };
            let db = match refdb_of(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            let b = db.backend();
            // as above.
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).reflog_delete }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(b, name) }
            }
        })
    }

    /// `git_reflog_entrycount`.
    ///
    /// # Safety
    ///
    /// See the module.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entrycount(reflog: *mut GitReflog) -> usize {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", 0);
            // SAFETY: a live reflog.
            unsafe { (*reflog).entries.len() }
        })
    }

    /// `git_reflog_entry_byindex`.
    ///
    /// # Safety
    ///
    /// See the module.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_byindex(
        reflog: *const GitReflog,
        idx: usize,
    ) -> *const GitReflogEntry {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", core::ptr::null());
            // SAFETY: a live reflog.
            unsafe { entry_byindex(reflog, idx) }.cast_const()
        })
    }

    /// `git_reflog_entry_id_old`.
    ///
    /// # Safety
    ///
    /// `entry` NULL or a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_id_old(entry: *const GitReflogEntry) -> *const Oid {
        barrier(|| {
            crate::git_assert_arg!(!entry.is_null(), "entry", core::ptr::null());
            // SAFETY: a live entry: a place projection.
            unsafe { &raw const (*entry).oid_old }
        })
    }

    /// `git_reflog_entry_id_new`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_id_new(entry: *const GitReflogEntry) -> *const Oid {
        barrier(|| {
            crate::git_assert_arg!(!entry.is_null(), "entry", core::ptr::null());
            // SAFETY: as above.
            unsafe { &raw const (*entry).oid_cur }
        })
    }

    /// `git_reflog_entry_committer`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_committer(
        entry: *const GitReflogEntry,
    ) -> *const git_signature {
        barrier(|| {
            crate::git_assert_arg!(!entry.is_null(), "entry", core::ptr::null());
            // SAFETY: a live entry.
            unsafe { (*entry).committer }.cast_const()
        })
    }

    /// `git_reflog_entry_message`.
    ///
    /// # Safety
    ///
    /// As [`git_reflog_entry_id_old`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_entry_message(
        entry: *const GitReflogEntry,
    ) -> *const c_char {
        barrier(|| {
            crate::git_assert_arg!(!entry.is_null(), "entry", core::ptr::null());
            // SAFETY: a live entry.
            unsafe { (*entry).msg }.cast_const()
        })
    }

    /// `git_reflog_drop`.
    ///
    /// # Safety
    ///
    /// See the module (C dereferences a NULL reflog after its assertion).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reflog_drop(
        reflog: *mut GitReflog,
        idx: usize,
        rewrite_previous_entry: c_int,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!reflog.is_null(), "reflog", -1);
            // SAFETY: a live reflog, exclusively used.
            status(unsafe { drop_entry(reflog, idx, rewrite_previous_entry != 0) })
        })
    }

    #[allow(unused_imports)]
    use errors as _;
}

crate::c_layout_test! {
    headers: ["reflog.h"],
    types: [
        GitReflogEntry => "git_reflog_entry" { oid_old, oid_cur, committer, msg },
        GitReflog => "git_reflog" { db, ref_name, oid_type, entries },
    ],
}
