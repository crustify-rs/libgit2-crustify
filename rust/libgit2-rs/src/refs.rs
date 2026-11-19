//! Port of `src/libgit2/refs.c`: references (`git_reference`), reference
//! name validation and normalization, and the reference API over the
//! repository's reference database.
//!
//! `git_reference` is mirrored (kind 3): C allocates it
//! (`git_reference__alloc*`, `refdb_reftable.c`), reads its fields
//! everywhere (`ref->type`, `ref->name`, `ref->target.oid`,
//! `ref->target.symbolic`, `ref->peel`, `ref->db`) and frees it with
//! `git_reference_free`. `name` is a flexible array member: a reference is
//! one `git__calloc` block of `sizeof(git_reference) + strlen(name) + 1`
//! bytes, from libgit2's allocator, as in C. The Rust holds one as a
//! [`RefBox`] (the sole owner, which frees as `git_reference_free`) and
//! reads one C lends through a [`RefView`]; both read the fields through
//! the raw pointer, whose provenance covers the name.
//!
//! The safe API is [`Reference`], [`ReferenceType`], and the iterators of
//! [`crate::refdb`]'s safe face.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::git_error_set;
use crate::oid::{Oid, OidType};
use crate::refdb::{self, GitRefdb};
use crate::refs::deps::{self as p, git_repository};
use crate::util::alloc;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::util::{prefixcmp, strcmp};

// ---------------------------------------------------------------------------
// Constants (refs.h, include/git2/types.h, include/git2/refs.h)
// ---------------------------------------------------------------------------

/// `GIT_REFERENCE_INVALID`.
pub const GIT_REFERENCE_INVALID: c_int = 0;
/// `GIT_REFERENCE_DIRECT`.
pub const GIT_REFERENCE_DIRECT: c_int = 1;
/// `GIT_REFERENCE_SYMBOLIC`.
pub const GIT_REFERENCE_SYMBOLIC: c_int = 2;
/// `GIT_REFERENCE_ALL`.
pub const GIT_REFERENCE_ALL: c_int = 3;

/// `GIT_REFERENCE_FORMAT_NORMAL`.
pub const GIT_REFERENCE_FORMAT_NORMAL: c_uint = 0;
/// `GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL`.
pub const GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL: c_uint = 1 << 0;
/// `GIT_REFERENCE_FORMAT_REFSPEC_PATTERN`.
pub const GIT_REFERENCE_FORMAT_REFSPEC_PATTERN: c_uint = 1 << 1;
/// `GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND`.
pub const GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND: c_uint = 1 << 2;
/// `GIT_REFERENCE_FORMAT__PRECOMPOSE_UNICODE`.
pub const GIT_REFERENCE_FORMAT_PRECOMPOSE_UNICODE: c_uint = 1 << 16;
/// `GIT_REFERENCE_FORMAT__VALIDATION_DISABLE`.
pub const GIT_REFERENCE_FORMAT_VALIDATION_DISABLE: c_uint = 1 << 15;

/// `GIT_REFNAME_MAX`.
pub const GIT_REFNAME_MAX: usize = 1024;

/// `GIT_REFS_DIR`.
pub const GIT_REFS_DIR: &[u8] = b"refs/";
/// `GIT_REFS_HEADS_DIR`.
pub const GIT_REFS_HEADS_DIR: &[u8] = b"refs/heads/";
/// `GIT_REFS_TAGS_DIR`.
pub const GIT_REFS_TAGS_DIR: &[u8] = b"refs/tags/";
/// `GIT_REFS_REMOTES_DIR`.
pub const GIT_REFS_REMOTES_DIR: &[u8] = b"refs/remotes/";
/// `GIT_REFS_NOTES_DIR`.
pub const GIT_REFS_NOTES_DIR: &[u8] = b"refs/notes/";
/// `GIT_REFS_DIR_MODE`.
pub const GIT_REFS_DIR_MODE: u32 = 0o777;
/// `GIT_REFS_FILE_MODE`.
pub const GIT_REFS_FILE_MODE: u32 = 0o666;
/// `GIT_SYMREF`.
pub const GIT_SYMREF: &[u8] = b"ref: ";
/// `GIT_PACKEDREFS_FILE`.
pub const GIT_PACKEDREFS_FILE: &[u8] = b"packed-refs";
/// `GIT_PACKEDREFS_HEADER`.
pub const GIT_PACKEDREFS_HEADER: &[u8] = b"# pack-refs with: peeled fully-peeled sorted ";
/// `GIT_PACKEDREFS_FILE_MODE`.
pub const GIT_PACKEDREFS_FILE_MODE: u32 = 0o666;
/// `GIT_HEAD_REF` / `GIT_HEAD_FILE`.
pub const GIT_HEAD_REF: &[u8] = b"HEAD";

/// `git_reference__enable_symbolic_ref_target_validation`: a global that
/// `git_libgit2_opts(GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION)` writes
/// (`settings.c`). A `bool` in C; an `AtomicBool` has its layout.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static git_reference__enable_symbolic_ref_target_validation: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// git_reference
// ---------------------------------------------------------------------------

/// The `target` union of `git_reference`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union RefTarget {
    /// A direct reference's target.
    pub oid: Oid,
    /// A symbolic reference's target, a NUL-terminated `git__strdup`.
    pub symbolic: *mut c_char,
}

/// `struct git_reference`, mirrored (see the [module documentation](self)).
#[repr(C)]
pub struct GitReference {
    /// The reference database, with a count held by the reference (or NULL).
    pub db: *mut GitRefdb,
    /// `git_reference_t`.
    pub kind: c_int,
    /// The target.
    pub target: RefTarget,
    /// The peeled target (zero if unknown).
    pub peel: Oid,
    /// The name, NUL-terminated, in the same allocation.
    pub name: [c_char; 0],
}

/// The kind of a reference (`git_reference_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReferenceType {
    /// A reference to an object id.
    Direct,
    /// A reference to another reference, by name.
    Symbolic,
}

/// The address of `r->name` (no access: plain pointer arithmetic, which
/// keeps `r`'s provenance over the whole block).
fn name_ptr(r: *mut GitReference) -> *mut c_char {
    r.wrapping_byte_add(core::mem::offset_of!(GitReference, name))
        .cast()
}

/// The address of `r->target` (no access).
fn target_ptr(r: *mut GitReference) -> *mut RefTarget {
    r.wrapping_byte_add(core::mem::offset_of!(GitReference, target))
        .cast()
}

/// A borrowed reference, which C (or a [`RefBox`]) owns: the getters.
///
/// Invariant (from [`from_ptr`](Self::from_ptr), the one constructor, or
/// an export's `Option<RefView<'_>>` parameter, whose contract is the
/// same): `ptr` is a live `git_reference` block that nothing writes or
/// frees during `'a`. `#[repr(transparent)]` over the pointer (the
/// `PhantomData` is a ZST), so `Option<RefView<'_>>` has the ABI of a C
/// `const git_reference *` (NULL is `None`).
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct RefView<'a> {
    ptr: NonNull<GitReference>,
    _life: PhantomData<&'a GitReference>,
}

impl<'a> RefView<'a> {
    /// The reference at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` is a live `git_reference` (one block, name included, its
    /// symbolic target a NUL-terminated string if it is symbolic), which
    /// nothing writes or frees during `'a`.
    pub unsafe fn from_ptr(ptr: NonNull<GitReference>) -> RefView<'a> {
        RefView {
            ptr,
            _life: PhantomData,
        }
    }

    /// The reference at `r`, which C dereferences unchecked (the barrier
    /// turns the panic of a NULL into an error).
    ///
    /// # Safety
    ///
    /// As [`from_ptr`](Self::from_ptr), for a non-NULL `r`.
    pub unsafe fn from_raw(r: *const GitReference) -> RefView<'a> {
        RefView {
            ptr: NonNull::new(r.cast_mut()).expect("NULL reference"),
            _life: PhantomData,
        }
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut GitReference {
        self.ptr.as_ptr()
    }

    /// `ref->type`.
    pub fn raw_kind(self) -> c_int {
        // SAFETY: `ptr` is a live block (the view's invariant), aligned
        // (`git__allocator` memory, see `alloc_ref`); `kind` is initialized
        // (zeroed, then set) and nothing writes it during `'a`.
        unsafe { (*self.ptr.as_ptr()).kind }
    }

    /// The kind, if valid.
    pub fn kind(self) -> Option<ReferenceType> {
        match self.raw_kind() {
            GIT_REFERENCE_DIRECT => Some(ReferenceType::Direct),
            GIT_REFERENCE_SYMBOLIC => Some(ReferenceType::Symbolic),
            _ => None,
        }
    }

    /// `ref->name`.
    pub fn name_cstr(self) -> &'a CStr {
        // SAFETY: the name is a NUL-terminated string in the reference's
        // block (`alloc_ref` sizes the block for it), which the pointer's
        // provenance covers (it is the allocation's), and nothing writes or
        // frees it during `'a` (the view's invariant).
        unsafe { CStr::from_ptr(name_ptr(self.ptr.as_ptr())) }
    }

    /// `ref->name`, without the NUL.
    pub fn name(self) -> &'a [u8] {
        self.name_cstr().to_bytes()
    }

    /// `&ref->target.oid` of a direct reference.
    pub fn target(self) -> Option<&'a Oid> {
        if self.raw_kind() != GIT_REFERENCE_DIRECT {
            return None;
        }
        Some(self.oid_field())
    }

    /// `&ref->target.oid`, whatever the kind (as C reads it).
    pub fn oid_field(self) -> &'a Oid {
        // SAFETY: the union lies in the live block (the view's invariant),
        // is at least an `Oid` wide, and `Oid` has align 1; its bytes are
        // initialized (the block is zeroed, then an oid or a pointer is
        // written) and any bytes are a valid `Oid`; nothing writes the
        // block during `'a`, which bounds the result.
        unsafe { &*target_ptr(self.ptr.as_ptr()).cast::<Oid>() }
    }

    /// `ref->target.symbolic` of a symbolic reference.
    pub fn symbolic_target_cstr(self) -> Option<&'a CStr> {
        if self.raw_kind() != GIT_REFERENCE_SYMBOLIC {
            return None;
        }
        // SAFETY: a live symbolic reference (the view's invariant and the
        // check above): the union holds its target pointer, written there by
        // `new_symbolic_cstr`, at an 8-aligned offset of an aligned block.
        let p = unsafe { target_ptr(self.ptr.as_ptr()).cast::<*mut c_char>().read() };
        if p.is_null() {
            return None;
        }
        // SAFETY: a symbolic reference's target is a NUL-terminated
        // `git__strdup` it owns, freed only with the block, which lives
        // unwritten for `'a` (the view's invariant).
        Some(unsafe { CStr::from_ptr(p) })
    }

    /// The symbolic target, without the NUL.
    pub fn symbolic_target(self) -> Option<&'a [u8]> {
        self.symbolic_target_cstr().map(CStr::to_bytes)
    }

    /// `&ref->peel`.
    pub fn peel(self) -> &'a Oid {
        // SAFETY: a live, aligned block (the view's invariant); `peel` is
        // initialized (zeroed or set at creation), unwritten for `'a`.
        unsafe { &(*self.ptr.as_ptr()).peel }
    }

    /// `ref->db`.
    pub fn db(self) -> *mut GitRefdb {
        // SAFETY: as in `peel`, for `db`.
        unsafe { (*self.ptr.as_ptr()).db }
    }
}

/// An owned `git_reference` (`git_reference__alloc*`), freed on drop as
/// `git_reference_free` frees it.
pub struct RefBox {
    ptr: NonNull<GitReference>,
}

// SAFETY: the one field owns the reference's block, which holds:
// - `kind`, `target.oid`, `peel` and `name`: plain data;
// - `target.symbolic`: a `git__strdup` the block owns (`git__allocator`
//   frees from any thread);
// - `db`: NULL or a refdb on which the block holds a count; the count is
//   atomic (`GitRefcount`), so dropping it on another thread is a
//   `GIT_REFCOUNT_DEC` there, as C allows. The refdb's other fields are an
//   atomic backend pointer and a `repo` pointer fixed at creation, which a
//   `RefBox` itself never follows (only code holding a `RepoPtr` does, under
//   that handle's contract).
// Nothing ties the block to the thread that made it.
unsafe impl Send for RefBox {}
// SAFETY: `&RefBox` gives only `view()`, which reads the block; nothing
// writes it while shared (writes need `&mut RefBox` or ownership).
unsafe impl Sync for RefBox {}

impl core::fmt::Debug for RefBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RefBox")
            .field("name", &Bytes(self.view().name()))
            .field("kind", &self.view().raw_kind())
            .finish()
    }
}

/// `alloc_ref`: a zeroed block with room for `name` and its NUL.
fn alloc_ref(name: &[u8]) -> Option<NonNull<GitReference>> {
    let len = core::mem::size_of::<GitReference>()
        .checked_add(name.len())?
        .checked_add(1)?;
    // `git__allocator` returns `malloc`-aligned memory (16 bytes on the
    // supported targets), enough for the struct.
    const _: () = assert!(core::mem::align_of::<GitReference>() <= 16);
    let p = NonNull::new(alloc::calloc(1, len).cast::<GitReference>())?;
    // SAFETY: a fresh zeroed block of `len` bytes: the name's bytes fit
    // after the struct with room left for the NUL (already zero); the
    // source is a distinct live slice.
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), name_ptr(p.as_ptr()).cast::<u8>(), name.len())
    };
    Some(p)
}

/// Bytes without a NUL: a C string's contents, or bytes cut at their first
/// NUL (as C reads them).
#[derive(Clone, Copy, Debug)]
pub struct NulFree<'a>(&'a [u8]);

impl<'a> NulFree<'a> {
    /// `s` up to its first NUL.
    pub fn cut(s: &'a [u8]) -> NulFree<'a> {
        NulFree(crate::util::fs_path::c_str(s))
    }

    /// A C string's bytes (which have no NUL).
    pub fn of(s: &'a CStr) -> NulFree<'a> {
        NulFree(s.to_bytes())
    }

    /// The bytes.
    pub fn bytes(self) -> &'a [u8] {
        self.0
    }
}

impl RefBox {
    /// `git_reference__alloc(name, oid, peel)`: a direct reference with no
    /// database. Fails (NULL in C) only for lack of memory, with the error
    /// set.
    pub fn new_direct(name: &[u8], oid: &Oid, peel: Option<&Oid>) -> Result<RefBox> {
        Self::new_direct_cstr(NulFree::cut(name), oid, peel)
    }

    /// [`new_direct`](Self::new_direct) of a name already without a NUL
    /// (a C string's bytes), which is not scanned again.
    pub fn new_direct_cstr(name: NulFree<'_>, oid: &Oid, peel: Option<&Oid>) -> Result<RefBox> {
        let name = name.0;
        let ptr = alloc_ref(name).ok_or(ErrorCode::GENERIC)?;
        let r = ptr.as_ptr();
        let peel = match peel {
            Some(peel) => *peel,
            None => crate::refdb_fs::oid_clear(c_int::from(oid.raw_kind())),
        };
        // SAFETY: our fresh, zeroed, aligned block (`alloc_ref`), which
        // nothing else references yet.
        unsafe { (*r).kind = GIT_REFERENCE_DIRECT };
        // SAFETY: as above; the union is at least an `Oid` wide and `Oid`
        // has align 1; its other bytes stay zero.
        unsafe { target_ptr(r).cast::<Oid>().write(*oid) };
        // SAFETY: as above.
        unsafe { (*r).peel = peel };
        Ok(RefBox { ptr })
    }

    /// `git_reference__alloc_symbolic(name, target)`.
    pub fn new_symbolic(name: &[u8], target: &[u8]) -> Result<RefBox> {
        Self::new_symbolic_cstr(NulFree::cut(name), NulFree::cut(target))
    }

    /// [`new_symbolic`](Self::new_symbolic) of strings already without a
    /// NUL, which are not scanned again.
    pub fn new_symbolic_cstr(name: NulFree<'_>, target: NulFree<'_>) -> Result<RefBox> {
        let (name, target) = (name.0, target.0);
        let ptr = alloc_ref(name).ok_or(ErrorCode::GENERIC)?;
        let t = alloc::substrdup(target);
        if t.is_null() {
            // SAFETY: our block, from `git__calloc`, not handed out; freed
            // once.
            unsafe { alloc::free(ptr.as_ptr().cast()) };
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: our fresh, zeroed, aligned block (`alloc_ref`), which
        // nothing else references yet.
        unsafe { (*ptr.as_ptr()).kind = GIT_REFERENCE_SYMBOLIC };
        // SAFETY: as above: the union's pointer field, at an 8-aligned
        // offset (the union is `repr(C)` after a pointer and an int).
        unsafe { target_ptr(ptr.as_ptr()).cast::<*mut c_char>().write(t) };
        Ok(RefBox { ptr })
    }

    /// Adopt a reference C made.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live `git_reference` from `git_reference__alloc*`
    /// (or `git_reference_dup`) that nothing else uses or frees.
    pub unsafe fn from_raw(ptr: *mut GitReference) -> Option<RefBox> {
        NonNull::new(ptr).map(|ptr| RefBox { ptr })
    }

    /// Give the reference to C (which frees it with `git_reference_free`).
    pub fn into_raw(self) -> *mut GitReference {
        let p = self.ptr.as_ptr();
        core::mem::forget(self);
        p
    }

    /// The raw pointer, for a C call that does not keep it.
    pub fn as_ptr(&self) -> *mut GitReference {
        self.ptr.as_ptr()
    }

    /// The getters.
    pub fn view(&self) -> RefView<'_> {
        // SAFETY: we own the live block; `&self` keeps it unchanged.
        unsafe { RefView::from_ptr(self.ptr) }
    }

    /// Set `ref->db` to `db`, taking a count on it (`GIT_REFCOUNT_INC(db);
    /// ref->db = db;`). The count of a database set before is not dropped
    /// (a leak, not a dangling pointer), as in C, where these references
    /// come straight from a backend.
    pub fn attach_db(&mut self, db: RefdbPtr) {
        db.incref();
        // SAFETY: we own the live block (`&mut self`: no other reference
        // to it); `db` is aligned and in bounds of the block's header.
        unsafe { (*self.ptr.as_ptr()).db = db.as_ptr() };
    }

    /// `git_reference__realloc(&ref, name)`: give the reference a new name,
    /// in a reallocated block. On failure the reference is consumed (C
    /// leaves `*ptr_to_ref` NULL and the old block to the caller, which
    /// frees it: here, dropped).
    pub fn rename(self, name: &[u8]) -> Result<RefBox> {
        let name = crate::util::fs_path::c_str(name);
        let Some(len) = core::mem::size_of::<GitReference>()
            .checked_add(name.len())
            .and_then(|l| l.checked_add(1))
        else {
            return Err(errors::oom());
        };
        let old = self.ptr.as_ptr();
        // SAFETY: our block (we own `self`), from `git__allocator`; no
        // reference into it is live (`name` cannot borrow `self`, which is
        // moved in). On success the old block is gone and only `p` is used.
        let p = unsafe { alloc::realloc(old.cast(), len) }.cast::<GitReference>();
        let Some(ptr) = NonNull::new(p) else {
            // `self` still owns the old block: dropped as C's caller frees it.
            return Err(ErrorCode::GENERIC);
        };
        core::mem::forget(self);
        let dst = name_ptr(ptr.as_ptr()).cast::<u8>();
        // SAFETY: the reallocated block is `len` bytes: the name's bytes
        // fit after the struct; `name` is a live slice outside it.
        unsafe { core::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len()) };
        // SAFETY: as above: the last byte of the block.
        unsafe { dst.wrapping_add(name.len()).write(0) };
        Ok(RefBox { ptr })
    }

    /// `git_reference_dup`: a copy, with a count on the same database.
    pub fn dup(src: RefView<'_>) -> Result<RefBox> {
        let mut d = match src.symbolic_target() {
            Some(t) if src.raw_kind() == GIT_REFERENCE_SYMBOLIC => {
                RefBox::new_symbolic(src.name(), t)
            }
            _ if src.raw_kind() == GIT_REFERENCE_SYMBOLIC => {
                // C: `git_reference__alloc_symbolic(name, NULL)` asserts
                // `target`, then `GIT_ERROR_CHECK_ALLOC` sets the
                // out-of-memory error.
                let _ = errors::assert_arg_failed("target");
                Err(ErrorCode::GENERIC)
            }
            _ => RefBox::new_direct(src.name(), src.oid_field(), Some(src.peel())),
        }
        .map_err(|_| errors::oom())?;
        // C increments `db` unconditionally (`GIT_REFCOUNT_INC((*dest)->db)`
        // with a NULL `db` crashes in C): here only a set database is
        // counted. The fresh block's `db` is NULL already.
        if let Some(db) = NonNull::new(src.db()) {
            // SAFETY: the source holds a count on its database, which keeps
            // it live while the source is (all of `src`'s lifetime, longer
            // than this call); only its count is touched here.
            d.attach_db(unsafe { RefdbPtr::new(db) });
        }
        Ok(d)
    }
}

impl Drop for RefBox {
    /// `git_reference_free`.
    fn drop(&mut self) {
        // SAFETY: we own the live reference (the constructors' contract),
        // given up here: nothing uses it after the drop.
        unsafe { free_raw(self.ptr.as_ptr()) };
    }
}

/// `git_reference_free` on a raw pointer.
///
/// # Safety
///
/// `r` is NULL or a live reference that the caller owns and gives up.
pub unsafe fn free_raw(r: *mut GitReference) {
    let Some(nn) = NonNull::new(r) else {
        return;
    };
    // SAFETY: a live reference that the caller gives up (the contract), so
    // nothing else writes or frees it; `v` is used only before the frees
    // below.
    let v = unsafe { RefView::from_ptr(nn) };
    let db = v.db();
    if v.raw_kind() == GIT_REFERENCE_SYMBOLIC {
        // SAFETY: as in `RefView::symbolic_target_cstr`: the union holds
        // the target pointer of a symbolic reference.
        let t = unsafe { target_ptr(r).cast::<*mut c_char>().read() };
        // SAFETY: NULL or the reference's own `git__strdup`, freed once (the
        // block, and so the pointer, is freed just after).
        unsafe { alloc::free(t.cast()) };
    }
    if !db.is_null() {
        // SAFETY: the reference holds a count on its live database, given
        // up here.
        unsafe { refdb::decref(db) };
    }
    // SAFETY: the block, from `git__allocator`, freed once; nothing refers
    // to it any more.
    unsafe { alloc::free(r.cast()) };
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// The byte at `i`, or NUL past the end (C reads the terminator).
#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `is_valid_ref_char`: control characters (C's `(unsigned) ch <= ' '`
/// lets the signed bytes >= 0x80 through), DEL and `~^:\?[` are not.
fn is_valid_ref_char(ch: u8) -> bool {
    if ch <= b' ' || ch == 0x7f {
        return false;
    }
    !matches!(ch, b'~' | b'^' | b':' | b'\\' | b'?' | b'[')
}

/// `ensure_segment_validity`: the length of the segment at the start of
/// `name` (up to `/` or the end), or `None` if it is invalid.
fn ensure_segment_validity(
    name: &[u8],
    may_contain_glob: bool,
    allow_caret_prefix: bool,
) -> Option<usize> {
    const LOCK: &[u8] = b".lock";
    let mut may_contain_glob = may_contain_glob;
    if at(name, 0) == b'.' {
        return None;
    }
    let mut i = usize::from(allow_caret_prefix && at(name, 0) == b'^');
    let mut prev = 0u8;
    loop {
        let c = at(name, i);
        if c == 0 || c == b'/' {
            break;
        }
        if !is_valid_ref_char(c) {
            return None;
        }
        if prev == b'.' && c == b'.' {
            return None;
        }
        if prev == b'@' && c == b'{' {
            return None;
        }
        if c == b'*' {
            if !may_contain_glob {
                return None;
            }
            may_contain_glob = false;
        }
        prev = c;
        i += 1;
    }
    if i >= LOCK.len() && &name[i - LOCK.len()..i] == LOCK {
        return None;
    }
    Some(i)
}

/// `is_valid_normalized_name`: all-caps-and-underscores (a root
/// reference such as `HEAD`, `FETCH_HEAD`), or `@`, or with a leading `^`.
fn is_valid_normalized_name(name: &[u8], len: usize) -> bool {
    if len == 0 {
        // GIT_ASSERT_ARG in a function returning `bool`: -1 is true.
        let _ = errors::assert_arg_failed("len > 0");
        return true;
    }
    for i in 0..len {
        let c = at(name, i);
        if i == 0 && c == b'^' {
            continue;
        }
        if len == 1 && c == b'@' {
            return true;
        }
        if !c.is_ascii_uppercase() && c != b'_' {
            return false;
        }
    }
    !(at(name, 0) == b'_' || at(name, len - 1) == b'_')
}

/// `git_reference__normalize_name(buf, name, flags)`: validate `name`
/// (up to its first NUL, as C reads it), and if `buf` is given, write its
/// normalized form (`@` is `HEAD`, empty segments are squashed). An
/// invalid name is `GIT_EINVALIDSPEC` with C's message; on any error `buf`
/// is disposed.
pub fn normalize_name(buf: Option<&mut GitStr>, name: &[u8], flags: c_uint) -> Result<()> {
    normalize_name_nul_free(buf, NulFree::cut(name), flags)
}

/// [`normalize_name`] of a name already without a NUL.
pub fn normalize_name_nul_free(
    buf: Option<&mut GitStr>,
    name: NulFree<'_>,
    flags: c_uint,
) -> Result<()> {
    let name = name.0;
    let mut buf = buf;
    let r = normalize_inner(buf.as_deref_mut(), name, flags);
    if r == Err(ErrorCode::INVALID_SPEC) {
        let _ = git_error_set!(
            ErrorClass::REFERENCE,
            "the given reference name '{}' is not valid",
            Bytes(name)
        );
    }
    if r.is_err()
        && let Some(b) = buf
    {
        b.dispose();
    }
    r
}

fn normalize_inner(mut buf: Option<&mut GitStr>, name: &[u8], flags: c_uint) -> Result<()> {
    let invalid = Err(ErrorCode::INVALID_SPEC);
    let normalize = buf.is_some();
    let validate = flags & GIT_REFERENCE_FORMAT_VALIDATION_DISABLE == 0;
    let mut process_flags = flags;
    let mut allow_caret_prefix = true;
    let mut segments_count = 0usize;

    if validate && at(name, 0) == b'/' {
        return invalid;
    }
    if let Some(b) = buf.as_deref_mut() {
        b.clear();
    }
    // GIT_I18N_ICONV is not defined in this configuration: precomposing
    // (GIT_REFERENCE_FORMAT__PRECOMPOSE_UNICODE) is a no-op, as in C.
    if !validate {
        // C calls `git_str_sets(buf, …)` even when `buf` is NULL (a crash
        // in C; see DEFECTS.md): without a buffer there is nothing to do.
        if let Some(b) = buf {
            let _ = b.sets(name);
            if b.is_oom() {
                return Err(ErrorCode::GENERIC);
            }
        }
        return Ok(());
    }

    let mut current = 0usize;
    let mut segment_len;
    loop {
        let may_contain_glob = process_flags & GIT_REFERENCE_FORMAT_REFSPEC_PATTERN != 0;
        let rest = &name[current.min(name.len())..];
        segment_len = match ensure_segment_validity(rest, may_contain_glob, allow_caret_prefix) {
            Some(n) => n,
            None => return invalid,
        };
        if segment_len > 0 {
            let seg = &rest[..segment_len];
            if seg.contains(&b'*') {
                process_flags &= !GIT_REFERENCE_FORMAT_REFSPEC_PATTERN;
            }
            if let Some(b) = buf.as_deref_mut() {
                if segments_count == 0 && segment_len == 1 && seg[0] == b'@' {
                    let _ = b.sets(GIT_HEAD_REF);
                } else {
                    // git_str_joinpath(buf, buf, current) then truncating
                    // to the segment: a separator unless `buf` is empty.
                    if !b.is_empty() {
                        let _ = b.putc(b'/');
                    }
                    let _ = b.put(seg);
                }
                if b.is_oom() {
                    return Err(ErrorCode::GENERIC);
                }
            }
            segments_count += 1;
        }
        if segment_len == 0 && !normalize {
            return invalid;
        }
        if at(rest, segment_len) == 0 {
            break;
        }
        current += segment_len + 1;
        allow_caret_prefix = false;
    }

    if segment_len == 0 && segments_count == 0 {
        return invalid;
    }
    // `current[segment_len - 1]`: the segment's last byte, or (an empty
    // last segment) the separator before it.
    let last = name[current + segment_len - 1];
    if last == b'.' || last == b'/' {
        return invalid;
    }
    if segments_count == 1 && flags & GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL == 0 {
        return invalid;
    }
    if segments_count == 1
        && flags & GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND == 0
        && !(is_valid_normalized_name(name, segment_len)
            || (flags & GIT_REFERENCE_FORMAT_REFSPEC_PATTERN != 0 && name == b"*"))
    {
        return invalid;
    }
    if segments_count > 1 && flags & GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND == 0 {
        let first = memchr::memchr(b'/', name).unwrap_or(name.len());
        if is_valid_normalized_name(name, first) {
            return invalid;
        }
    }
    Ok(())
}

/// `git_reference__name_is_valid`: whether `name` is valid under `flags`
/// (an invalid name is not an error; other errors are).
pub fn name_is_valid(name: &[u8], flags: c_uint) -> Result<bool> {
    match normalize_name(None, name, flags) {
        Ok(()) => Ok(true),
        Err(ErrorCode::INVALID_SPEC) => Ok(false),
        Err(e) => Err(e),
    }
}

/// `git_reference_normalize_name` into a `GIT_REFNAME_MAX`-style buffer:
/// the normalized name, or `GIT_EBUFS` if it needs more than
/// `buffer_size - 1` bytes.
pub fn normalize_to_buffer(name: &[u8], flags: c_uint, buffer_size: usize) -> Result<GitStr> {
    let mut buf = GitStr::new();
    normalize_name(Some(&mut buf), name, flags)?;
    if buf.len() > buffer_size.wrapping_sub(1) {
        let _ = git_error_set!(
            ErrorClass::REFERENCE,
            "the provided buffer is too short to hold the normalization of '{}'",
            Bytes(crate::util::fs_path::c_str(name))
        );
        return Err(ErrorCode::BUFS);
    }
    Ok(buf)
}

/// `git_reference__is_branch`.
pub fn is_branch(name: &[u8]) -> bool {
    name.starts_with(GIT_REFS_HEADS_DIR)
}
/// `git_reference__is_remote`.
pub fn is_remote(name: &[u8]) -> bool {
    name.starts_with(GIT_REFS_REMOTES_DIR)
}
/// `git_reference__is_tag`.
pub fn is_tag(name: &[u8]) -> bool {
    name.starts_with(GIT_REFS_TAGS_DIR)
}
/// `git_reference__is_note`.
pub fn is_note(name: &[u8]) -> bool {
    name.starts_with(GIT_REFS_NOTES_DIR)
}

/// `git_reference__is_pseudoref`: `MERGE_HEAD` and `FETCH_HEAD`, which are
/// read as loose files whatever the backend.
pub fn is_pseudoref(name: &[u8]) -> bool {
    name == b"MERGE_HEAD" || name == b"FETCH_HEAD"
}

/// `git_reference__is_per_worktree_ref`: references outside `refs/`, and
/// those under `refs/bisect/`, `refs/worktree/` and `refs/rewritten/`.
pub fn is_per_worktree_ref(name: &[u8]) -> bool {
    if !name.starts_with(b"refs/") {
        return true;
    }
    [&b"refs/bisect/"[..], b"refs/worktree/", b"refs/rewritten/"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// `git_reference__shorthand`: the offset of the short name in `name`.
pub fn shorthand_offset(name: &[u8]) -> usize {
    for p in [
        GIT_REFS_HEADS_DIR,
        GIT_REFS_TAGS_DIR,
        GIT_REFS_REMOTES_DIR,
        GIT_REFS_DIR,
    ] {
        if name.starts_with(p) {
            return p.len();
        }
    }
    0
}

/// `git_reference_cmp`: by name, then symbolic before direct, then target.
pub fn cmp(a: RefView<'_>, b: RefView<'_>) -> c_int {
    let r = strcmp(a.name(), b.name());
    if r != 0 {
        return r;
    }
    let (t1, t2) = (a.raw_kind(), b.raw_kind());
    if t1 != t2 {
        return if t1 == GIT_REFERENCE_SYMBOLIC { -1 } else { 1 };
    }
    if t1 == GIT_REFERENCE_SYMBOLIC {
        return strcmp(
            a.symbolic_target().unwrap_or_default(),
            b.symbolic_target().unwrap_or_default(),
        );
    }
    a.oid_field().cmp_raw(b.oid_field())
}

/// `git_reference__lookup_loose(out, ref_dir, ref_name, oid_type)`: read
/// `<ref_dir>/<ref_name>` as a loose reference; with `want == false` only
/// check that it parses (`out == NULL`).
pub fn lookup_loose(
    ref_dir: &[u8],
    ref_name: &[u8],
    oid_type: c_int,
    want: bool,
) -> Result<Option<RefBox>> {
    use crate::util::futils;
    let mut path = GitStr::new();
    path.joinpath(ref_dir, ref_name)?;
    let mut buf = GitStr::new();
    futils::readbuffer(&mut buf, &crate::util::fs_path::str_cpath(&path))?;
    if prefixcmp(buf.as_bytes(), GIT_SYMREF) == 0 {
        buf.rtrim();
        let target = loose_parse_symbolic(buf.as_bytes())?;
        if !want {
            return Ok(None);
        }
        return RefBox::new_symbolic(ref_name, target).map(Some);
    }
    let oid = loose_parse_oid(ref_name, buf.as_bytes(), oid_type)?;
    if !want {
        return Ok(None);
    }
    RefBox::new_direct(ref_name, &oid, None).map(Some)
}

/// `loose_parse_symbolic`: the target after `ref: `.
pub fn loose_parse_symbolic(content: &[u8]) -> Result<&[u8]> {
    if content.len() < GIT_SYMREF.len() + 1 {
        return Err(git_error_set!(
            ErrorClass::REFERENCE,
            "corrupted loose reference file"
        ));
    }
    // C reads up to the NUL of the buffer.
    Ok(crate::util::fs_path::c_str(&content[GIT_SYMREF.len()..]))
}

/// `loose_parse_oid`: a loose file's object id, which must be followed by
/// the end of the string or whitespace.
pub fn loose_parse_oid(filename: &[u8], content: &[u8], oid_type: c_int) -> Result<Oid> {
    let corrupted = || {
        git_error_set!(
            ErrorClass::REFERENCE,
            "corrupted loose reference file: {}",
            Bytes(filename)
        )
    };
    let Some(t) = OidType::from_raw(oid_type as u32) else {
        return Err(corrupted());
    };
    let hexsize = t.hexsize();
    if content.len() < hexsize {
        return Err(corrupted());
    }
    let Ok(oid) = Oid::parse_prefix(&content[..hexsize], t) else {
        return Err(corrupted());
    };
    let next = at(content, hexsize);
    if next == 0 || crate::util::ctype_compat::isspace(next) {
        return Ok(oid);
    }
    Err(corrupted())
}

// ---------------------------------------------------------------------------
// The reference API over a repository
// ---------------------------------------------------------------------------

use crate::refdb::RefdbPtr;
use crate::refs::deps::{ObjBox, RepoPtr, SigBox, SigRef, configmap, git_signature};
use crate::util::fs_path::to_cstring;
use std::ffi::CString;

/// `GIT_OBJECT_ANY`.
const OBJECT_ANY: c_int = -2;
/// `GIT_OBJECT_TAG`.
const OBJECT_TAG: c_int = 4;

/// `reference_normalize_for_repo`: `name` normalized as a reference of
/// `repo` (`core.precomposeUnicode`; `validate == false` only copies it).
pub fn normalize_for_repo(repo: RepoPtr, name: &[u8], validate: bool) -> Result<CString> {
    let mut flags = GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL;
    if matches!(repo.configmap(configmap::PRECOMPOSE), Ok(p) if p != 0) {
        flags |= GIT_REFERENCE_FORMAT_PRECOMPOSE_UNICODE;
    }
    if !validate {
        flags |= GIT_REFERENCE_FORMAT_VALIDATION_DISABLE;
    }
    let buf = normalize_to_buffer(name, flags, GIT_REFNAME_MAX)?;
    Ok(to_cstring(buf.as_bytes()))
}

/// `git_reference_lookup_resolved`: look `name` up, following symbolic
/// references up to `max_nesting` deep; with `max_nesting != 0` an
/// unresolved (dangling) one is `GIT_ENOTFOUND`.
pub fn lookup_resolved(repo: RepoPtr, name: &[u8], max_nesting: c_int) -> Result<RefBox> {
    if is_pseudoref(name) {
        let gitdir = repo.gitdir().unwrap_or_default();
        let r = lookup_loose(&gitdir, name, repo.oid_type(), true)?;
        let mut r = r.ok_or(ErrorCode::GENERIC)?;
        r.attach_db(RefdbPtr::of_repo(repo)?);
        return Ok(r);
    }
    let normalized = normalize_for_repo(repo, name, true)?;
    let db = RefdbPtr::of_repo(repo)?;
    let r = db.resolve(&normalized, max_nesting)?;
    if max_nesting != 0 && r.view().raw_kind() == GIT_REFERENCE_SYMBOLIC {
        return Err(ErrorCode::NOT_FOUND);
    }
    Ok(r)
}

/// `git_reference_lookup`.
pub fn lookup(repo: RepoPtr, name: &[u8]) -> Result<RefBox> {
    lookup_resolved(repo, name, 0)
}

/// `git_reference_name_to_id`.
pub fn name_to_id(repo: RepoPtr, name: &[u8]) -> Result<Oid> {
    let r = lookup_resolved(repo, name, -1)?;
    Ok(*r.view().oid_field())
}

/// `git_reference_dwim`: look a short name up as `git` does (`<name>`,
/// `refs/<name>`, `refs/tags/<name>`, `refs/heads/<name>`,
/// `refs/remotes/<name>`, `refs/remotes/<name>/HEAD`).
pub fn dwim(repo: RepoPtr, refname: &[u8]) -> Result<RefBox> {
    const FORMATS: [(&[u8], &[u8]); 6] = [
        (b"", b""),
        (b"refs/", b""),
        (b"refs/tags/", b""),
        (b"refs/heads/", b""),
        (b"refs/remotes/", b""),
        (b"refs/remotes/", b"/HEAD"),
    ];
    let refname = crate::util::fs_path::c_str(refname);
    let (name, fallback): (&[u8], bool) = if refname.is_empty() {
        (GIT_HEAD_REF, false)
    } else {
        (refname, true)
    };
    let mut found_valid = false;
    let mut error = ErrorCode::GENERIC;
    let mut result = None;
    for (i, (pre, post)) in FORMATS.iter().enumerate() {
        if !(fallback || i == 0) {
            break;
        }
        let mut full = Vec::with_capacity(pre.len() + name.len() + post.len());
        full.extend_from_slice(pre);
        full.extend_from_slice(name);
        full.extend_from_slice(post);
        match name_is_valid(&full, GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL) {
            Err(e) => {
                error = e;
                break;
            }
            Ok(false) => {
                error = ErrorCode::INVALID_SPEC;
                continue;
            }
            Ok(true) => {}
        }
        found_valid = true;
        match lookup_resolved(repo, &full, -1) {
            Ok(r) => {
                result = Some(r);
                break;
            }
            Err(e) => {
                error = e;
                if e != ErrorCode::NOT_FOUND {
                    break;
                }
            }
        }
    }
    if let Some(r) = result {
        return Ok(r);
    }
    if !found_valid {
        let _ = git_error_set!(
            ErrorClass::REFERENCE,
            "could not use '{}' as valid reference name",
            Bytes(name)
        );
    }
    if error == ErrorCode::NOT_FOUND {
        let _ = git_error_set!(
            ErrorClass::REFERENCE,
            "no reference found for shorthand '{}'",
            Bytes(refname)
        );
    }
    Err(error)
}

/// What a new reference points to.
#[derive(Clone, Copy)]
pub enum NewTarget<'a> {
    /// An object id.
    Oid(&'a Oid),
    /// Another reference's name.
    Symbolic(&'a [u8]),
}

/// `reference__create`: write a new reference `name` to `target`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create(
    repo: RepoPtr,
    name: &[u8],
    target: NewTarget<'_>,
    force: bool,
    signature: Option<SigRef<'_>>,
    log_message: Option<&CStr>,
    old_id: Option<&Oid>,
    old_target: Option<&CStr>,
) -> Result<RefBox> {
    crate::git_assert_arg!(
        matches!(target, NewTarget::Symbolic(_)) || signature.is_some(),
        "symbolic || signature"
    );
    let normalized = normalize_for_repo(repo, name, true)?;
    let db = RefdbPtr::of_repo(repo)?;
    let r = match target {
        NewTarget::Oid(oid) => {
            if !crate::refs::deps::object_is_valid(repo, oid, OBJECT_ANY) {
                return Err(git_error_set!(
                    ErrorClass::REFERENCE,
                    "target OID for the reference doesn't exist on the repository"
                ));
            }
            RefBox::new_direct(normalized.to_bytes(), oid, None)
        }
        NewTarget::Symbolic(t) => {
            let validate =
                git_reference__enable_symbolic_ref_target_validation.load(Ordering::Relaxed);
            let nt = normalize_for_repo(repo, t, validate)?;
            RefBox::new_symbolic(normalized.to_bytes(), nt.to_bytes())
        }
    }
    .map_err(|_| errors::oom())?;
    let mut r = r;
    db.write(&mut r, force, signature, log_message, old_id, old_target)?;
    Ok(r)
}

/// `git_reference__log_signature`: the signature reflog entries use
/// (`user.name`/`user.email` as configured on the repository, else the
/// default signature, else `unknown`).
pub fn log_signature(repo: RepoPtr) -> Result<SigBox> {
    if let (Some(n), Some(e)) = repo.ident()
        && let Ok(s) = SigBox::now(&to_cstring(&n), &to_cstring(&e))
    {
        return Ok(s);
    }
    if let Ok(s) = SigBox::default_for(repo) {
        return Ok(s);
    }
    SigBox::now(c"unknown", c"unknown")
}

/// `git_reference_create_matching`.
pub fn create_matching(
    repo: RepoPtr,
    name: &[u8],
    id: &Oid,
    force: bool,
    old_id: Option<&Oid>,
    log_message: Option<&CStr>,
) -> Result<RefBox> {
    let who = log_signature(repo)?;
    create(
        repo,
        name,
        NewTarget::Oid(id),
        force,
        Some(who.sig()),
        log_message,
        old_id,
        None,
    )
}

/// `git_reference_symbolic_create_matching`.
pub fn symbolic_create_matching(
    repo: RepoPtr,
    name: &[u8],
    target: &[u8],
    force: bool,
    old_target: Option<&CStr>,
    log_message: Option<&CStr>,
) -> Result<RefBox> {
    let who = log_signature(repo)?;
    create(
        repo,
        name,
        NewTarget::Symbolic(target),
        force,
        Some(who.sig()),
        log_message,
        None,
        old_target,
    )
}

/// The repository of a reference (`ref->db->repo`), for use while the
/// reference is.
pub fn owner(r: RefView<'_>) -> Result<RepoPtr> {
    let db = NonNull::new(r.db()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: `RefdbPtr::new`'s conditions, for the uses of the result
    // while `r` is used: the reference holds a count on its database (so
    // it and its backend are live: only `db->repo`, fixed at creation, is
    // read); its repository, if set, is live too, because a reference is
    // not used after its repository is freed (`git_repository_free`'s
    // contract for the objects it spawned, which `RefView`'s carries).
    let db = unsafe { RefdbPtr::new(db) };
    db.repo().ok_or(ErrorCode::GENERIC)
}

/// `git_reference_set_target`.
pub fn set_target(r: RefView<'_>, id: &Oid, log_message: Option<&CStr>) -> Result<RefBox> {
    let repo = owner(r)?;
    if r.raw_kind() != GIT_REFERENCE_DIRECT {
        return Err(git_error_set!(
            ErrorClass::REFERENCE,
            "cannot set OID on symbolic reference"
        ));
    }
    let old = *r.oid_field();
    create_matching(repo, r.name(), id, true, Some(&old), log_message)
}

/// `git_reference_symbolic_set_target`.
pub fn symbolic_set_target(
    r: RefView<'_>,
    target: &[u8],
    log_message: Option<&CStr>,
) -> Result<RefBox> {
    if r.raw_kind() != GIT_REFERENCE_SYMBOLIC {
        return Err(git_error_set!(
            ErrorClass::REFERENCE,
            "cannot set symbolic target on a direct reference"
        ));
    }
    let repo = owner(r)?;
    let old = r.symbolic_target_cstr().map(CStr::to_owned);
    symbolic_create_matching(repo, r.name(), target, true, old.as_deref(), log_message)
}

struct UpdateHead<'a> {
    old_name: &'a [u8],
    new_name: &'a [u8],
}

/// `refs_update_head`: point the HEAD of `worktree` at the renamed
/// reference.
fn update_head(worktree: RepoPtr, p: &UpdateHead<'_>) -> c_int {
    let head = match lookup(worktree, GIT_HEAD_REF) {
        Ok(h) => h,
        Err(e) => return e.raw(),
    };
    let v = head.view();
    if v.raw_kind() != GIT_REFERENCE_SYMBOLIC
        || strcmp(v.symbolic_target().unwrap_or_default(), p.old_name) != 0
    {
        return 0;
    }
    match symbolic_set_target(v, p.new_name, None) {
        Ok(_) => 0,
        Err(e) => {
            let _ = git_error_set!(
                ErrorClass::REFERENCE,
                "failed to update HEAD after renaming reference"
            );
            e.raw()
        }
    }
}

/// `git_reference_rename`: the renamed reference (written to `out` before
/// the worktrees' HEADs are updated, as C).
pub fn rename(
    r: RefView<'_>,
    new_name: &[u8],
    force: bool,
    log_message: Option<&CStr>,
    out: &mut Option<RefBox>,
) -> Result<()> {
    let repo = owner(r)?;
    let signature = log_signature(repo)?;
    let normalized = normalize_for_repo(repo, new_name, true)?;
    let db = NonNull::new(r.db()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the reference holds a count on its database, which keeps it
    // live while `r` is (all of this call), and `owner` above found its
    // repository, whose refdb this is (its backend is live by the
    // contract of the `RepoPtr` that made `r`).
    let db = unsafe { RefdbPtr::new(db) };
    *out = db.rename(
        r.name_cstr(),
        &normalized,
        force,
        Some(signature.sig()),
        log_message,
        true,
    )?;
    let payload = UpdateHead {
        old_name: r.name(),
        new_name: normalized.to_bytes(),
    };
    // `git_repository_foreach_worktree(repo, refs_update_head, &payload)`.
    let e = crate::repository::head::foreach_worktree(repo.get(), &mut |w| {
        // SAFETY: `foreach_worktree` lends `w`, open for this call of the
        // closure; the handle and the references `update_head` gets
        // through it are dropped before it returns.
        let wt = unsafe { RepoPtr::new(NonNull::from(w)) };
        update_head(wt, &payload)
    });
    ErrorCode::check(e).map(drop)
}

/// `git_reference_resolve`.
pub fn resolve(r: RefView<'_>) -> Result<RefBox> {
    match r.raw_kind() {
        GIT_REFERENCE_DIRECT => lookup(owner(r)?, r.name()),
        GIT_REFERENCE_SYMBOLIC => {
            lookup_resolved(owner(r)?, r.symbolic_target().unwrap_or_default(), -1)
        }
        _ => Err(git_error_set!(ErrorClass::REFERENCE, "invalid reference")),
    }
}

/// `git_reference__update_terminal`: follow `ref_name` to its direct
/// reference and set it to `oid` (creating it if it is missing).
pub(crate) fn update_terminal(
    repo: RepoPtr,
    ref_name: &[u8],
    oid: &Oid,
    sig: Option<SigRef<'_>>,
    log_message: Option<&CStr>,
) -> Result<()> {
    let who;
    let to_use = match sig {
        Some(s) => Some(s),
        None => {
            who = log_signature(repo)?;
            Some(who.sig())
        }
    };
    let db = RefdbPtr::of_repo(repo)?;
    let r = match db.resolve(&to_cstring(ref_name), -1) {
        Ok(r) => r,
        Err(ErrorCode::NOT_FOUND) => {
            errors::clear();
            return create(
                repo,
                ref_name,
                NewTarget::Oid(oid),
                false,
                to_use,
                log_message,
                None,
                None,
            )
            .map(drop);
        }
        Err(e) => return Err(e),
    };
    let v = r.view();
    if v.raw_kind() == GIT_REFERENCE_SYMBOLIC {
        let target = v.symbolic_target().unwrap_or_default().to_vec();
        create(
            repo,
            &target,
            NewTarget::Oid(oid),
            false,
            to_use,
            log_message,
            None,
            None,
        )
        .map(drop)
    } else {
        let old = *v.oid_field();
        create(
            repo,
            v.name(),
            NewTarget::Oid(oid),
            true,
            to_use,
            log_message,
            Some(&old),
            None,
        )
        .map(drop)
    }
}

/// `git_reference__update_for_commit`.
pub fn update_for_commit(
    repo: RepoPtr,
    r: Option<RefView<'_>>,
    ref_name: &[u8],
    id: &Oid,
    operation: Option<&[u8]>,
) -> Result<()> {
    let commit = p::CommitBox::lookup(repo, id)?;
    let kind: &[u8] = match commit.parentcount() {
        0 => b" (initial)",
        1 => b"",
        _ => b" (merge)",
    };
    let mut msg = operation.unwrap_or(b"commit").to_vec();
    msg.extend_from_slice(kind);
    msg.extend_from_slice(b": ");
    msg.extend_from_slice(&commit.summary().unwrap_or_else(|| b"(null)".to_vec()));
    let msg = to_cstring(&msg);
    let who = commit.committer();
    match r {
        Some(r) => {
            if r.raw_kind() != GIT_REFERENCE_DIRECT {
                return Err(git_error_set!(
                    ErrorClass::REFERENCE,
                    "cannot set OID on symbolic reference"
                ));
            }
            let old = *r.oid_field();
            create(
                repo,
                r.name(),
                NewTarget::Oid(id),
                true,
                who,
                Some(&msg),
                Some(&old),
                None,
            )
            .map(drop)
        }
        None => update_terminal(repo, ref_name, id, who, Some(&msg)),
    }
}

/// `peel_error`.
fn peel_error(error: ErrorCode, r: RefView<'_>, msg: &str) -> ErrorCode {
    let _ = git_error_set!(
        ErrorClass::INVALID,
        "the reference '{}' cannot be peeled - {}",
        Bytes(r.name()),
        msg
    );
    error
}

/// `git_reference_peel`: the object `r` peels to, of `target_type`.
pub fn peel(r: RefView<'_>, target_type: c_int) -> Result<ObjBox> {
    let allocated;
    let resolved = if r.raw_kind() == GIT_REFERENCE_DIRECT {
        r
    } else {
        allocated = resolve(r).map_err(|e| peel_error(e, r, "Cannot resolve reference"))?;
        allocated.view()
    };
    let repo = owner(r)?;
    let id = if target_type != OBJECT_TAG && !resolved.peel().is_zero() {
        resolved.peel()
    } else {
        resolved.oid_field()
    };
    let target = ObjBox::lookup(repo, id, OBJECT_ANY)
        .map_err(|e| peel_error(e, r, "Cannot retrieve reference target"))?;
    if target_type == OBJECT_ANY && target.kind() != OBJECT_TAG {
        target.dup()
    } else {
        target.peel(target_type)
    }
}

/// `git_reference__is_unborn_head`.
pub fn is_unborn_head(r: RefView<'_>, repo: RepoPtr) -> Result<bool> {
    if r.raw_kind() == GIT_REFERENCE_DIRECT {
        return Ok(false);
    }
    match lookup_resolved(repo, r.name(), -1) {
        Ok(_) => Ok(false),
        Err(ErrorCode::NOT_FOUND) => Ok(strcmp(r.name(), GIT_HEAD_REF) == 0),
        Err(e) => Err(e),
    }
}

/// A reference iterator (`git_reference_iterator_new` / `_glob_new`),
/// freed on drop.
pub struct RefIter(NonNull<crate::refdb::git_reference_iterator>);

// SAFETY: the one field owns a backend iterator, used only through
// `&mut self` (one thread at a time). What it reaches:
// - `db`: a refdb on which it holds a count (atomic `GitRefcount`), dropped
//   by `Drop` on whichever thread drops the value;
// - the iterator's own state: for the crate's backends an `FsIter` or
//   `VecIter` (both `Send`: they own their pools and vectors, and the
//   `Arc` of the shared backend state); for a C backend (reftable, a
//   user's) C state that libgit2 lets any one thread use at a time (the
//   backend API has no thread affinity), as C callers move iterators too.
// Not `Sync`: `&RefIter` gives nothing, so no claim is needed.
unsafe impl Send for RefIter {}

impl RefIter {
    /// `git_reference_iterator_new` / `git_reference_iterator_glob_new`.
    pub fn new(repo: RepoPtr, glob: Option<&CStr>) -> Result<RefIter> {
        let db = RefdbPtr::of_repo(repo).map_err(|_| ErrorCode::GENERIC)?;
        db.iterator(glob).map(RefIter)
    }

    /// `git_reference_next`.
    pub fn next_ref(&mut self) -> Result<RefBox> {
        // SAFETY: our live iterator from `git_refdb_iterator`
        // (`RefdbPtr::iterator`), used by nobody else during the call
        // (`&mut self`).
        unsafe { refdb::iterator_next(self.0.as_ptr()) }
    }

    /// `git_reference_next_name`: the name, valid until the next call.
    pub fn next_name(&mut self) -> Result<&CStr> {
        let mut out = core::ptr::null();
        // SAFETY: as in `next_ref`; `out` is our writable local.
        ErrorCode::check(unsafe { refdb::iterator_next_name(&raw mut out, self.0.as_ptr()) })?;
        if out.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: the backend's name, valid until the iterator advances or
        // is freed: both need `&mut self`, which this borrow holds.
        Ok(unsafe { CStr::from_ptr(out) })
    }

    /// Give the iterator to C.
    pub fn into_raw(self) -> *mut crate::refdb::git_reference_iterator {
        let p = self.0.as_ptr();
        core::mem::forget(self);
        p
    }
}

impl Drop for RefIter {
    fn drop(&mut self) {
        // SAFETY: our live iterator from `git_refdb_iterator`, given up
        // here (freed once).
        unsafe { refdb::iterator_free(self.0.as_ptr()) };
    }
}

/// `git_reference_foreach_name` / `_glob`: call `cb` with each name; a
/// non-zero return stops and is returned (with `action`'s callback error).
pub fn foreach_name(
    repo: RepoPtr,
    glob: Option<&CStr>,
    action: &str,
    cb: &mut dyn FnMut(&CStr) -> c_int,
) -> Result<()> {
    let mut it = RefIter::new(repo, glob)?;
    loop {
        match it.next_name() {
            Ok(n) => {
                let e = cb(n);
                if e != 0 {
                    errors::set_after_callback(e, action);
                    return ErrorCode::check_zero(e);
                }
            }
            Err(ErrorCode::ITER_OVER) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

pub(crate) mod deps;

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `refs.c`.
///
/// Each export's `# Safety` lists its pointers. Throughout:
///
/// - a "live repository" is a `git_repository` (from this crate) that stays
///   open for the call; its refdb (and the refdb's backend) are not
///   replaced or freed during the call (`git_repository_set_refdb`,
///   `git_refdb_set_backend` race with every use, as in C);
/// - a "live reference" is a `git_reference` block from
///   `git_reference__alloc*` / a backend, not freed during the call; where
///   the export reaches the repository through it, its `db` is a live
///   database with a live backend;
/// - strings are NUL-terminated and stay readable for the call;
/// - libgit2's threading rules hold: one thread uses a reference or an
///   iterator at a time; a repository may be shared.
///
/// Exports whose raw signature other modules call keep raw pointer
/// parameters and convert them once; the others take [`Option<RepoPtr>`],
/// [`Option<RefView<'_>>`](RefView), [`CStrArg`] and [`Out`], which have the
/// ABI of the C pointers.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::refdb::git_reference_iterator;
    use crate::refs::deps::git_object;
    use crate::strarray::GitStrArray;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};
    use crate::util::vector::GitVector;

    /// The first bytes of the name `p` (at most 32, which is more than any
    /// prefix or root name the predicates compare with), without measuring
    /// the whole string: C's `git__prefixcmp` reads only to the mismatch.
    /// NULL (which C dereferences) reads as the empty name.
    fn head<'a>(p: CStrArg<'a>) -> &'a [u8] {
        const HEAD_MAX: usize = 32;
        if p.is_null() {
            return &[];
        }
        let p = p.as_ptr();
        // SAFETY: a non-null `CStrArg` is a NUL-terminated string readable
        // for its lifetime (the type's invariant): `strnlen` reads to its
        // NUL or 32 bytes, whichever comes first.
        let n = unsafe { libc::strnlen(p, HEAD_MAX) };
        // SAFETY: those `n` (<= 32) bytes precede the NUL in the same
        // string, readable and unwritten for the `CStrArg`'s lifetime,
        // which bounds the result's.
        unsafe { crate::util::export::bytes(p, n) }
    }

    /// The repository, checked as `GIT_ASSERT_ARG(repo)`.
    fn repo_arg(repo: Option<RepoPtr>) -> Result<RepoPtr> {
        repo.ok_or_else(|| errors::assert_arg_failed("repo"))
    }

    /// A repository argument C dereferences unchecked (the barrier turns
    /// the panic of a NULL into an error), for the exports whose raw
    /// signature other modules call.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository that stays live for the export's
    /// call (the export's contract), during which the result is used.
    unsafe fn repo_unchecked(repo: *mut git_repository) -> RepoPtr {
        // SAFETY: the caller's guarantee above is `from_raw`'s, for the
        // call (NULL panics in `from_raw`).
        unsafe { RepoPtr::from_raw(repo) }
    }

    /// A reference argument C reads, for the exports whose raw signature
    /// other modules call.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference that nothing writes or frees during
    /// `'a` (the export's call).
    unsafe fn view<'a>(r: *const GitReference) -> Option<RefView<'a>> {
        // SAFETY: the caller's guarantee is `from_ptr`'s for `'a`, and
        // `NonNull::new` excludes NULL.
        NonNull::new(r.cast_mut()).map(|p| unsafe { RefView::from_ptr(p) })
    }

    /// Write a reference result to `out` (NULL on failure: C sets it on
    /// most failure paths, and leaves it as it was on a few).
    fn put(mut out: Out<'_, *mut GitReference>, r: Result<RefBox>) -> c_int {
        match r {
            Ok(r) => {
                out.write(r.into_raw());
                0
            }
            Err(e) => {
                out.write(core::ptr::null_mut());
                e.raw()
            }
        }
    }

    /// `git_reference__alloc_symbolic`.
    ///
    /// # Safety
    ///
    /// `name` and `target` are NULL or NUL-terminated strings, readable
    /// and unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__alloc_symbolic(
        name: CStrArg<'_>,
        target: CStrArg<'_>,
    ) -> *mut GitReference {
        barrier(|| {
            crate::git_assert_arg!(!name.is_null(), "name", core::ptr::null_mut());
            crate::git_assert_arg!(!target.is_null(), "target", core::ptr::null_mut());
            let (n, t) = (name.to_bytes(), target.to_bytes());
            RefBox::new_symbolic_cstr(NulFree(n), NulFree(t))
                .map_or(core::ptr::null_mut(), RefBox::into_raw)
        })
    }

    /// `git_reference__alloc`.
    ///
    /// # Safety
    ///
    /// `name` is NULL or a NUL-terminated string; `oid` and `peel` are NULL
    /// or valid `git_oid`s; none is written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__alloc(
        name: CStrArg<'_>,
        oid: Option<&Oid>,
        peel: Option<&Oid>,
    ) -> *mut GitReference {
        barrier(|| {
            crate::git_assert_arg!(!name.is_null(), "name", core::ptr::null_mut());
            let o = git_arg!(oid, "oid", core::ptr::null_mut());
            RefBox::new_direct_cstr(NulFree(name.to_bytes()), o, peel)
                .map_or(core::ptr::null_mut(), RefBox::into_raw)
        })
    }

    /// `git_reference__realloc`.
    ///
    /// # Safety
    ///
    /// `ptr_to_ref` is NULL, or aligned, readable and writable, holding
    /// NULL or a live reference (from `git_reference__alloc*`) that the
    /// caller owns and nothing else uses during the call; `name` is NULL or
    /// a NUL-terminated string (it may be inside that reference).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__realloc(
        ptr_to_ref: *mut *mut GitReference,
        name: CStrArg<'_>,
    ) -> *mut GitReference {
        barrier(|| {
            crate::git_assert_arg!(!ptr_to_ref.is_null(), "ptr_to_ref", core::ptr::null_mut());
            crate::git_assert_arg!(!name.is_null(), "name", core::ptr::null_mut());
            // SAFETY: `ptr_to_ref` is non-null (checked), aligned and
            // readable (this export's contract).
            let old = unsafe { *ptr_to_ref };
            // SAFETY: as above, writable; no reference to `*ptr_to_ref`
            // exists.
            unsafe { *ptr_to_ref = core::ptr::null_mut() };
            // The name may be the reference's own name (or its target),
            // which the reallocation frees: copied first, while the block
            // is still live (see DEFECTS.md).
            let n = name.to_bytes().to_vec();
            // SAFETY: `old` is NULL or a live reference the caller owns and
            // gives up here (this export's contract); `*ptr_to_ref` no
            // longer points to it.
            let Some(b) = (unsafe { RefBox::from_raw(old) }) else {
                return core::ptr::null_mut();
            };
            // On failure C leaves the old block with the caller, NULL in
            // `*ptr_to_ref`: it leaks (see DEFECTS.md); here `rename`
            // frees it.
            b.rename(&n).map_or(core::ptr::null_mut(), RefBox::into_raw)
        })
    }

    /// `git_reference_dup`.
    ///
    /// # Safety
    ///
    /// `dest` is NULL or writable (NULL is skipped, where C crashes);
    /// `source` is NULL or a live reference, unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_dup(
        dest: Out<'_, *mut GitReference>,
        source: Option<RefView<'_>>,
    ) -> c_int {
        barrier(|| {
            let src = git_arg!(source, "source", -1);
            put(dest, RefBox::dup(src))
        })
    }

    /// `git_reference_free`.
    ///
    /// # Safety
    ///
    /// `reference` is NULL or a live reference (from this module or a
    /// backend) that the caller owns and gives up: nothing uses it after
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_free(reference: *mut GitReference) {
        // SAFETY: `reference` is NULL or a live reference the caller gives
        // up (this export's contract), as `free_raw` requires.
        barrier(|| unsafe { free_raw(reference) })
    }

    /// `git_reference_delete`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten and not freed during the
    /// call, whose `db` (if set) is a live database with a live backend.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_delete(r: *mut GitReference) -> c_int {
        // SAFETY: this export's contract is `view`'s, for the call.
        barrier(|| delete_c(unsafe { view(r) }))
    }

    /// The body of [`git_reference_delete`].
    fn delete_c(r: Option<RefView<'_>>) -> c_int {
        let v = git_arg!(r, "ref", -1);
        if v.name() == b"HEAD" {
            let _ = git_error_set!(ErrorClass::REFERENCE, "cannot delete HEAD");
            return -1;
        }
        let (old_id, old_target) = if v.raw_kind() == GIT_REFERENCE_DIRECT {
            (Some(v.oid_field()), None)
        } else {
            (None, v.symbolic_target_cstr())
        };
        let Some(db) = NonNull::new(v.db()) else {
            return errors::assert_arg_failed("db").raw();
        };
        // SAFETY: the reference holds a count on its database, which keeps
        // it live for the call (during which `v` is live); its backend is
        // live (the export's contract).
        status(unsafe { RefdbPtr::new(db) }.delete(v.name_cstr(), old_id, old_target))
    }

    /// `git_reference_remove`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `name` NULL
    /// or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_remove(
        repo: Option<RepoPtr>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let db = match RefdbPtr::of_repo(repo) {
                Ok(d) => d,
                Err(e) => return e.raw(),
            };
            status(db.delete(name.or_empty(), None, None))
        })
    }

    /// `git_reference_lookup`.
    ///
    /// # Safety
    ///
    /// As [`git_reference_lookup_resolved`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_lookup(
        ref_out: Out<'_, *mut GitReference>,
        repo: *mut git_repository,
        name: CStrArg<'_>,
    ) -> c_int {
        // SAFETY: this export's contract is `git_reference_lookup_resolved`'s.
        unsafe { git_reference_lookup_resolved(ref_out, repo, name, 0) }
    }

    /// `git_reference_lookup_resolved`.
    ///
    /// # Safety
    ///
    /// `ref_out` is NULL or writable; `repo` NULL or a live repository
    /// (see the module); `name` NULL or a NUL-terminated string, unwritten
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_lookup_resolved(
        ref_out: Out<'_, *mut GitReference>,
        repo: *mut git_repository,
        name: CStrArg<'_>,
        max_nesting: c_int,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!ref_out.is_null(), "ref_out", -1);
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            crate::git_assert_arg!(!name.is_null(), "name", -1);
            // SAFETY: a live repository (non-null, checked; this export's
            // contract) for the call.
            let repo = unsafe { repo_unchecked(repo) };
            // C sets `*ref_out` to NULL in the lookups (all but a name or
            // repository error, after which it is left as it was): NULL
            // here on any error.
            put(ref_out, lookup_resolved(repo, name.to_bytes(), max_nesting))
        })
    }

    /// `git_reference_name_to_id`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable for an `Oid`; `repo` NULL or a live
    /// repository (see the module); `name` NULL or a NUL-terminated string,
    /// unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_name_to_id(
        out: *mut Oid,
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int {
        // SAFETY: `out` NULL or writable, with no reference to `*out`
        // (this export's contract).
        let mut out = unsafe { Out::from_ptr(out) };
        // SAFETY: `name` NULL or a string unwritten during the call.
        let name = unsafe { CStrArg::from_ptr(name) };
        barrier(|| {
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            crate::git_assert_arg!(!name.is_null(), "name", -1);
            // SAFETY: a live repository (non-null, checked) for the call.
            let repo = unsafe { repo_unchecked(repo) };
            match name_to_id(repo, name.to_bytes()) {
                Ok(id) => {
                    out.write(id);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference_dwim`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository (see
    /// the module); `refname` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_dwim(
        out: Out<'_, *mut GitReference>,
        repo: Option<RepoPtr>,
        refname: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            put(out, dwim(repo, refname.to_bytes()))
        })
    }

    /// `git_reference_type`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_type(r: *const GitReference) -> c_int {
        barrier(|| {
            // SAFETY: this export's contract is `view`'s, for the call.
            match unsafe { view(r) } {
                Some(v) => v.raw_kind(),
                None => errors::assert_arg_failed("ref").raw(),
            }
        })
    }

    /// `git_reference_name`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference; the result lives as long as it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_name(r: *const GitReference) -> *const c_char {
        barrier(|| {
            crate::git_assert_arg!(!r.is_null(), "ref", core::ptr::null());
            name_ptr(r.cast_mut()).cast_const()
        })
    }

    /// `git_reference_owner`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call, whose
    /// `db` is a live database (C dereferences a NULL one).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_owner(r: Option<RefView<'_>>) -> *mut git_repository {
        barrier(|| {
            let v = git_arg!(r, "ref", core::ptr::null_mut());
            // C dereferences a NULL `db`: the panic becomes an error.
            let db = NonNull::new(v.db()).expect("reference without a database");
            // SAFETY: the reference holds a count on its database, live
            // for the call (this export's contract); only `repo`, fixed
            // once made, is read.
            let db = unsafe { RefdbPtr::new(db) };
            db.repo().map_or(core::ptr::null_mut(), RepoPtr::as_ptr)
        })
    }

    /// `git_reference_target`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call; the
    /// result lives as long as it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_target(r: *const GitReference) -> *const Oid {
        barrier(|| {
            crate::git_assert_arg!(!r.is_null(), "ref", core::ptr::null());
            // SAFETY: this export's contract is `view`'s, for the call.
            let v = unsafe { view(r) };
            v.and_then(RefView::target)
                .map_or(core::ptr::null(), |o| o as *const Oid)
        })
    }

    /// `git_reference_target_peel`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call; the
    /// result lives as long as it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_target_peel(r: Option<RefView<'_>>) -> *const Oid {
        barrier(|| {
            let v = git_arg!(r, "ref", core::ptr::null());
            if v.raw_kind() != GIT_REFERENCE_DIRECT || v.peel().is_zero() {
                return core::ptr::null();
            }
            v.peel() as *const Oid
        })
    }

    /// `git_reference_symbolic_target`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call; the
    /// result lives as long as it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_symbolic_target(
        r: *const GitReference,
    ) -> *const c_char {
        barrier(|| {
            crate::git_assert_arg!(!r.is_null(), "ref", core::ptr::null());
            // SAFETY: this export's contract is `view`'s, for the call.
            let v = unsafe { view(r) };
            v.and_then(RefView::symbolic_target_cstr)
                .map_or(core::ptr::null(), CStr::as_ptr)
        })
    }

    /// `git_reference_create_matching`.
    ///
    /// # Safety
    ///
    /// `ref_out` is NULL or writable; `repo` NULL or a live repository
    /// (see the module); `name` and `log_message` NULL or NUL-terminated
    /// strings; `id` and `old_id` NULL or valid `git_oid`s; none of the
    /// inputs is written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_create_matching(
        mut ref_out: Out<'_, *mut GitReference>,
        repo: Option<RepoPtr>,
        name: CStrArg<'_>,
        id: Option<&Oid>,
        force: c_int,
        old_id: Option<&Oid>,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let id = git_arg!(id, "id", -1);
            ref_out.write(core::ptr::null_mut());
            let r = (|| {
                let repo = repo_arg(repo)?;
                crate::git_assert_arg!(!name.is_null(), "name");
                let msg = log_message.get();
                create_matching(repo, name.to_bytes(), id, force != 0, old_id, msg)
            })();
            match r {
                Ok(r) if !ref_out.is_null() => {
                    ref_out.write(r.into_raw());
                    0
                }
                Ok(_) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference_create`.
    ///
    /// # Safety
    ///
    /// As [`git_reference_create_matching`] (`repo`, `name` and `id` raw
    /// pointers with that contract).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_create(
        ref_out: *mut *mut GitReference,
        repo: *mut git_repository,
        name: *const c_char,
        id: *const Oid,
        force: c_int,
        log_message: CStrArg<'_>,
    ) -> c_int {
        // SAFETY: `ref_out` NULL or writable with no reference to its
        // pointee (this export's contract).
        let ref_out = unsafe { Out::from_ptr(ref_out) };
        // SAFETY: `repo` NULL or a live repository for the call (as above).
        let repo = NonNull::new(repo).map(|r| unsafe { RepoPtr::new(r) });
        // SAFETY: `name` NULL or a string unwritten during the call.
        let name = unsafe { CStrArg::from_ptr(name) };
        // SAFETY: `id` NULL or a valid oid unwritten during the call.
        let id = unsafe { id.as_ref() };
        // SAFETY: the arguments as `git_reference_create_matching` takes
        // them, with this export's (identical) contract.
        unsafe { git_reference_create_matching(ref_out, repo, name, id, force, None, log_message) }
    }

    /// `git_reference_symbolic_create_matching`.
    ///
    /// # Safety
    ///
    /// `ref_out` is NULL or writable; `repo` NULL or a live repository
    /// (see the module); the strings NULL or NUL-terminated, unwritten
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_symbolic_create_matching(
        mut ref_out: Out<'_, *mut GitReference>,
        repo: Option<RepoPtr>,
        name: CStrArg<'_>,
        target: CStrArg<'_>,
        force: c_int,
        old_target: CStrArg<'_>,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!target.is_null(), "target", -1);
            ref_out.write(core::ptr::null_mut());
            let r = (|| {
                let repo = repo_arg(repo)?;
                crate::git_assert_arg!(!name.is_null(), "name");
                let (n, t) = (name.to_bytes(), target.to_bytes());
                let (old, msg) = (old_target.get(), log_message.get());
                symbolic_create_matching(repo, n, t, force != 0, old, msg)
            })();
            match r {
                Ok(r) if !ref_out.is_null() => {
                    ref_out.write(r.into_raw());
                    0
                }
                Ok(_) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference_symbolic_create`.
    ///
    /// # Safety
    ///
    /// As [`git_reference_symbolic_create_matching`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_symbolic_create(
        ref_out: Out<'_, *mut GitReference>,
        repo: Option<RepoPtr>,
        name: CStrArg<'_>,
        target: CStrArg<'_>,
        force: c_int,
        log_message: CStrArg<'_>,
    ) -> c_int {
        // SAFETY: the arguments as `git_reference_symbolic_create_matching`
        // takes them, with this export's (identical) contract.
        unsafe {
            git_reference_symbolic_create_matching(
                ref_out,
                repo,
                name,
                target,
                force,
                CStrArg::NULL,
                log_message,
            )
        }
    }

    /// `git_reference_set_target`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `r` NULL or a live reference, unwritten
    /// during the call, whose `db` is live (see the module); `id` NULL or
    /// a valid `git_oid`; `log_message` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_set_target(
        out: Out<'_, *mut GitReference>,
        r: Option<RefView<'_>>,
        id: Option<&Oid>,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            let v = git_arg!(r, "ref", -1);
            let id = git_arg!(id, "id", -1);
            put(out, set_target(v, id, log_message.get()))
        })
    }

    /// `git_reference_symbolic_set_target`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `r` NULL or a live reference, unwritten
    /// during the call, whose `db` is live (see the module); the strings
    /// NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_symbolic_set_target(
        out: Out<'_, *mut GitReference>,
        r: Option<RefView<'_>>,
        target: CStrArg<'_>,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            let v = git_arg!(r, "ref", -1);
            crate::git_assert_arg!(!target.is_null(), "target", -1);
            put(
                out,
                symbolic_set_target(v, target.to_bytes(), log_message.get()),
            )
        })
    }

    /// `git_reference_rename`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `r` NULL or a live reference, unwritten
    /// and not freed during the call, whose `db` is live (see the module);
    /// the strings NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_rename(
        mut out: Out<'_, *mut GitReference>,
        r: Option<RefView<'_>>,
        new_name: CStrArg<'_>,
        force: c_int,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            let v = git_arg!(r, "ref", -1);
            let mut made = None;
            let (n, msg) = (new_name.to_bytes(), log_message.get());
            let res = rename(v, n, force != 0, msg, &mut made);
            if let Some(m) = made {
                out.write(m.into_raw());
            }
            status(res)
        })
    }

    /// `git_reference_resolve`.
    ///
    /// # Safety
    ///
    /// `ref_out` is NULL or writable; `r` NULL or a live reference,
    /// unwritten during the call, whose `db` is live (see the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_resolve(
        ref_out: Out<'_, *mut GitReference>,
        r: Option<RefView<'_>>,
    ) -> c_int {
        barrier(|| {
            let v = git_arg!(r, "ref", -1);
            put(ref_out, resolve(v))
        })
    }

    /// The C callback of `git_reference_foreach`.
    pub type ForeachCb = unsafe extern "C" fn(*mut GitReference, *mut c_void) -> c_int;
    /// The C callback of `git_reference_foreach_name` / `_glob`.
    pub type ForeachNameCb = unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int;

    /// `git_reference_foreach`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `callback` a
    /// function that may be called with a reference it then owns (and
    /// frees) and `payload`, which is passed through unread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_foreach(
        repo: Option<RepoPtr>,
        callback: ForeachCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let mut it = match RefIter::new(repo, None) {
                Ok(it) => it,
                Err(e) => return e.raw(),
            };
            loop {
                match it.next_ref() {
                    Ok(r) => {
                        // SAFETY: C's callback with the arguments its
                        // contract names: a live reference it now owns and
                        // the caller's payload.
                        let e = unsafe { callback(r.into_raw(), payload) };
                        if e != 0 {
                            return errors::set_after_callback(e, "git_reference_foreach");
                        }
                    }
                    Err(ErrorCode::ITER_OVER) => return 0,
                    Err(e) => return e.raw(),
                }
            }
        })
    }

    /// The body of the `foreach_name` exports: `callback(name, payload)`
    /// for each name of `glob` (all for `None`).
    ///
    /// # Safety
    ///
    /// `callback` may be called with a NUL-terminated name valid during
    /// the call, and `payload` (passed through unread).
    unsafe fn foreach_name_c(
        repo: RepoPtr,
        glob: Option<&CStr>,
        action: &str,
        callback: ForeachNameCb,
        payload: *mut c_void,
    ) -> c_int {
        let mut it = match RefIter::new(repo, glob) {
            Ok(it) => it,
            Err(e) => return e.raw(),
        };
        loop {
            match it.next_name() {
                Ok(n) => {
                    // SAFETY: the callback with the arguments this
                    // function's contract allows: a name valid during the
                    // call (the iterator's, which `it` keeps until the
                    // next step) and the payload.
                    let e = unsafe { callback(n.as_ptr(), payload) };
                    if e != 0 {
                        return errors::set_after_callback(e, action);
                    }
                }
                Err(ErrorCode::ITER_OVER) => return 0,
                Err(e) => return e.raw(),
            }
        }
    }

    /// `git_reference_foreach_name`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `callback` a
    /// function that may be called with a name valid during the call and
    /// `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_foreach_name(
        repo: *mut git_repository,
        callback: ForeachNameCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `repo` NULL (a panic, as C crashes) or a live
            // repository for the call (this export's contract).
            let repo = unsafe { repo_unchecked(repo) };
            let action = "git_reference_foreach_name";
            // SAFETY: `callback` and `payload` as this export's contract
            // allows them.
            unsafe { foreach_name_c(repo, None, action, callback, payload) }
        })
    }

    /// `git_reference_foreach_glob`.
    ///
    /// # Safety
    ///
    /// As [`git_reference_foreach_name`]; `glob` NULL or a NUL-terminated
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_foreach_glob(
        repo: Option<RepoPtr>,
        glob: CStrArg<'_>,
        callback: ForeachNameCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let action = "git_reference_foreach_glob";
            // SAFETY: `callback` and `payload` as this export's contract
            // allows them.
            unsafe { foreach_name_c(repo, glob.get(), action, callback, payload) }
        })
    }

    /// `git_reference_iterator_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository (see
    /// the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_iterator_new(
        out: Out<'_, *mut git_reference_iterator>,
        repo: Option<RepoPtr>,
    ) -> c_int {
        barrier(|| iterator_glob_new_c(out, git_arg!(repo, "repo", -1), CStrArg::NULL))
    }

    /// `git_reference_iterator_glob_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository (see
    /// the module); `glob` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_iterator_glob_new(
        out: *mut *mut git_reference_iterator,
        repo: *mut git_repository,
        glob: CStrArg<'_>,
    ) -> c_int {
        // SAFETY: `out` NULL or writable with no reference to its pointee
        // (this export's contract).
        let out = unsafe { Out::from_ptr(out) };
        barrier(|| {
            // SAFETY: `repo` NULL (a panic, as C crashes) or a live
            // repository for the call (this export's contract).
            let repo = unsafe { repo_unchecked(repo) };
            iterator_glob_new_c(out, repo, glob)
        })
    }

    /// The body of the iterator constructors.
    fn iterator_glob_new_c(
        mut out: Out<'_, *mut git_reference_iterator>,
        repo: RepoPtr,
        glob: CStrArg<'_>,
    ) -> c_int {
        let g = glob.get();
        let db = match RefdbPtr::of_repo(repo) {
            Ok(d) => d,
            Err(_) => return -1,
        };
        match db.iterator(g) {
            Ok(it) => {
                out.write(it.as_ptr());
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_reference_next`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_iterator_next`](crate::refdb::ffi::git_refdb_iterator_next).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_next(
        out: *mut *mut GitReference,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        // SAFETY: this export's contract is `git_refdb_iterator_next`'s.
        unsafe { crate::refdb::ffi::git_refdb_iterator_next(out, iter) }
    }

    /// `git_reference_next_name`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_iterator_next_name`](crate::refdb::ffi::git_refdb_iterator_next_name).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_next_name(
        out: *mut *const c_char,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        // SAFETY: this export's contract is `git_refdb_iterator_next_name`'s.
        unsafe { crate::refdb::ffi::git_refdb_iterator_next_name(out, iter) }
    }

    /// `git_reference_iterator_free`.
    ///
    /// # Safety
    ///
    /// `iter` is NULL or a live iterator from `git_refdb_iterator` that the
    /// caller gives up: nothing uses it after the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_iterator_free(iter: *mut git_reference_iterator) {
        if iter.is_null() {
            return;
        }
        // SAFETY: a non-null `iter` is a live iterator given up (this
        // export's contract), as `git_refdb_iterator_free` takes it.
        unsafe { crate::refdb::ffi::git_refdb_iterator_free(iter) }
    }

    /// `git_reference_list`.
    ///
    /// # Safety
    ///
    /// `array` is NULL or writable for a `git_strarray` (its old contents
    /// are not read); `repo` NULL or a live repository (see the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_list(
        mut array: Out<'_, GitStrArray>,
        repo: Option<RepoPtr>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!array.is_null(), "array", -1);
            let repo = git_arg!(repo, "repo", -1);
            array.write(GitStrArray {
                strings: core::ptr::null_mut(),
                count: 0,
            });
            let Ok(mut list) = GitVector::with_capacity(8, None) else {
                return -1;
            };
            let mut cb = |n: &CStr| -> c_int {
                let s = alloc::strdup(n);
                if s.is_null() {
                    return -1;
                }
                match list.insert(s.cast()) {
                    Ok(()) => 0,
                    Err(e) => {
                        // SAFETY: our `git__strdup` string, not inserted,
                        // freed once.
                        unsafe { alloc::free(s.cast()) };
                        e.raw()
                    }
                }
            };
            if foreach_name(repo, None, "git_reference_foreach_name", &mut cb).is_err() {
                // C leaks the names here (DEFECTS.md); they are freed.
                for s in list.iter() {
                    // SAFETY: each element is one of our `git__strdup`
                    // strings, freed once (the vector is dropped after).
                    unsafe { alloc::free(s) };
                }
                return -1;
            }
            let (ptr, len, _) = list.detach();
            // The strings and the array are from `git__allocator`, as
            // `git_strarray_dispose` frees them.
            array.write(GitStrArray {
                strings: ptr.cast(),
                count: len,
            });
            0
        })
    }

    /// `git_reference__normalize_name`.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_str` that nothing else uses during the
    /// call; `name` NULL or a NUL-terminated string, which may lie in
    /// `buf`'s buffer.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__normalize_name(
        buf: *mut GitStr,
        name: *const c_char,
        flags: c_uint,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!name.is_null(), "name", -1);
            // SAFETY: `buf` is NULL or a valid, aligned `git_str` that
            // nothing else uses during the call (this export's contract):
            // the `&mut` covers its header; `name` may point into its heap
            // buffer, which the header only points to.
            let mut b = unsafe { buf.as_mut() };
            let in_buf = b.as_deref().is_some_and(|s| s.points_into(name));
            let validate = flags & GIT_REFERENCE_FORMAT_VALIDATION_DISABLE == 0;
            let n = match b.as_deref_mut() {
                Some(s) if in_buf => {
                    // C checks the leading '/' on the name as given, then
                    // clears `buf` (which may cut a name that starts at its
                    // buffer), then reads the name: so here, with a copy.
                    // SAFETY: `name` is a NUL-terminated string readable now
                    // (this export's contract); the reference ends with the
                    // read of its first byte.
                    let first = unsafe { name.cast::<u8>().read() };
                    if !(validate && first == b'/') {
                        s.clear();
                    }
                    // SAFETY: as above, read now and copied (it lies in
                    // `s`'s buffer).
                    unsafe { s.arg_bytes(name) }
                }
                _ => {
                    // SAFETY: a NUL-terminated string outside `buf`'s buffer,
                    // which nothing writes during the call (this export's
                    // contract).
                    let c: &CStr = unsafe { CStr::from_ptr(name) };
                    Some(std::borrow::Cow::Borrowed(c.to_bytes()))
                }
            };
            let n = n.unwrap_or_default();
            status(normalize_name_nul_free(b, NulFree(&n), flags))
        })
    }

    /// `git_reference_normalize_name`.
    ///
    /// # Safety
    ///
    /// `buffer_out` is NULL or writable for `buffer_size` bytes (which may
    /// be uninitialized), not overlapping `name`; `name` NULL or a
    /// NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_normalize_name(
        buffer_out: *mut c_char,
        buffer_size: usize,
        name: CStrArg<'_>,
        flags: c_uint,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!name.is_null(), "name", -1);
            let buf = match normalize_to_buffer(name.to_bytes(), flags, buffer_size) {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            // `git_str_copy_cstr`, written through the raw pointer: the
            // caller's buffer may be uninitialized (a `git_refname_t` on
            // the stack), so no `&mut [u8]` is made over it.
            crate::git_assert_arg!(!buffer_out.is_null(), "data", -1);
            crate::git_assert_arg!(buffer_size != 0, "datasize", -1);
            let out = buffer_out.cast::<u8>();
            // SAFETY: `out` is non-null (checked) and writable for
            // `buffer_size` (> 0, checked) bytes (this export's contract);
            // a `u8` needs no alignment.
            unsafe { out.write(0) };
            if buf.is_empty() || buf.capacity() == 0 {
                return 0;
            }
            let src = buf.as_bytes();
            let n = src.len().min(buffer_size - 1);
            // SAFETY: `n < buffer_size` bytes of the caller's writable
            // buffer, which cannot overlap our fresh `buf`.
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), out, n) };
            // SAFETY: `n < buffer_size`, so in the writable range.
            unsafe { out.wrapping_add(n).write(0) };
            0
        })
    }

    /// `git_reference_cmp`.
    ///
    /// # Safety
    ///
    /// `ref1` and `ref2` are NULL or live references, unwritten during the
    /// call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_cmp(
        ref1: Option<RefView<'_>>,
        ref2: Option<RefView<'_>>,
    ) -> c_int {
        barrier(|| {
            let a = git_arg!(ref1, "ref1", -1);
            let b = git_arg!(ref2, "ref2", -1);
            cmp(a, b)
        })
    }

    /// `git_reference__cmp_cb`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are NULL or point to live references, unwritten during
    /// the call (a `git_vector` of references passes its elements).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__cmp_cb(a: *const c_void, b: *const c_void) -> c_int {
        // SAFETY: each pointer is NULL or a live reference unwritten during
        // the call (this export's contract), as `view` requires.
        let ra = unsafe { view(a.cast()) };
        // SAFETY: as above.
        let rb = unsafe { view(b.cast()) };
        // SAFETY: `git_reference_cmp`'s contract, as just established.
        unsafe { git_reference_cmp(ra, rb) }
    }

    /// `git_reference__update_terminal`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `oid` NULL or
    /// a valid `git_oid`; `sig` NULL or a valid signature (its strings
    /// NUL-terminated); the strings NULL or NUL-terminated; none is written
    /// or freed during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__update_terminal(
        repo: Option<RepoPtr>,
        ref_name: CStrArg<'_>,
        oid: Option<&Oid>,
        sig: *const git_signature,
        log_message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let oid = git_arg!(oid, "oid", -1);
            let repo = git_arg!(repo, "repo", -1);
            // SAFETY: `sig` is NULL or a valid signature that nothing
            // writes or frees during the call (this export's contract).
            let sig = unsafe { SigRef::from_raw(sig) };
            let r = update_terminal(repo, ref_name.to_bytes(), oid, sig, log_message.get());
            status(r)
        })
    }

    /// `git_reference__update_for_commit`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `r` NULL or a
    /// live reference, unwritten during the call; `id` NULL or a valid
    /// `git_oid`; the strings NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__update_for_commit(
        repo: *mut git_repository,
        r: *mut GitReference,
        ref_name: CStrArg<'_>,
        id: *const Oid,
        operation: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `repo` NULL (a panic, as C crashes) or a live
            // repository for the call (this export's contract).
            let repo = unsafe { repo_unchecked(repo) };
            // SAFETY: this export's contract on `r` is `view`'s.
            let v = unsafe { view(r) };
            // SAFETY: `id` NULL or a valid oid unwritten during the call;
            // C dereferences NULL: the panic becomes an error.
            let id = unsafe { id.as_ref() }.expect("NULL id");
            // SAFETY: `operation` NULL or a string unwritten during the
            // call.
            let op = unsafe { CStrArg::from_ptr(operation) };
            let res = update_for_commit(
                repo,
                v,
                ref_name.to_bytes(),
                id,
                op.get().map(CStr::to_bytes),
            );
            status(res)
        })
    }

    /// `git_reference_has_log`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository (see the module); `refname`
    /// NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_has_log(
        repo: Option<RepoPtr>,
        refname: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let name = git_arg!(refname.get(), "refname", -1);
            match RefdbPtr::of_repo(repo) {
                Ok(db) => match db.has_log_raw(name) {
                    Ok(v) => v,
                    Err(e) => e.raw(),
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference_ensure_log`.
    ///
    /// # Safety
    ///
    /// As [`git_reference_has_log`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_ensure_log(
        repo: Option<RepoPtr>,
        refname: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let name = git_arg!(refname.get(), "refname", -1);
            match RefdbPtr::of_repo(repo) {
                Ok(db) => match db.ensure_log(name) {
                    Ok(v) => v,
                    Err(e) => e.raw(),
                },
                Err(e) => e.raw(),
            }
        })
    }

    macro_rules! name_predicates {
        ($($raw:ident, $pub:ident, $f:path;)*) => {$(
            #[doc = concat!("`", stringify!($raw), "`.")]
            ///
            /// # Safety
            ///
            /// `ref_name` is NULL or a NUL-terminated string.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $raw(ref_name: CStrArg<'_>) -> c_int {
                barrier(|| c_int::from($f(head(ref_name))))
            }

            #[doc = concat!("`", stringify!($pub), "`.")]
            ///
            /// # Safety
            ///
            /// `r` is NULL or a live reference, unwritten during the call.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $pub(r: Option<RefView<'_>>) -> c_int {
                barrier(|| match r {
                    Some(v) => c_int::from($f(v.name())),
                    None => errors::assert_arg_failed("ref").raw(),
                })
            }
        )*};
    }

    name_predicates! {
        git_reference__is_branch, git_reference_is_branch, is_branch;
        git_reference__is_remote, git_reference_is_remote, is_remote;
        git_reference__is_tag, git_reference_is_tag, is_tag;
        git_reference__is_note, git_reference_is_note, is_note;
    }

    /// `git_reference__is_pseudoref`.
    ///
    /// # Safety
    ///
    /// `ref_name` is NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__is_pseudoref(ref_name: CStrArg<'_>) -> c_int {
        barrier(|| c_int::from(is_pseudoref(head(ref_name))))
    }

    /// `git_reference__is_per_worktree_ref`.
    ///
    /// # Safety
    ///
    /// `ref_name` is NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__is_per_worktree_ref(ref_name: CStrArg<'_>) -> c_int {
        barrier(|| c_int::from(is_per_worktree_ref(head(ref_name))))
    }

    /// `git_reference_peel`.
    ///
    /// # Safety
    ///
    /// `peeled` is NULL or writable; `r` NULL or a live reference,
    /// unwritten during the call, whose `db` is live (see the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_peel(
        peeled: *mut *mut git_object,
        r: *const GitReference,
        target_type: c_int,
    ) -> c_int {
        // SAFETY: `peeled` NULL or writable, with no reference to its
        // pointee (this export's contract).
        let mut peeled = unsafe { Out::from_ptr(peeled) };
        barrier(|| {
            // SAFETY: this export's contract on `r` is `view`'s.
            let Some(v) = (unsafe { view(r) }) else {
                return errors::assert_arg_failed("ref").raw();
            };
            match peel(v, target_type) {
                Ok(o) => {
                    peeled.write(o.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference__name_is_valid`.
    ///
    /// # Safety
    ///
    /// `valid` is NULL or writable; `refname` NULL or a NUL-terminated
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__name_is_valid(
        mut valid: Out<'_, c_int>,
        refname: CStrArg<'_>,
        flags: c_uint,
    ) -> c_int {
        barrier(|| {
            if valid.is_null() || refname.is_null() {
                return errors::assert_failed("valid && refname").raw();
            }
            valid.write(0);
            match name_is_valid(refname.to_bytes(), flags) {
                Ok(v) => {
                    valid.write(c_int::from(v));
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference_name_is_valid`.
    ///
    /// # Safety
    ///
    /// As [`git_reference__name_is_valid`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_name_is_valid(
        valid: *mut c_int,
        refname: *const c_char,
    ) -> c_int {
        // SAFETY: `valid` NULL or writable with no reference to it (this
        // export's contract).
        let valid = unsafe { Out::from_ptr(valid) };
        // SAFETY: `refname` NULL or a string unwritten during the call.
        let refname = unsafe { CStrArg::from_ptr(refname) };
        // SAFETY: the arguments as `git_reference__name_is_valid` takes them.
        unsafe { git_reference__name_is_valid(valid, refname, GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL) }
    }

    /// `git_reference__shorthand`.
    ///
    /// # Safety
    ///
    /// `name` is NULL or a NUL-terminated string; the result points into
    /// it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__shorthand(name: CStrArg<'_>) -> *const c_char {
        barrier(|| {
            let n = head(name);
            name.as_ptr().wrapping_add(shorthand_offset(n))
        })
    }

    /// `git_reference_shorthand`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live reference, unwritten during the call; the
    /// result points into its name.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference_shorthand(r: Option<RefView<'_>>) -> *const c_char {
        barrier(|| {
            // C dereferences NULL.
            let Some(v) = r else {
                return core::ptr::null();
            };
            let name = v.name_cstr().as_ptr();
            name.wrapping_add(shorthand_offset(v.name()))
        })
    }

    /// `git_reference__is_unborn_head`.
    ///
    /// # Safety
    ///
    /// `unborn` is NULL or writable; `r` NULL or a live reference,
    /// unwritten during the call; `repo` NULL or a live repository (see
    /// the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__is_unborn_head(
        mut unborn: Out<'_, bool>,
        r: Option<RefView<'_>>,
        repo: Option<RepoPtr>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!unborn.is_null(), "unborn", -1);
            let v = git_arg!(r, "ref", -1);
            let repo = git_arg!(repo, "repo", -1);
            match is_unborn_head(v, repo) {
                Ok(u) => {
                    unborn.write(u);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference__log_signature`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository (see
    /// the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__log_signature(
        mut out: Out<'_, *mut git_signature>,
        repo: Option<RepoPtr>,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            match log_signature(repo) {
                Ok(s) => {
                    out.write(s.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["refs.h"],
    types: [
        GitReference => "git_reference" { db, kind as "type", target, peel },
    ],
    asserts: [
        "offsetof(git_reference, name) == 89",
        "GIT_REFERENCE_DIRECT == 1",
        "GIT_REFERENCE_SYMBOLIC == 2",
        "GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL == 1",
        "GIT_REFERENCE_FORMAT_REFSPEC_PATTERN == 2",
        "GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND == 4",
        "GIT_REFERENCE_FORMAT__PRECOMPOSE_UNICODE == (1u << 16)",
        "GIT_REFERENCE_FORMAT__VALIDATION_DISABLE == (1u << 15)",
        "sizeof(git_reference__enable_symbolic_ref_target_validation) == 1",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support;

    fn norm(name: &str, flags: c_uint) -> core::result::Result<String, (ErrorCode, Vec<u8>)> {
        let mut buf = GitStr::new();
        match normalize_name(Some(&mut buf), name.as_bytes(), flags) {
            Ok(()) => Ok(String::from_utf8(buf.as_bytes().to_vec()).unwrap()),
            Err(e) => {
                assert!(buf.is_empty());
                Err((e, errors::last_message().unwrap_or_default()))
            }
        }
    }

    const ONE: c_uint = GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL;
    const PAT: c_uint = GIT_REFERENCE_FORMAT_REFSPEC_PATTERN;
    const SHORT: c_uint = GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND;

    #[test]
    fn normalize() {
        let _g = test_support::init();
        assert_eq!(norm("refs/heads/master", 0).unwrap(), "refs/heads/master");
        assert_eq!(
            norm("refs///heads//master", 0).unwrap(),
            "refs/heads/master"
        );
        assert_eq!(norm("@", ONE).unwrap(), "HEAD");
        assert_eq!(norm("@/x", SHORT).unwrap(), "HEAD/x");
        assert_eq!(norm("HEAD", ONE).unwrap(), "HEAD");
        assert_eq!(norm("FETCH_HEAD", ONE).unwrap(), "FETCH_HEAD");
        assert_eq!(norm("refs/heads/*", PAT).unwrap(), "refs/heads/*");
        assert_eq!(norm("*", ONE | PAT).unwrap(), "*");
        assert_eq!(norm("master", ONE | SHORT).unwrap(), "master");
        assert_eq!(norm("^refs/heads/x", 0).unwrap(), "^refs/heads/x");
        assert_eq!(norm("refs/heads/\u{e9}", 0).unwrap(), "refs/heads/\u{e9}");
        for (bad, flags) in [
            ("", ONE),
            ("/refs/heads/x", 0),
            ("refs/heads/x/", 0),
            ("refs/heads/x.", 0),
            ("refs/heads/.x", 0),
            ("refs/heads/a..b", 0),
            ("refs/heads/x.lock", 0),
            ("refs/heads/a@{b", 0),
            ("refs/heads/a b", 0),
            ("refs/heads/a~", 0),
            ("refs/heads/a\u{7f}", 0),
            ("refs/heads/*/*", PAT),
            ("refs/heads/*", 0),
            ("HEAD", 0),
            ("lower", ONE),
            ("_HEAD", ONE),
            ("HEAD_", ONE),
            ("HEAD/x", 0),
            ("refs/^x", 0),
        ] {
            let (e, msg) = norm(bad, flags).unwrap_err();
            assert_eq!(e, ErrorCode::INVALID_SPEC, "{bad:?}");
            assert_eq!(
                String::from_utf8(msg).unwrap(),
                format!("the given reference name '{bad}' is not valid")
            );
        }
        // Without a buffer, an empty segment is invalid.
        assert_eq!(name_is_valid(b"refs//heads/x", 0), Ok(false));
        assert_eq!(name_is_valid(b"refs/heads/x", 0), Ok(true));
        // Validation disabled copies the name, buffer or not.
        assert_eq!(
            norm("not valid..", GIT_REFERENCE_FORMAT_VALIDATION_DISABLE).unwrap(),
            "not valid.."
        );
        assert_eq!(
            name_is_valid(b"..", GIT_REFERENCE_FORMAT_VALIDATION_DISABLE),
            Ok(true)
        );
        // The bytes after a NUL are not read (C reads a C string).
        assert_eq!(norm("refs/heads/x\0..", 0).unwrap(), "refs/heads/x");
    }

    #[test]
    fn buffers() {
        let _g = test_support::init();
        let b = normalize_to_buffer(b"refs/heads/abc", 0, 15).unwrap();
        assert_eq!(b.as_bytes(), b"refs/heads/abc");
        assert_eq!(
            normalize_to_buffer(b"refs/heads/abc", 0, 14).unwrap_err(),
            ErrorCode::BUFS
        );
        assert_eq!(
            errors::last_message().unwrap(),
            b"the provided buffer is too short to hold the normalization of 'refs/heads/abc'"
        );
    }

    #[test]
    fn predicates() {
        assert!(is_branch(b"refs/heads/x") && !is_branch(b"refs/tags/x"));
        assert!(
            is_tag(b"refs/tags/x") && is_remote(b"refs/remotes/o/x") && is_note(b"refs/notes/x")
        );
        assert!(
            is_pseudoref(b"FETCH_HEAD") && is_pseudoref(b"MERGE_HEAD") && !is_pseudoref(b"HEAD")
        );
        for (n, w) in [
            ("HEAD", true),
            ("ORIG_HEAD", true),
            ("refs/bisect/x", true),
            ("refs/worktree/x", true),
            ("refs/rewritten/x", true),
            ("refs/heads/x", false),
            ("refs/bisectx", false),
        ] {
            assert_eq!(is_per_worktree_ref(n.as_bytes()), w, "{n}");
        }
        for (n, s) in [
            ("refs/heads/a/b", "a/b"),
            ("refs/tags/t", "t"),
            ("refs/remotes/o/m", "o/m"),
            ("refs/notes/n", "notes/n"),
            ("HEAD", "HEAD"),
        ] {
            assert_eq!(
                &n.as_bytes()[shorthand_offset(n.as_bytes())..],
                s.as_bytes()
            );
        }
    }

    #[test]
    fn boxes() {
        let _g = test_support::init();
        let id =
            Oid::parse_hex(b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
        let d = RefBox::new_direct(b"refs/heads/x", &id, None).unwrap();
        let v = d.view();
        assert_eq!(
            (v.name(), v.kind(), v.target()),
            (&b"refs/heads/x"[..], Some(ReferenceType::Direct), Some(&id))
        );
        assert!(v.peel().is_zero() && v.peel().kind() == Some(OidType::Sha1));
        assert!(v.symbolic_target().is_none() && v.db().is_null());
        let s = RefBox::new_symbolic(b"HEAD", b"refs/heads/x").unwrap();
        assert_eq!(s.view().symbolic_target(), Some(&b"refs/heads/x"[..]));
        assert!(s.view().target().is_none());
        assert!(cmp(s.view(), d.view()) < 0);
        let s2 = RefBox::new_symbolic(b"HEAD", b"refs/heads/y").unwrap();
        assert!(cmp(s.view(), s2.view()) < 0);
        let s3 = RefBox::new_symbolic(b"HEAD", b"refs/heads/x").unwrap();
        assert_eq!(cmp(s.view(), s3.view()), 0);
        let dd = RefBox::dup(d.view()).unwrap();
        assert_eq!(cmp(dd.view(), d.view()), 0);
        let long = d
            .rename(b"refs/heads/a-much-longer-name-than-before")
            .unwrap();
        assert_eq!(
            long.view().name(),
            b"refs/heads/a-much-longer-name-than-before"
        );
        assert_eq!(long.view().target(), Some(&id));
        let short = long.rename(b"x").unwrap();
        assert_eq!(short.view().name(), b"x");
        let raw = short.into_raw();
        // SAFETY: the reference we just gave up.
        let back = unsafe { RefBox::from_raw(raw) }.unwrap();
        assert_eq!(back.view().name(), b"x");
    }

    #[test]
    fn loose_files() {
        let _g = test_support::init();
        const A: &[u8] = b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
        let sha1 = OidType::Sha1 as c_int;
        assert!(loose_parse_oid(b"r", A, sha1).is_ok());
        let mut with_nl = A.to_vec();
        with_nl.extend_from_slice(b"\n");
        assert!(loose_parse_oid(b"r", &with_nl, sha1).is_ok());
        let mut bad = A.to_vec();
        bad.push(b'x');
        assert_eq!(
            loose_parse_oid(b"r", &bad, sha1).unwrap_err(),
            ErrorCode::GENERIC
        );
        assert_eq!(
            errors::last_message().unwrap(),
            b"corrupted loose reference file: r"
        );
        assert!(loose_parse_oid(b"r", &A[..39], sha1).is_err());
        assert_eq!(loose_parse_symbolic(b"ref: x").unwrap(), b"x");
        assert!(loose_parse_symbolic(b"ref: ").is_err());
    }
}

#[cfg(test)]
// The test calls the C ABI in sequence, as C would, under one contract.
#[allow(clippy::multiple_unsafe_ops_per_block)]
mod ffi_tests {
    //! The exports that stay in Rust, called as C would (Miri-tested).
    use super::ffi::*;
    use super::*;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::test_support;

    /// A reference as the exports that take `Option<RefView>` see it.
    ///
    /// # Safety
    ///
    /// `p` is NULL or a live reference, unwritten for `'a`.
    unsafe fn v<'a>(p: *mut GitReference) -> Option<RefView<'a>> {
        // SAFETY: the caller's contract.
        NonNull::new(p).map(|p| unsafe { RefView::from_ptr(p) })
    }

    fn cs<'a>(p: *const c_char) -> &'a [u8] {
        assert!(!p.is_null());
        // SAFETY: a string the export returned.
        unsafe { CStr::from_ptr(p) }.to_bytes()
    }

    #[test]
    fn alloc_realloc_getters() {
        let _g = test_support::init();
        let id =
            Oid::parse_hex(b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
        // SAFETY: valid strings and oids; each reference freed once.
        unsafe {
            let d = git_reference__alloc(c"refs/tags/v1".into(), Some(&id), None);
            assert_eq!(cs(git_reference_name(d)), b"refs/tags/v1");
            assert_eq!(cs(git_reference_shorthand(v(d))), b"v1");
            assert_eq!(git_reference_is_tag(v(d)), 1);
            assert!(git_reference_target_peel(v(d)).is_null());
            // The new name may be the reference's own.
            let mut p = d;
            let d = git_reference__realloc(&raw mut p, CStrArg::from_ptr(git_reference_name(d)));
            assert_eq!(cs(git_reference_name(d)), b"refs/tags/v1");
            let mut p = d;
            let r = git_reference__realloc(
                &raw mut p,
                c"refs/tags/a-longer-name-for-the-same-tag".into(),
            );
            assert!(p.is_null());
            assert_eq!(
                cs(git_reference_name(r)),
                b"refs/tags/a-longer-name-for-the-same-tag"
            );
            assert_eq!(*git_reference_target(r), id);
            let s = git_reference__alloc_symbolic(c"HEAD".into(), c"refs/tags/v1".into());
            let mut dup = core::ptr::null_mut();
            assert_eq!(git_reference_dup(Out::from(&mut dup), v(s)), 0);
            assert_eq!(git_reference_cmp(v(dup), v(s)), 0);
            assert!(git_reference_cmp(v(s), v(r)) < 0);
            git_reference_free(dup);
            git_reference_free(s);
            git_reference_free(r);

            let mut buf = [0 as c_char; 8];
            assert_eq!(
                git_reference_normalize_name(buf.as_mut_ptr(), 8, c"a/..b".into(), 0),
                -12
            );
            assert_eq!(
                git_reference_normalize_name(buf.as_mut_ptr(), 8, c"x/y//z".into(), 0),
                0
            );
            assert_eq!(cs(buf.as_ptr()), b"x/y/z");
            let mut valid = 7;
            assert_eq!(
                git_reference__name_is_valid(Out::from(&mut valid), c"refs/x".into(), 0),
                0
            );
            assert_eq!(valid, 1);
            // A name inside `buf`'s own buffer is read as C reads it: after
            // the clear (which empties a name at the start of the buffer).
            let mut s = GitStr::new();
            s.puts(b"xxq//w").unwrap();
            let own = s.as_ptr().wrapping_add(2);
            assert_eq!(git_reference__normalize_name(&raw mut s, own, 0), 0);
            assert_eq!(s.as_bytes(), b"q/w");
            s.sets(b"q//w").unwrap();
            let own = s.as_ptr();
            assert!(git_reference__normalize_name(&raw mut s, own, 0) < 0);
            assert_eq!(git_reference__is_branch(c"refs/heads/".into()), 1);
            // The predicates read a bounded head of the name.
            let long = c"refs/heads/a-branch-name-much-longer-than-thirty-two-bytes";
            assert_eq!(
                cs(git_reference__shorthand(long.into())),
                &long.to_bytes()[11..]
            );
            assert_eq!(git_reference__is_pseudoref(c"FETCH_HEAD".into()), 1);
            assert_eq!(
                git_reference__is_pseudoref(c"FETCH_HEAD_and_a_lot_more_after_it_x".into()),
                0
            );
            assert_eq!(git_reference__is_pseudoref(c"FETCH_HEADS".into()), 0);
            assert_eq!(
                git_reference__is_per_worktree_ref(c"refs/bisect/".into()),
                1
            );
            assert_eq!(git_reference__is_per_worktree_ref(c"refs/bisec".into()), 0);
            assert_eq!(git_reference__is_per_worktree_ref(c"ref".into()), 1);
        }
    }
}
