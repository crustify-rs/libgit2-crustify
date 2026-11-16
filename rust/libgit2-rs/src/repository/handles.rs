//! Owners of, and borrowed handles to, the objects of the modules that
//! other L-wave work packages port concurrently (refdb, references,
//! objects), as the repository uses them. (The odb, config and index are
//! `crate::odb`'s, `crate::config`'s and `crate::index`'s own types.)
//!
//! PENDING(l-refs, l-obj, l-commit): each type
//! here stands for that work package's own Rust type (`Refdb`,
//! `Reference`, `Object`) until the integration step,
//! which replaces it; they reach the C through `sys::pending::l_repo`. They
//! are thin: a non-null pointer, a `Drop` that calls the C free, and the
//! few safe calls the repository makes.
//!
//! The refcounted ones (`git_config`, `git_odb`, `git_refdb`, `git_index`)
//! start with a `git_refcount` (`src/util/util.h`, mirrored by
//! [`GitRefcount`]): [`refcount`] reaches it.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::oid::Oid;
use crate::sys::pending::l_repo as c;
use crate::util::errors::{ErrorCode, Result};
use crate::util::thread::GitRefcount;

/// The `git_refcount` at the start of a refcounted libgit2 object.
///
/// # Safety
///
/// `p` points to a live `git_config`, `git_odb`, `git_refdb` or
/// `git_index` (each starts with a `git_refcount`), and stays live while
/// the result is used.
pub(crate) unsafe fn refcount<'a>(p: NonNull<c_void>) -> &'a GitRefcount {
    // SAFETY: the caller's contract; `GitRefcount` is all atomics, so a
    // shared reference allows the C's concurrent updates.
    unsafe { p.cast::<GitRefcount>().as_ref() }
}

/// Read the field of type `T` at byte `offset` of the C struct at `p`.
///
/// # Safety
///
/// `p` points to a live struct that has an initialized, aligned `T` at
/// `offset`, which nothing writes during the read.
pub(crate) unsafe fn read_field<T: Copy>(p: NonNull<c_void>, offset: usize) -> T {
    // SAFETY: the caller's contract: `offset` is within the struct.
    let field = unsafe { p.as_ptr().byte_add(offset) }.cast::<T>();
    // SAFETY: the caller's contract: aligned and initialized.
    unsafe { field.read() }
}

/// Turn a C status into a `Result`.
#[inline]
pub(crate) fn check(code: c_int) -> Result<c_int> {
    ErrorCode::check(code)
}

/// A `const char *` result of C as a borrowed C string.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that stays valid for `'a`.
#[inline]
pub(crate) unsafe fn opt_cstr<'a>(p: *const c_char) -> Option<&'a CStr> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the caller's contract.
        Some(unsafe { CStr::from_ptr(p) })
    }
}

/// A NULL-or-string argument.
#[inline]
pub(crate) fn opt_ptr(s: Option<&CStr>) -> *const c_char {
    s.map_or(ptr::null(), CStr::as_ptr)
}

macro_rules! owner {
    ($(#[$m:meta])* $name:ident, $free:path) => {
        $(#[$m])*
        #[derive(Debug)]
        pub struct $name(NonNull<c_void>);

        #[allow(dead_code)]
        impl $name {
            /// Adopt a pointer C returned (NULL gives `None`).
            ///
            /// # Safety
            ///
            /// `p` is NULL, or an object of this type whose reference the
            /// caller owns and gives up.
            pub(crate) unsafe fn from_raw(p: *mut c_void) -> Option<$name> {
                NonNull::new(p).map($name)
            }

            /// The C pointer (still owned).
            pub(crate) fn as_ptr(&self) -> *mut c_void {
                self.0.as_ptr()
            }

            /// The C pointer, non-null.
            pub(crate) fn as_nonnull(&self) -> NonNull<c_void> {
                self.0
            }

            /// Give the reference up to C, as a non-null pointer.
            pub(crate) fn into_nonnull(self) -> NonNull<c_void> {
                let p = self.0;
                core::mem::forget(self);
                p
            }

            /// Give the reference up to C.
            pub(crate) fn into_raw(self) -> *mut c_void {
                let p = self.0.as_ptr();
                core::mem::forget(self);
                p
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: this owner holds one reference of a live object
                // (`from_raw`'s contract), released once here.
                unsafe { $free(self.0.as_ptr()) }
            }
        }
    };
}

/// Run a C constructor `f(&mut out)` and adopt what it wrote.
fn make<T>(
    f: impl FnOnce(*mut *mut c_void) -> c_int,
    adopt: unsafe fn(*mut c_void) -> Option<T>,
) -> Result<T> {
    let mut out: *mut c_void = ptr::null_mut();
    check(f(&raw mut out))?;
    // SAFETY: on success the constructor gave the caller its reference
    // (or NULL, which is `None`).
    match unsafe { adopt(out) } {
        Some(v) => Ok(v),
        None => Err(ErrorCode::GENERIC),
    }
}

owner!(
    /// An owned reference to a `git_refdb` (`git_refdb_free` on drop).
    /// PENDING(l-refs): `crate::refdb::Refdb`.
    Refdb,
    c::git_refdb_free
);

// SAFETY: `git_refdb` is refcounted with atomics; the files backend locks
// its caches (tests/libgit2/threads/refdb.c shares one between threads).
unsafe impl Send for Refdb {}

impl Refdb {
    /// Another reference to `p`.
    ///
    /// # Safety
    ///
    /// `p` is a live `git_refdb`.
    pub(crate) unsafe fn new_ref(p: NonNull<c_void>) -> Refdb {
        // SAFETY: the caller's contract.
        unsafe { refcount(p) }.inc();
        Refdb(p)
    }

    /// `git_refdb_init`.
    pub(crate) fn init(&self, head: Option<&CStr>, mode: u32, flags: u32) -> Result<()> {
        // SAFETY: a live refdb; `head` is NULL or NUL-terminated.
        check(unsafe {
            c::git_refdb_init(self.as_ptr(), opt_ptr(head), mode as libc::mode_t, flags)
        })
        .map(drop)
    }
}

owner!(
    /// An owned `git_reference` (`git_reference_free` on drop).
    /// PENDING(l-refs): `crate::refs::Reference`.
    Reference,
    c::git_reference_free
);

/// `GIT_REFERENCE_DIRECT`.
pub const GIT_REFERENCE_DIRECT: c_int = 1;
/// `GIT_REFERENCE_SYMBOLIC`.
pub const GIT_REFERENCE_SYMBOLIC: c_int = 2;

impl Reference {
    /// `git_reference_type`.
    pub fn kind(&self) -> c_int {
        // SAFETY: a live reference.
        unsafe { c::git_reference_type(self.as_ptr()) }
    }

    /// Whether it is symbolic.
    pub fn is_symbolic(&self) -> bool {
        self.kind() == GIT_REFERENCE_SYMBOLIC
    }

    /// `git_reference_name`.
    pub fn name(&self) -> &CStr {
        // SAFETY: a live reference's name lives as long as it.
        let p = unsafe { c::git_reference_name(self.as_ptr()) };
        // SAFETY: the name lives as long as the reference.
        unsafe { opt_cstr(p) }.unwrap_or(c"")
    }

    /// `git_reference_symbolic_target` (`None` for a direct reference).
    pub fn symbolic_target(&self) -> Option<&CStr> {
        // SAFETY: as `name`.
        let p = unsafe { c::git_reference_symbolic_target(self.as_ptr()) };
        // SAFETY: as `name`.
        unsafe { opt_cstr(p) }
    }

    /// `git_reference_target` (`None` for a symbolic reference).
    pub fn target(&self) -> Option<&Oid> {
        // SAFETY: NULL or the id inside the live reference.
        let p = unsafe { c::git_reference_target(self.as_ptr()) };
        // SAFETY: NULL or the id inside the live reference.
        unsafe { p.as_ref() }
    }

    /// `git_reference_is_branch`.
    pub fn is_branch(&self) -> bool {
        // SAFETY: a live reference.
        unsafe { c::git_reference_is_branch(self.as_ptr()) != 0 }
    }

    /// `git_reference_is_tag`.
    pub fn is_tag(&self) -> bool {
        // SAFETY: a live reference.
        unsafe { c::git_reference_is_tag(self.as_ptr()) != 0 }
    }

    /// `git_reference_is_remote`.
    pub fn is_remote(&self) -> bool {
        // SAFETY: a live reference.
        unsafe { c::git_reference_is_remote(self.as_ptr()) != 0 }
    }

    /// `git_reference_peel`.
    pub(crate) fn peel(&self, t: c_int) -> Result<Object> {
        // SAFETY: a live reference, a local out-parameter.
        make(
            |out| unsafe { c::git_reference_peel(out, self.as_ptr(), t) },
            Object::from_raw,
        )
    }
}

owner!(
    /// An owned `git_object` (`git_object_free` on drop).
    /// PENDING(l-obj): `crate::object::Object`.
    Object,
    c::git_object_free
);

impl Object {
    /// `git_object_id`.
    pub fn id(&self) -> Oid {
        // SAFETY: a live object's id.
        let p = unsafe { c::git_object_id(self.as_ptr()) };
        // SAFETY: the id inside the live object.
        unsafe { *p }
    }

    /// `git_object_peel`.
    pub(crate) fn peel(&self, t: c_int) -> Result<Object> {
        // SAFETY: a live object, a local out-parameter.
        make(
            |out| unsafe { c::git_object_peel(out, self.as_ptr(), t) },
            Object::from_raw,
        )
    }
}

owner!(
    /// An owned `git_reference_iterator`.
    RefIter,
    c::git_reference_iterator_free
);

/// `git_reference__is_branch` and friends, on names.
pub(crate) mod refname {
    use super::*;

    /// `git_reference__is_branch`.
    pub fn is_branch(name: &CStr) -> bool {
        // SAFETY: a NUL-terminated name.
        unsafe { c::git_reference__is_branch(name.as_ptr()) != 0 }
    }
    /// `git_reference__is_tag`.
    pub fn is_tag(name: &CStr) -> bool {
        // SAFETY: as above.
        unsafe { c::git_reference__is_tag(name.as_ptr()) != 0 }
    }
    /// `git_reference__is_remote`.
    pub fn is_remote(name: &CStr) -> bool {
        // SAFETY: as above.
        unsafe { c::git_reference__is_remote(name.as_ptr()) != 0 }
    }
    /// `git_reference__shorthand`: a suffix of `name`.
    pub fn shorthand(name: &CStr) -> &CStr {
        // SAFETY: the result points into `name` (a suffix), so it lives
        // as long as it.
        let p = unsafe { c::git_reference__shorthand(name.as_ptr()) };
        // SAFETY: a suffix of `name`.
        unsafe { opt_cstr(p) }.unwrap_or(name)
    }
    /// `git_reference_name_is_valid`.
    pub fn is_valid(name: &CStr) -> Result<bool> {
        let mut v: c_int = 0;
        // SAFETY: a NUL-terminated name, a local out.
        check(unsafe { c::git_reference_name_is_valid(&raw mut v, name.as_ptr()) })?;
        Ok(v != 0)
    }
}
