//! The safe Rust API of the revision walker: [`Revwalk`], an iterator of
//! commit ids, with [`Sort`] flags and push/hide builders.
//!
//! ```no_run
//! use libgit2_rs::revwalk::api::Repository;
//! use libgit2_rs::{Revwalk, Sort};
//!
//! let repo = Repository::open("/path/to/repo.git")?;
//! let mut walk = Revwalk::new(&repo)?;
//! walk.sorting(Sort::TOPOLOGICAL | Sort::TIME)?
//!     .push_ref("refs/heads/main")?
//!     .hide_ref("refs/tags/v1.0")?;
//! for id in walk {
//!     println!("{}", id?);
//! }
//! # Ok::<(), libgit2_rs::Error>(())
//! ```

use core::ffi::{c_int, c_void};
use core::marker::PhantomData;
use core::ptr::NonNull;
use std::any::Any;
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::{GIT_SORT_NONE, GIT_SORT_REVERSE, GIT_SORT_TIME, GIT_SORT_TOPOLOGICAL};
use super::{GitRevwalk, PushOptions, State, free_walk, new_walk};
use crate::git_error_set;
use crate::oid::Oid;
use crate::sys::pending::l_revwalk as p;
use crate::util::errors::{Error, ErrorClass, ErrorCode};

type Result<T> = core::result::Result<T, Error>;

/// A repository to walk. PENDING(l-repo): a minimal stand-in for the
/// repository WP's `Repository`, which replaces it at integration (the
/// walk only needs to borrow one).
#[derive(Debug)]
pub struct Repository(NonNull<p::git_repository>);

impl Repository {
    /// `git_repository_open`.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Repository> {
        use std::os::unix::ffi::OsStrExt;
        let c = cstring(path.as_ref().as_os_str().as_bytes())?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `out` writable; `c` a string.
        let e = unsafe { p::git_repository_open(&mut out, c.as_ptr()) };
        ErrorCode::check(e)?;
        NonNull::new(out)
            .map(Repository)
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))
    }

    fn as_ptr(&self) -> *mut p::git_repository {
        self.0.as_ptr()
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        // SAFETY: we own the repository; not used after.
        unsafe { p::git_repository_free(self.0.as_ptr()) }
    }
}

fn cstring(b: &[u8]) -> Result<CString> {
    CString::new(b).map_err(|_| {
        Error::from(git_error_set!(
            ErrorClass::INVALID,
            "string contains a NUL byte"
        ))
    })
}

/// The order of a walk (`git_sort_t`), combinable with `|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sort(u32);

impl Sort {
    /// `GIT_SORT_NONE`: the walk's natural order (reverse chronological
    /// along the parents, as `git log` without options).
    pub const NONE: Sort = Sort(GIT_SORT_NONE);
    /// `GIT_SORT_TOPOLOGICAL`: no parent before all of its children.
    pub const TOPOLOGICAL: Sort = Sort(GIT_SORT_TOPOLOGICAL);
    /// `GIT_SORT_TIME`: by commit time, newest first.
    pub const TIME: Sort = Sort(GIT_SORT_TIME);
    /// `GIT_SORT_REVERSE`: the reverse of the selected order.
    pub const REVERSE: Sort = Sort(GIT_SORT_REVERSE);

    /// The flags as bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The flags of `bits` (unknown bits are kept, as C does).
    pub const fn from_bits(bits: u32) -> Sort {
        Sort(bits)
    }

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Sort) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for Sort {
    type Output = Sort;
    fn bitor(self, rhs: Sort) -> Sort {
        Sort(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for Sort {
    fn bitor_assign(&mut self, rhs: Sort) {
        self.0 |= rhs.0;
    }
}

/// The hide callback's closure, and a panic it raised (resumed when the
/// walk call returns).
struct HideState<'repo> {
    f: Box<dyn FnMut(&Oid) -> bool + 'repo>,
    panic: Option<Box<dyn Any + Send>>,
}

/// The C callback over a [`HideState`].
unsafe extern "C" fn hide_trampoline(id: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: `payload` is the walk's boxed `HideState`, alive while the
    // walk runs, and reached only from here during the call (the walk
    // calls back from `&mut Revwalk` methods); `id` points to a node's id.
    let (st, id) = unsafe { (&mut *payload.cast::<HideState<'_>>(), &*id) };
    if st.panic.is_some() {
        return 1;
    }
    match catch_unwind(AssertUnwindSafe(|| (st.f)(id))) {
        Ok(hide) => c_int::from(hide),
        Err(e) => {
            st.panic = Some(e);
            1
        }
    }
}

/// A revision walk over a repository (`git_revwalk`): an iterator of
/// commit ids in the order of [`Sort`].
///
/// Push the commits to start from and hide those (with their ancestors)
/// to leave out, then iterate. At the end of the iteration the walk
/// resets (as C's): pushes, hides, sorting and first-parent mode must be
/// set again for another walk; the hide callback stays.
pub struct Revwalk<'repo> {
    raw: NonNull<GitRevwalk>,
    /// From `Box::into_raw`: C holds it as the callback's payload.
    hide: Option<NonNull<HideState<'repo>>>,
    _repo: PhantomData<&'repo Repository>,
}

impl core::fmt::Debug for Revwalk<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Revwalk").finish_non_exhaustive()
    }
}

impl Drop for Revwalk<'_> {
    fn drop(&mut self) {
        // SAFETY: our walk, not used after.
        unsafe { free_walk(self.raw) };
        self.drop_hide();
    }
}

impl<'repo> Revwalk<'repo> {
    /// `git_revwalk_new`: a walk over `repo`.
    pub fn new(repo: &'repo Repository) -> Result<Revwalk<'repo>> {
        let raw = new_walk(repo.as_ptr())?;
        Ok(Revwalk {
            raw,
            hide: None,
            _repo: PhantomData,
        })
    }

    /// Run `f` on the walk and its state, then resume a panic of the hide
    /// callback.
    fn with<R>(&mut self, f: impl FnOnce(&GitRevwalk, &mut State) -> R) -> R {
        // SAFETY: our live walk; `&mut self` excludes any other use of it
        // during `f` (the hide callback reaches only its own state).
        let w = unsafe { self.raw.as_ref() };
        // SAFETY: as above: the only reference to the state.
        let st = unsafe { w.state() };
        let r = f(w, st);
        if let Some(h) = self.hide {
            // SAFETY: our live hide state; the walk is not running, so
            // nothing else reaches it.
            if let Some(p) = unsafe { (*h.as_ptr()).panic.take() } {
                resume_unwind(p);
            }
        }
        r
    }

    /// Free the hide state (after C stopped using it).
    fn drop_hide(&mut self) {
        if let Some(h) = self.hide.take() {
            // SAFETY: from `Box::into_raw`, no longer the walk's payload.
            drop(unsafe { Box::from_raw(h.as_ptr()) });
        }
    }

    /// `git_revwalk_sorting`: set the order (resets a walk in progress).
    pub fn sorting(&mut self, sort: Sort) -> Result<&mut Self> {
        self.with(|w, st| w.set_sorting(st, sort.0));
        Ok(self)
    }

    /// `git_revwalk_simplify_first_parent`: follow only first parents.
    pub fn simplify_first_parent(&mut self) -> Result<&mut Self> {
        self.with(|_, st| st.first_parent = true);
        Ok(self)
    }

    fn push_id(&mut self, id: &Oid, uninteresting: c_int) -> Result<&mut Self> {
        let opts = PushOptions {
            uninteresting,
            ..PushOptions::default()
        };
        self.with(|w, st| w.push_commit(st, id, &opts))?;
        Ok(self)
    }

    /// `git_revwalk_push`: start from `id` (a commit, or what peels to one).
    pub fn push(&mut self, id: &Oid) -> Result<&mut Self> {
        self.push_id(id, 0)
    }

    /// `git_revwalk_hide`: leave out `id` and its ancestors.
    pub fn hide(&mut self, id: &Oid) -> Result<&mut Self> {
        self.push_id(id, 1)
    }

    fn push_refname(&mut self, name: &[u8], uninteresting: c_int) -> Result<&mut Self> {
        let c = cstring(name)?;
        let opts = PushOptions {
            uninteresting,
            ..PushOptions::default()
        };
        self.with(|w, st| w.push_ref(st, c.as_ptr(), &opts))?;
        Ok(self)
    }

    /// `git_revwalk_push_ref`: start from what the reference points to.
    pub fn push_ref(&mut self, name: &str) -> Result<&mut Self> {
        self.push_refname(name.as_bytes(), 0)
    }

    /// `git_revwalk_hide_ref`.
    pub fn hide_ref(&mut self, name: &str) -> Result<&mut Self> {
        self.push_refname(name.as_bytes(), 1)
    }

    /// `git_revwalk_push_head`.
    pub fn push_head(&mut self) -> Result<&mut Self> {
        self.push_refname(b"HEAD", 0)
    }

    /// `git_revwalk_hide_head`.
    pub fn hide_head(&mut self) -> Result<&mut Self> {
        self.push_refname(b"HEAD", 1)
    }

    fn glob(&mut self, glob: &str, uninteresting: c_int) -> Result<&mut Self> {
        let c = cstring(glob.as_bytes())?;
        let opts = PushOptions {
            uninteresting,
            ..PushOptions::default()
        };
        self.with(|w, st| w.push_glob(st, &c, Some(&opts)))?;
        Ok(self)
    }

    /// `git_revwalk_push_glob`: start from every reference matching `glob`
    /// (`refs/` is implied; a glob without `?`, `*` or `[` gets `/*`).
    pub fn push_glob(&mut self, glob: &str) -> Result<&mut Self> {
        self.glob(glob, 0)
    }

    /// `git_revwalk_hide_glob`.
    pub fn hide_glob(&mut self, glob: &str) -> Result<&mut Self> {
        self.glob(glob, 1)
    }

    /// `git_revwalk_push_range`: `a..b` pushes `b` and hides `a`. The
    /// symmetric difference `a...b` is not supported (an
    /// `ErrorCode::INVALID_SPEC` error, as in C).
    pub fn push_range(&mut self, range: &str) -> Result<&mut Self> {
        let c = cstring(range.as_bytes())?;
        self.with(|w, st| w.push_range(st, c.as_ptr()))?;
        Ok(self)
    }

    /// `git_revwalk_reset`: forget the pushes, hides, sorting and
    /// first-parent mode.
    pub fn reset(&mut self) -> &mut Self {
        self.with(|w, st| w.reset(st));
        self
    }

    /// `git_revwalk_add_hide_cb`: also hide each commit for which `f`
    /// returns `true` (and, in the walk's order, what is only reachable
    /// through it). Replaces a previous callback; resets a walk in
    /// progress. A panic in `f` stops the walk and resumes in the caller.
    pub fn set_hide_callback(&mut self, f: impl FnMut(&Oid) -> bool + 'repo) -> &mut Self {
        let state = Box::new(HideState {
            f: Box::new(f),
            panic: None,
        });
        // SAFETY: `Box::into_raw` is never null.
        let state = unsafe { NonNull::new_unchecked(Box::into_raw(state)) };
        let payload = state.as_ptr().cast::<c_void>();
        self.with(|w, st| w.set_hide_cb(st, Some(hide_trampoline), payload));
        self.drop_hide();
        self.hide = Some(state);
        self
    }

    /// `git_revwalk_add_hide_cb(walk, NULL, NULL)`: remove the callback.
    pub fn clear_hide_callback(&mut self) -> &mut Self {
        self.with(|w, st| w.set_hide_cb(st, None, core::ptr::null_mut()));
        self.drop_hide();
        self
    }
}

impl Iterator for Revwalk<'_> {
    type Item = Result<Oid>;

    /// `git_revwalk_next`: the next id; `None` at the end of the walk.
    fn next(&mut self) -> Option<Result<Oid>> {
        match self.with(|w, st| w.next(st)) {
            Ok(id) => Some(Ok(id)),
            Err(ErrorCode::ITER_OVER) => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}
