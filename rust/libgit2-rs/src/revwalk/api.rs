//! The safe Rust API of the revision walker: [`Revwalk`], an iterator of
//! commit ids, with [`Sort`] flags and push/hide builders.
//!
//! `git_revwalk_pathspec` has no safe counterpart: it would need a Rust
//! `Pathspec`, and pathspec.c stays C (PORTING.md). Reference names, globs
//! and ranges are bytes (`impl AsRef<[u8]>`): git allows names that are not
//! UTF-8.
//!
//! ```no_run
//! use libgit2_rs::Repository;
//! use libgit2_rs::{Revwalk, Sort};
//!
//! let repo = Repository::open("/path/to/repo.git")?;
//! let mut walk = Revwalk::new(&repo)?;
//! walk.sorting(Sort::TOPOLOGICAL | Sort::TIME)
//!     .push_ref("refs/heads/main")?
//!     .hide_ref("refs/tags/v1.0")?;
//! for id in walk {
//!     println!("{}", id?);
//! }
//! # Ok::<(), libgit2_rs::Error>(())
//! ```

use core::ffi::{c_int, c_void};
use core::ptr::NonNull;
use std::any::Any;
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::{GIT_SORT_NONE, GIT_SORT_REVERSE, GIT_SORT_TIME, GIT_SORT_TOPOLOGICAL};
use super::{GitRevwalk, PushOptions, State, free_walk, new_walk};
use crate::git_error_set;
use crate::oid::Oid;
use crate::util::errors::{Error, ErrorClass, ErrorCode};

type Result<T> = core::result::Result<T, Error>;

pub use crate::repository::Repository;

fn cstring(b: &[u8]) -> Result<CString> {
    CString::new(b).map_err(|_| {
        Error::from(git_error_set!(
            ErrorClass::INVALID,
            "string contains a NUL byte"
        ))
    })
}

crate::api::flags::bitflags! {
    /// The order of a walk (`git_sort_t`), combinable with `|`.
    pub struct Sort: u32 {
        /// `GIT_SORT_NONE`: the walk's natural order (reverse
        /// chronological along the parents, as `git log` without options).
        NONE = GIT_SORT_NONE;
        /// `GIT_SORT_TOPOLOGICAL`: no parent before all of its children.
        TOPOLOGICAL = GIT_SORT_TOPOLOGICAL;
        /// `GIT_SORT_TIME`: by commit time, newest first.
        TIME = GIT_SORT_TIME;
        /// `GIT_SORT_REVERSE`: the reverse of the selected order.
        REVERSE = GIT_SORT_REVERSE;
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
    let st = unsafe { &mut *payload.cast::<HideState<'_>>() };
    // SAFETY: as above: a node's id, valid for the call.
    let id = unsafe { &*id };
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
    repo: &'repo Repository,
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
        // SAFETY: a live repository, which outlives the walk: the walk
        // borrows it for `'repo`.
        let raw = unsafe { new_walk(repo.raw_ptr()) }?;
        Ok(Revwalk {
            raw,
            hide: None,
            repo,
        })
    }

    /// Run `f` on the walk and its state.
    fn with<R>(&mut self, f: impl FnOnce(&GitRevwalk, &State) -> R) -> R {
        // SAFETY: our live walk; `&mut self` excludes any other use of it
        // during `f` (the hide closure cannot reach `self`).
        let w = unsafe { self.raw.as_ref() };
        f(w, w.state())
    }

    /// `git_revwalk_next`; if the hide closure panicked during it (the
    /// only call that runs the closure), the walk is reset and the panic
    /// resumed.
    fn step(&mut self) -> crate::util::errors::Result<Oid> {
        let r = self.with(|w, st| w.next(st));
        let Some(h) = self.hide else { return r };
        // SAFETY: our live hide state; the walk is not running, so nothing
        // else reaches it.
        let Some(p) = (unsafe { (*h.as_ptr()).panic.take() }) else {
            return r;
        };
        self.with(|w, st| w.reset(st));
        std::panic::resume_unwind(p)
    }

    /// The repository of the walk (`git_revwalk_repository`).
    pub fn repository(&self) -> &'repo Repository {
        self.repo
    }

    /// Free the hide state (after C stopped using it).
    fn drop_hide(&mut self) {
        if let Some(h) = self.hide.take() {
            // SAFETY: from `Box::into_raw`, no longer the walk's payload.
            drop(unsafe { Box::from_raw(h.as_ptr()) });
        }
    }

    /// `git_revwalk_sorting`: set the order (resets a walk in progress).
    pub fn sorting(&mut self, sort: Sort) -> &mut Self {
        self.with(|w, st| w.set_sorting(st, sort.raw()));
        self
    }

    /// `git_revwalk_simplify_first_parent`: follow only first parents.
    pub fn simplify_first_parent(&mut self) -> &mut Self {
        self.with(|_, st| st.first_parent.set(true));
        self
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
        self.with(|w, st| w.push_ref(st, &c, &opts))?;
        Ok(self)
    }

    /// `git_revwalk_push_ref`: start from what the reference points to.
    pub fn push_ref(&mut self, name: impl AsRef<[u8]>) -> Result<&mut Self> {
        self.push_refname(name.as_ref(), 0)
    }

    /// `git_revwalk_hide_ref`.
    pub fn hide_ref(&mut self, name: impl AsRef<[u8]>) -> Result<&mut Self> {
        self.push_refname(name.as_ref(), 1)
    }

    /// `git_revwalk_push_head`.
    pub fn push_head(&mut self) -> Result<&mut Self> {
        self.push_refname(b"HEAD", 0)
    }

    /// `git_revwalk_hide_head`.
    pub fn hide_head(&mut self) -> Result<&mut Self> {
        self.push_refname(b"HEAD", 1)
    }

    fn glob(&mut self, glob: &[u8], uninteresting: c_int) -> Result<&mut Self> {
        let c = cstring(glob)?;
        let opts = PushOptions {
            uninteresting,
            ..PushOptions::default()
        };
        self.with(|w, st| w.push_glob(st, &c, Some(&opts)))?;
        Ok(self)
    }

    /// `git_revwalk_push_glob`: start from every reference matching `glob`
    /// (`refs/` is implied; a glob without `?`, `*` or `[` gets `/*`).
    pub fn push_glob(&mut self, glob: impl AsRef<[u8]>) -> Result<&mut Self> {
        self.glob(glob.as_ref(), 0)
    }

    /// `git_revwalk_hide_glob`.
    pub fn hide_glob(&mut self, glob: impl AsRef<[u8]>) -> Result<&mut Self> {
        self.glob(glob.as_ref(), 1)
    }

    /// `git_revwalk_push_range`: `a..b` pushes `b` and hides `a`. The
    /// symmetric difference `a...b` is not supported (an
    /// `ErrorCode::INVALID_SPEC` error, as in C).
    pub fn push_range(&mut self, range: impl AsRef<[u8]>) -> Result<&mut Self> {
        let c = cstring(range.as_ref())?;
        self.with(|w, st| w.push_range(st, &c))?;
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
    /// progress. If `f` panics, the panic is caught where C calls it: the
    /// walk answers "hide" for the rest of that step, then resets, and the
    /// panic is resumed from the iteration step; the walk can be used
    /// again after it.
    pub fn set_hide_callback(&mut self, f: impl FnMut(&Oid) -> bool + 'repo) -> &mut Self {
        let state = Box::new(HideState {
            f: Box::new(f),
            panic: None,
        });
        let state = Box::into_non_null(state);
        let payload = state.as_ptr().cast::<c_void>();
        // SAFETY: `hide_trampoline` is sound with this payload, a live
        // `HideState` that is freed only after the walk lets go of it
        // (`drop_hide` after a replacement or the walk's free); it does not
        // reach the walk.
        self.with(|w, st| unsafe { w.set_hide_cb(st, Some(hide_trampoline), payload) });
        self.drop_hide();
        self.hide = Some(state);
        self
    }

    /// `git_revwalk_add_hide_cb(walk, NULL, NULL)`: remove the callback.
    pub fn clear_hide_callback(&mut self) -> &mut Self {
        // SAFETY: no callback.
        self.with(|w, st| unsafe { w.set_hide_cb(st, None, core::ptr::null_mut()) });
        self.drop_hide();
        self
    }
}

impl Iterator for Revwalk<'_> {
    type Item = Result<Oid>;

    /// `git_revwalk_next`: the next id; `None` at the end of the walk.
    fn next(&mut self) -> Option<Result<Oid>> {
        match self.step() {
            Ok(id) => Some(Ok(id)),
            Err(ErrorCode::ITER_OVER) => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}
