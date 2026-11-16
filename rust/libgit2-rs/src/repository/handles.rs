//! Owners of, and borrowed handles to, the objects of the modules that
//! other L-wave work packages port concurrently (config, odb, refdb,
//! index, references, objects), as the repository uses them.
//!
//! PENDING(l-config, l-odb, l-refs, l-index, l-obj, l-commit): each type
//! here stands for that work package's own Rust type (`Config`, `Odb`,
//! `Refdb`, `Index`, `Reference`, `Object`) until the integration step,
//! which replaces it; they reach the C through `sys::pending::l_repo`. They
//! are thin: a non-null pointer, a `Drop` that calls the C free, and the
//! few safe calls the repository makes.
//!
//! The refcounted ones (`git_config`, `git_odb`, `git_refdb`, `git_index`)
//! start with a `git_refcount` (`src/util/util.h`, mirrored by
//! [`GitRefcount`]): [`refcount`] reaches it.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::oid::{Oid, OidType};
use crate::sys::pending::l_repo as c;
use crate::util::errors::{ErrorCode, Result};
use crate::util::str::GitStr;
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
    check(f(&mut out))?;
    // SAFETY: on success the constructor gave the caller its reference
    // (or NULL, which is `None`).
    match unsafe { adopt(out) } {
        Some(v) => Ok(v),
        None => Err(ErrorCode::GENERIC),
    }
}

owner!(
    /// An owned reference to a `git_config` (`git_config_free` on drop).
    /// PENDING(l-config): `crate::config::Config`.
    Config,
    c::git_config_free
);

// SAFETY: `git_config` is refcounted with atomics and released from any
// thread; its backends lock their state (libgit2 shares a repository's
// configuration between threads).
unsafe impl Send for Config {}

impl Config {
    /// `git_config_new`.
    pub(crate) fn new() -> Result<Config> {
        // SAFETY: `out` is a local out-parameter.
        make(|out| unsafe { c::git_config_new(out) }, Config::from_raw)
    }

    /// `git_config_open_ondisk`.
    pub(crate) fn open_ondisk(path: &CStr) -> Result<Config> {
        // SAFETY: a NUL-terminated path and a local out-parameter.
        make(
            |out| unsafe { c::git_config_open_ondisk(out, path.as_ptr()) },
            Config::from_raw,
        )
    }

    /// `git_config_open_default`.
    pub(crate) fn open_default() -> Result<Config> {
        // SAFETY: a local out-parameter.
        make(|out| unsafe { c::git_config_open_default(out) }, Config::from_raw)
    }

    /// Another reference to the configuration `p` (`GIT_REFCOUNT_INC`).
    ///
    /// # Safety
    ///
    /// `p` is a live `git_config`.
    pub(crate) unsafe fn new_ref(p: NonNull<c_void>) -> Config {
        // SAFETY: the caller's contract.
        unsafe { refcount(p) }.inc();
        Config(p)
    }

    /// A borrowed handle.
    pub(crate) fn borrow(&self) -> ConfigRef<'_> {
        ConfigRef(self.0, core::marker::PhantomData)
    }
}

/// A borrowed `git_config` (a weak pointer, or a [`Config`]'s).
#[derive(Clone, Copy, Debug)]
pub struct ConfigRef<'a>(NonNull<c_void>, core::marker::PhantomData<&'a ()>);

impl<'a> ConfigRef<'a> {
    /// Borrow the configuration `p`.
    ///
    /// # Safety
    ///
    /// `p` is a live `git_config` for `'a`.
    pub(crate) unsafe fn from_ptr(p: NonNull<c_void>) -> ConfigRef<'a> {
        ConfigRef(p, core::marker::PhantomData)
    }

    /// The C pointer.
    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// `git_config_get_bool`.
    pub(crate) fn get_bool(self, name: &CStr) -> Result<bool> {
        let mut v: c_int = 0;
        // SAFETY: a live config (the handle's invariant), a NUL-terminated
        // name and a local out-parameter.
        check(unsafe { c::git_config_get_bool(&mut v, self.as_ptr(), name.as_ptr()) })?;
        Ok(v != 0)
    }

    /// `git_config_get_int32`.
    pub(crate) fn get_int32(self, name: &CStr) -> Result<i32> {
        let mut v: i32 = 0;
        // SAFETY: as `get_bool`.
        check(unsafe { c::git_config_get_int32(&mut v, self.as_ptr(), name.as_ptr()) })?;
        Ok(v)
    }

    /// `git_config_get_entry`.
    pub(crate) fn get_entry(self, name: &CStr) -> Result<ConfigEntry> {
        let mut e: *mut c::ConfigEntryRaw = ptr::null_mut();
        // SAFETY: as `get_bool`.
        check(unsafe { c::git_config_get_entry(&mut e, self.as_ptr(), name.as_ptr()) })?;
        NonNull::new(e).map(ConfigEntry).ok_or(ErrorCode::GENERIC)
    }

    /// `git_config__lookup_entry` (`None` when not found).
    pub(crate) fn lookup_entry(self, key: &CStr, no_errors: bool) -> Result<Option<ConfigEntry>> {
        let mut e: *mut c::ConfigEntryRaw = ptr::null_mut();
        // SAFETY: as `get_bool`.
        check(unsafe {
            c::git_config__lookup_entry(&mut e, self.as_ptr(), key.as_ptr(), no_errors)
        })?;
        Ok(NonNull::new(e).map(ConfigEntry))
    }

    /// `git_config_set_bool`.
    pub(crate) fn set_bool(self, name: &CStr, v: bool) -> Result<()> {
        // SAFETY: a live config and a NUL-terminated name.
        check(unsafe { c::git_config_set_bool(self.as_ptr(), name.as_ptr(), c_int::from(v)) })
            .map(drop)
    }

    /// `git_config_set_int32`.
    pub(crate) fn set_int32(self, name: &CStr, v: i32) -> Result<()> {
        // SAFETY: as `set_bool`.
        check(unsafe { c::git_config_set_int32(self.as_ptr(), name.as_ptr(), v) }).map(drop)
    }

    /// `git_config_set_string`.
    pub(crate) fn set_string(self, name: &CStr, v: &CStr) -> Result<()> {
        // SAFETY: as `set_bool`, and a NUL-terminated value.
        check(unsafe { c::git_config_set_string(self.as_ptr(), name.as_ptr(), v.as_ptr()) })
            .map(drop)
    }

    /// `git_config_delete_entry`.
    pub(crate) fn delete_entry(self, name: &CStr) -> Result<()> {
        // SAFETY: as `set_bool`.
        check(unsafe { c::git_config_delete_entry(self.as_ptr(), name.as_ptr()) }).map(drop)
    }

    /// `git_config__update_entry`.
    pub(crate) fn update_entry(
        self,
        key: &CStr,
        value: Option<&CStr>,
        overwrite_existing: bool,
        only_if_existing: bool,
    ) -> Result<()> {
        // SAFETY: as `set_bool`; `value` is NULL or NUL-terminated.
        check(unsafe {
            c::git_config__update_entry(
                self.as_ptr(),
                key.as_ptr(),
                opt_ptr(value),
                overwrite_existing,
                only_if_existing,
            )
        })
        .map(drop)
    }

    /// `git_config__get_path`.
    pub(crate) fn get_path(self, out: &mut GitStr, name: &CStr) -> Result<()> {
        // SAFETY: a live config, an exclusive `git_str`, a NUL-terminated
        // name.
        check(unsafe { c::git_config__get_path(out, self.as_ptr(), name.as_ptr()) }).map(drop)
    }

    /// `git_config__get_string_buf`.
    pub(crate) fn get_string_buf(self, out: &mut GitStr, name: &CStr) -> Result<()> {
        // SAFETY: as `get_path`.
        check(unsafe { c::git_config__get_string_buf(out, self.as_ptr(), name.as_ptr()) })
            .map(drop)
    }

    /// `git_config_add_file_ondisk`.
    pub(crate) fn add_file_ondisk(
        self,
        path: &CStr,
        level: c_int,
        repo: *const c_void,
        force: bool,
    ) -> Result<()> {
        // SAFETY: a live config, a NUL-terminated path; `repo` is NULL or
        // the live repository the caller holds.
        check(unsafe {
            c::git_config_add_file_ondisk(
                self.as_ptr(),
                path.as_ptr(),
                level,
                repo,
                c_int::from(force),
            )
        })
        .map(drop)
    }

    /// `git_config_set_writeorder` with one level.
    pub(crate) fn set_writeorder(self, level: c_int) -> Result<()> {
        let mut l = level;
        // SAFETY: a live config and a one-element array.
        check(unsafe { c::git_config_set_writeorder(self.as_ptr(), &mut l, 1) }).map(drop)
    }

    /// `git_config_snapshot`.
    pub(crate) fn snapshot(self) -> Result<Config> {
        // SAFETY: a live config and a local out-parameter.
        make(
            |out| unsafe { c::git_config_snapshot(out, self.as_ptr()) },
            Config::from_raw,
        )
    }

    /// `git_config_open_level`.
    pub(crate) fn open_level(self, level: c_int) -> Result<Config> {
        // SAFETY: as `snapshot`.
        make(
            |out| unsafe { c::git_config_open_level(out, self.as_ptr(), level) },
            Config::from_raw,
        )
    }

    /// `git_config_get_multivar_foreach` with a Rust closure; the closure's
    /// error stops the iteration and is returned (as C's callback result).
    pub(crate) fn multivar_foreach(
        self,
        name: &CStr,
        f: &mut dyn FnMut(&ConfigEntryView<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut payload = Foreach { f, panic: None };
        // SAFETY: a live config, a NUL-terminated name, no regexp; the
        // payload outlives the call and `foreach_cb` treats it as such.
        let r = unsafe {
            c::git_config_get_multivar_foreach(
                self.as_ptr(),
                name.as_ptr(),
                ptr::null(),
                foreach_cb,
                (&raw mut payload).cast(),
            )
        };
        if let Some(p) = payload.panic {
            std::panic::resume_unwind(p);
        }
        check(r).map(drop)
    }

    /// `git_config_foreach_match` with a Rust closure.
    pub(crate) fn foreach_match(
        self,
        regexp: &CStr,
        f: &mut dyn FnMut(&ConfigEntryView<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut payload = Foreach { f, panic: None };
        // SAFETY: as `multivar_foreach`.
        let r = unsafe {
            c::git_config_foreach_match(
                self.as_ptr(),
                regexp.as_ptr(),
                foreach_cb,
                (&raw mut payload).cast(),
            )
        };
        if let Some(p) = payload.panic {
            std::panic::resume_unwind(p);
        }
        check(r).map(drop)
    }
}

struct Foreach<'f> {
    f: &'f mut dyn FnMut(&ConfigEntryView<'_>) -> Result<()>,
    panic: Option<Box<dyn core::any::Any + Send>>,
}

unsafe extern "C" fn foreach_cb(entry: *const c::ConfigEntryRaw, payload: *mut c_void) -> c_int {
    // SAFETY: `payload` is the `Foreach` of the running call, which nothing
    // else touches during it; `entry` is the live entry C passes.
    let (data, entry) = unsafe { (&mut *payload.cast::<Foreach<'_>>(), &*entry) };
    if data.panic.is_some() {
        return -1;
    }
    let view = ConfigEntryView(entry);
    match std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| (data.f)(&view))) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => e.raw(),
        Err(p) => {
            data.panic = Some(p);
            -1
        }
    }
}

/// A `git_config_entry` C lends for the extent of a callback.
pub struct ConfigEntryView<'a>(&'a c::ConfigEntryRaw);

impl ConfigEntryView<'_> {
    /// The entry's name.
    pub fn name(&self) -> &CStr {
        // SAFETY: C's entries have a NUL-terminated name, live for the
        // callback.
        unsafe { opt_cstr(self.0.name) }.unwrap_or(c"")
    }

    /// The entry's value (`None` for a valueless entry).
    pub fn value(&self) -> Option<&CStr> {
        // SAFETY: NULL or NUL-terminated, live for the callback.
        unsafe { opt_cstr(self.0.value) }
    }
}

/// An owned `git_config_entry` (`git_config_entry_free` on drop).
#[derive(Debug)]
pub struct ConfigEntry(NonNull<c::ConfigEntryRaw>);

impl ConfigEntry {
    /// The entry's name.
    pub fn name(&self) -> &CStr {
        // SAFETY: the entry is live while owned; its name is NUL-terminated.
        unsafe { opt_cstr(self.0.as_ref().name) }.unwrap_or(c"")
    }

    /// The entry's value.
    pub fn value(&self) -> Option<&CStr> {
        // SAFETY: as `name`.
        unsafe { opt_cstr(self.0.as_ref().value) }
    }
}

impl Drop for ConfigEntry {
    fn drop(&mut self) {
        // SAFETY: the owned entry, freed once.
        unsafe { c::git_config_entry_free(self.0.as_ptr()) }
    }
}

owner!(
    /// An owned reference to a `git_odb` (`git_odb_free` on drop).
    /// PENDING(l-odb): `crate::odb::Odb`.
    Odb,
    c::git_odb_free
);

// SAFETY: `git_odb` is refcounted with atomics and locks internally
// (docs/threading.md: thread-safe).
unsafe impl Send for Odb {}
// SAFETY: as above.
unsafe impl Sync for Odb {}

impl Odb {
    /// `git_odb_new_ext` with `oid_type`.
    pub(crate) fn new(oid_type: OidType) -> Result<Odb> {
        let opts = c::OidTypeOptions {
            version: 1,
            oid_type: oid_type.raw(),
        };
        // SAFETY: valid options and a local out-parameter.
        make(|out| unsafe { c::git_odb_new_ext(out, &opts) }, Odb::from_raw)
    }

    /// Another reference to `p` (`GIT_REFCOUNT_INC`).
    ///
    /// # Safety
    ///
    /// `p` is a live `git_odb`.
    pub(crate) unsafe fn new_ref(p: NonNull<c_void>) -> Odb {
        // SAFETY: the caller's contract.
        unsafe { refcount(p) }.inc();
        Odb(p)
    }

    /// `git_odb_add_disk_alternate`.
    pub(crate) fn add_disk_alternate(&self, path: &CStr) -> Result<()> {
        // SAFETY: a live odb and a NUL-terminated path.
        check(unsafe { c::git_odb_add_disk_alternate(self.as_ptr(), path.as_ptr()) }).map(drop)
    }

    /// `git_odb__set_caps`.
    pub(crate) fn set_caps(&self, caps: c_int) -> Result<()> {
        // SAFETY: a live odb.
        check(unsafe { c::git_odb__set_caps(self.as_ptr(), caps) }).map(drop)
    }

    /// `git_odb__add_default_backends(odb, dir, 0, 0)`.
    pub(crate) fn add_default_backends(&self, dir: &CStr) -> Result<()> {
        // SAFETY: a live odb and a NUL-terminated path.
        check(unsafe {
            c::git_odb__add_default_backends(self.as_ptr(), dir.as_ptr(), false, 0)
        })
        .map(drop)
    }

    /// `git_odb_exists`.
    pub fn exists(&self, id: &Oid) -> bool {
        // SAFETY: a live odb and a valid id.
        unsafe { c::git_odb_exists(self.as_ptr(), id) != 0 }
    }

    /// The odb's `options.oid_type`.
    ///
    /// PENDING(l-odb): a field read of `struct git_odb` (`odb.h`) at the
    /// offset the layout test of `repository` checks.
    pub(crate) fn oid_type_raw(&self) -> crate::oid::git_oid_t {
        // SAFETY: a live odb; `options.oid_type` is at byte 60 of
        // `struct git_odb` (checked by the layout test), written only when
        // the odb is created.
        unsafe {
            self.as_ptr()
                .cast::<u8>()
                .add(ODB_OID_TYPE_OFFSET)
                .cast::<crate::oid::git_oid_t>()
                .read()
        }
    }
}

/// `offsetof(git_odb, options.oid_type)`.
pub(crate) const ODB_OID_TYPE_OFFSET: usize = 60;

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
    /// An owned reference to a `git_index` (`git_index_free` on drop).
    /// PENDING(l-index): `crate::index::Index`.
    Index,
    c::git_index_free
);

// SAFETY: refcounted with atomics, freed from any thread; one owner uses
// it at a time (it is not `Sync`).
unsafe impl Send for Index {}

impl Index {
    /// `git_index_open_ext`.
    pub(crate) fn open(path: &CStr, oid_type: OidType) -> Result<Index> {
        let opts = c::OidTypeOptions {
            version: 1,
            oid_type: oid_type.raw(),
        };
        // SAFETY: a NUL-terminated path, valid options, a local out.
        make(
            |out| unsafe { c::git_index_open_ext(out, path.as_ptr(), &opts) },
            Index::from_raw,
        )
    }

    /// Another reference to `p`.
    ///
    /// # Safety
    ///
    /// `p` is a live `git_index`.
    pub(crate) unsafe fn new_ref(p: NonNull<c_void>) -> Index {
        // SAFETY: the caller's contract.
        unsafe { refcount(p) }.inc();
        Index(p)
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
        unsafe { opt_cstr(c::git_reference_name(self.as_ptr())) }.unwrap_or(c"")
    }

    /// `git_reference_symbolic_target` (`None` for a direct reference).
    pub fn symbolic_target(&self) -> Option<&CStr> {
        // SAFETY: as `name`.
        unsafe { opt_cstr(c::git_reference_symbolic_target(self.as_ptr())) }
    }

    /// `git_reference_target` (`None` for a symbolic reference).
    pub fn target(&self) -> Option<&Oid> {
        // SAFETY: NULL or the id inside the live reference.
        unsafe { c::git_reference_target(self.as_ptr()).as_ref() }
    }

    /// The raw `git_reference_target` pointer (NULL for symbolic).
    pub(crate) fn target_ptr(&self) -> *const Oid {
        // SAFETY: a live reference.
        unsafe { c::git_reference_target(self.as_ptr()) }
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
        unsafe { *c::git_object_id(self.as_ptr()) }
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
        unsafe { opt_cstr(c::git_reference__shorthand(name.as_ptr())) }.unwrap_or(name)
    }
    /// `git_reference_name_is_valid`.
    pub fn is_valid(name: &CStr) -> Result<bool> {
        let mut v: c_int = 0;
        // SAFETY: a NUL-terminated name, a local out.
        check(unsafe { c::git_reference_name_is_valid(&mut v, name.as_ptr()) })?;
        Ok(v != 0)
    }
}
