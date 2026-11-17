//! HEAD and the repository's state: `git_repository_head` and friends,
//! `set_head`/`detach_head`, the state files, the merge message,
//! `hashfile`, shallow roots, worktree iteration, commit parents.

use crate::util::cabi::CStrArg;
use crate::util::cabi::Out;
use core::ffi::{CStr, c_int, c_void};
use core::ptr::{self, NonNull};

use super::handles::{self, Object, RefIter, Reference, check, refname};
use super::*;
use crate::oid::Oid;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::fs_path::{self, str_c_bytes};
use crate::util::futils;
use crate::util::posix;
use crate::util::str::GitStr;
use crate::util::util as u;
use crate::{git_assert_arg, git_error_set};

/// `GIT_HEAD_REF`.
pub(crate) const GIT_HEAD_REF: &CStr = c"HEAD";
const GIT_ORIG_HEAD_REF: &CStr = c"ORIG_HEAD";
const GIT_REVERT_HEAD_REF: &CStr = c"REVERT_HEAD";
const GIT_CHERRYPICK_HEAD_REF: &CStr = c"CHERRY_PICK_HEAD";
const GIT_MERGE_MSG_FILE: &[u8] = b"MERGE_MSG";
const GIT_BRANCH_DEFAULT: &[u8] = b"master";
use crate::refs::GIT_REFS_HEADS_DIR;

use crate::odb::{GIT_OBJECT_ANY, GIT_OBJECT_COMMIT, GIT_OBJECT_TREE};

/// The `git_repository *` of `repo`, for the refs exports.
fn rp(repo: &GitRepository) -> *mut GitRepository {
    ptr::from_ref(repo).cast_mut()
}

/// `git_reference_lookup`.
pub(crate) fn lookup<'r>(repo: &'r GitRepository, name: &CStr) -> Result<Reference<'r>> {
    let mut out = ptr::null_mut();
    // SAFETY: a live repository, a NUL-terminated name, a local out.
    check(unsafe {
        crate::refs::ffi::git_reference_lookup(&raw mut out, rp(repo), name.as_ptr())
    })?;
    // SAFETY: the reference C returned to us.
    unsafe { Reference::from_raw(out) }.ok_or(ErrorCode::GENERIC)
}

/// `git_reference_lookup_resolved(…, -1)`.
fn lookup_resolved<'r>(repo: &'r GitRepository, name: &CStr) -> Result<Reference<'r>> {
    let mut out = ptr::null_mut();
    // SAFETY: as `lookup`.
    check(unsafe {
        crate::refs::ffi::git_reference_lookup_resolved(&raw mut out, rp(repo), name.as_ptr(), -1)
    })?;
    // SAFETY: as `lookup`.
    unsafe { Reference::from_raw(out) }.ok_or(ErrorCode::GENERIC)
}

/// `git_reference_create`.
fn reference_create(
    repo: &GitRepository,
    name: &CStr,
    id: &Oid,
    force: bool,
    log: Option<&CStr>,
) -> Result<()> {
    let mut out = ptr::null_mut();
    // SAFETY: a live repository, NUL-terminated strings (or NULL), a
    // valid id, a local out.
    check(unsafe {
        crate::refs::ffi::git_reference_create(
            &raw mut out,
            rp(repo),
            name.as_ptr(),
            id,
            c_int::from(force),
            CStrArg::from(log),
        )
    })?;
    // SAFETY: the new reference, ours to free.
    drop(unsafe { Reference::from_raw(out) });
    Ok(())
}

/// `git_reference_symbolic_create`.
fn reference_symbolic_create(
    repo: &GitRepository,
    name: &CStr,
    target: &CStr,
    force: bool,
    log: Option<&CStr>,
) -> Result<()> {
    let mut out = ptr::null_mut();
    // SAFETY: as `reference_create`.
    check(unsafe {
        crate::refs::ffi::git_reference_symbolic_create(
            &raw mut out,
            rp(repo),
            name.as_ptr(),
            target.as_ptr(),
            c_int::from(force),
            CStrArg::from(log),
        )
    })?;
    // SAFETY: the new reference, ours to free.
    drop(unsafe { Reference::from_raw(out) });
    Ok(())
}

/// `git_object_lookup`.
fn object_lookup<'r>(repo: &'r GitRepository, id: &Oid, t: c_int) -> Result<Object<'r>> {
    let mut out = ptr::null_mut();
    // SAFETY: `rp(repo)` is NULL or a live repository (as the call below requires), only read
    // through `&` (its state is interior-mutable).
    let rp_repo_ref = unsafe { rp(repo).as_ref() };
    // SAFETY: a live repository, a valid id, a local out.
    check(unsafe {
        crate::object::ffi::git_object_lookup(Out::from(&mut out), rp_repo_ref, Some(id), t)
    })?;
    let out = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the object the lookup returned to us, of `repo`.
    Ok(unsafe { Object::adopt(out) })
}

/// `git_repository_head`.
pub(crate) fn head(repo: &GitRepository) -> Result<Reference<'_>> {
    let head = lookup(repo, GIT_HEAD_REF)?;
    if head.kind() == handles::GIT_REFERENCE_DIRECT {
        return Ok(head);
    }
    let target = head
        .symbolic_target()
        .map(CStr::to_owned)
        .unwrap_or_default();
    drop(head);
    match lookup_resolved(repo, &target) {
        Err(ErrorCode::NOT_FOUND) => Err(ErrorCode::UNBORN_BRANCH),
        r => r,
    }
}

/// `git_repository_head_detached`: 1, 0 or -1.
pub(crate) fn head_detached(repo: &GitRepository) -> c_int {
    let Ok(odb) = repo.odb() else {
        return -1;
    };
    let Ok(r) = lookup(repo, GIT_HEAD_REF) else {
        return -1;
    };
    if r.is_symbolic() {
        return 0;
    }
    match r.target() {
        Some(id) => c_int::from(odb.exists_ext(id, 0)),
        // `git_odb_exists(odb, NULL)` fails its argument check.
        None => errors::assert_arg_failed("id").raw(),
    }
}

/// `git_repository_head_unborn`: 1, 0 or -1.
pub(crate) fn head_unborn(repo: &GitRepository) -> c_int {
    match head(repo) {
        Ok(_) => 0,
        Err(ErrorCode::UNBORN_BRANCH) => {
            errors::clear();
            1
        }
        Err(_) => -1,
    }
}

/// `git_repository_open_from_worktree`.
pub(crate) fn open_from_worktree(gitlink_path: Option<&CStr>) -> Result<NonNull<GitRepository>> {
    let g = gitlink_path.map(CStr::to_bytes).unwrap_or_default();
    let Some(path) = open::worktree_repo_path(g) else {
        return Err(ErrorCode::GENERIC);
    };
    let path = fs_path::to_cstring(path);
    open::open_ext(Some(&path), GIT_REPOSITORY_OPEN_NO_SEARCH, None, true)?
        .ok_or(ErrorCode::GENERIC)
}

/// A repository this module opened and frees.
pub(crate) struct OwnedRepo(NonNull<GitRepository>);

impl OwnedRepo {
    /// Own `p`.
    ///
    /// # Safety
    ///
    /// `p` is a live repository (from `GitRepository::alloc`, or one C
    /// opened) that the caller owns and gives up: the owner frees it.
    pub(crate) unsafe fn new(p: NonNull<GitRepository>) -> OwnedRepo {
        OwnedRepo(p)
    }
    pub(crate) fn get(&self) -> &GitRepository {
        // SAFETY: the repository we opened and own, live until drop.
        unsafe { self.0.as_ref() }
    }
}

impl Drop for OwnedRepo {
    fn drop(&mut self) {
        // SAFETY: ours alone.
        unsafe { GitRepository::free(self.0.as_ptr()) }
    }
}

/// `git_repository_head_for_worktree`: `f` gets the HEAD of the worktree
/// `name`, resolved, while that worktree's repository is open.
///
/// C returns the reference after freeing that repository; the reference
/// keeps its refdb alive, but the refdb's repository pointer dangles. The
/// export hands the raw reference to C as C did; Rust callers get it only
/// inside `f`.
pub(crate) fn with_head_for_worktree<T>(
    repo: &GitRepository,
    name: &CStr,
    f: impl FnOnce(Reference<'_>) -> T,
) -> Result<T> {
    let wt = crate::sys::worktree::Worktree::lookup(repo, name)?;
    let wrepo = OwnedRepo(open_from_worktree(wt.gitlink_path())?);
    let head = lookup(wrepo.get(), GIT_HEAD_REF)?;
    if head.kind() != handles::GIT_REFERENCE_DIRECT {
        let t = head
            .symbolic_target()
            .map(CStr::to_owned)
            .unwrap_or_default();
        return Ok(f(lookup_resolved(wrepo.get(), &t)?));
    }
    Ok(f(head))
}

/// `git_repository_head_detached_for_worktree`.
pub(crate) fn head_detached_for_worktree(repo: &GitRepository, name: &CStr) -> Result<bool> {
    with_head_for_worktree(repo, name, |r| r.kind() != handles::GIT_REFERENCE_SYMBOLIC)
}

/// `git_repository_foreach_worktree`: `cb` on the main repository and each
/// worktree's; the first non-zero result stops it and is returned.
pub(crate) fn foreach_worktree(
    repo: &GitRepository,
    cb: &mut dyn FnMut(&GitRepository) -> c_int,
) -> c_int {
    let Some(commondir) = repo.commondir() else {
        return cb(repo);
    };
    let main = match open::open_ext(Some(commondir), GIT_REPOSITORY_OPEN_NO_SEARCH, None, true) {
        // SAFETY: the repository `open_ext` just opened, ours.
        Ok(Some(r)) => unsafe { OwnedRepo::new(r) },
        Ok(None) => return -1,
        Err(e) => return e.raw(),
    };
    let e = cb(main.get());
    if e != 0 {
        return e;
    }
    drop(main);

    let worktrees = match crate::sys::worktree::list(repo) {
        Ok(w) => w,
        Err(e) => return e.raw(),
    };
    for name in worktrees.iter() {
        let opened = crate::sys::worktree::Worktree::lookup(repo, name)
            .and_then(|wt| open_from_worktree(wt.gitlink_path()));
        let wrepo = match opened {
            // SAFETY: the repository just opened, ours.
            Ok(r) => unsafe { OwnedRepo::new(r) },
            Err(ErrorCode::NOT_FOUND) => continue,
            Err(e) => return e.raw(),
        };
        let e = cb(wrepo.get());
        if e != 0 {
            return e;
        }
    }
    0
}

/// `repo_contains_no_reference`: 1, 0, or an error code.
fn repo_contains_no_reference(repo: &GitRepository) -> c_int {
    let mut it = ptr::null_mut();
    // SAFETY: a live repository and a local out.
    let e = unsafe { crate::refs::ffi::git_reference_iterator_new(&raw mut it, rp(repo)) };
    if e < 0 {
        return e;
    }
    // SAFETY: the iterator C made for us.
    let it = unsafe { RefIter::from_raw(it) };
    let Some(it) = it else {
        return -1;
    };
    let mut name = ptr::null();
    // SAFETY: a live iterator and a local out.
    let e = unsafe { crate::refs::ffi::git_reference_next_name(&raw mut name, it.as_ptr()) };
    drop(it);
    if e == ErrorCode::ITER_OVER.raw() {
        1
    } else {
        e
    }
}

/// `git_repository_initialbranch`.
pub(crate) fn initialbranch(out: &mut GitStr, repo: &GitRepository) -> Result<()> {
    let config = repo.config_weak()?;
    let entry = config.get_entry(c"init.defaultbranch", true, crate::config::Want::AllErrors);
    let branch: Vec<u8> = match &entry {
        Ok(Some(e)) if e.value().is_some_and(|v| !v.is_empty()) => {
            e.value().map(<[u8]>::to_vec).unwrap_or_default()
        }
        Ok(_) | Err(ErrorCode::NOT_FOUND) => GIT_BRANCH_DEFAULT.to_vec(),
        Err(e) => return Err(*e),
    };
    out.puts(GIT_REFS_HEADS_DIR)?;
    out.puts(&branch)?;
    if !refname::is_valid(&cs(out))? {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "the value of init.defaultBranch is not a valid branch name"
        ));
    }
    Ok(())
}

/// `git_repository_is_empty`: 1, 0, or an error code.
pub(crate) fn is_empty(repo: &GitRepository) -> c_int {
    let head = match lookup(repo, GIT_HEAD_REF) {
        Ok(h) => h,
        Err(e) => return e.raw(),
    };
    let mut initial = GitStr::new();
    if let Err(e) = initialbranch(&mut initial, repo) {
        return e.raw();
    }
    let r = head.is_symbolic()
        && head.symbolic_target().map(CStr::to_bytes) == Some(str_c_bytes(&initial))
        && repo_contains_no_reference(repo) != 0;
    c_int::from(r)
}

/// `checkout_message`.
fn checkout_message(out: &mut GitStr, old: &Reference, new: &CStr) -> Result<()> {
    let _ = out.puts(b"checkout: moving from ");
    if old.is_symbolic() {
        let t = old.symbolic_target().unwrap_or(c"");
        let _ = out.puts(refname::shorthand(t).to_bytes());
    } else {
        let id = old.target().ok_or(ErrorCode::GENERIC)?;
        let _ = out.puts(id.hex().as_bytes());
    }
    let _ = out.puts(b" to ");
    if refname::is_branch(new) || refname::is_tag(new) || refname::is_remote(new) {
        let _ = out.puts(refname::shorthand(new).to_bytes());
    } else {
        let _ = out.puts(new.to_bytes());
    }
    if out.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    Ok(())
}

/// `detach`.
fn detach(repo: &GitRepository, id: Option<&Oid>, new: Option<&CStr>) -> Result<()> {
    git_assert_arg!(id.is_some(), "id");
    let id = id.ok_or(ErrorCode::GENERIC)?;
    let current = lookup(repo, GIT_HEAD_REF)?;
    let object = object_lookup(repo, id, GIT_OBJECT_ANY)?;
    let peeled = object.peel_raw(GIT_OBJECT_COMMIT)?;
    let pid = *peeled.id();
    let hex;
    let new = match new {
        Some(n) => n,
        None => {
            hex = fs_path::to_cstring(pid.hex().as_bytes());
            &hex
        }
    };
    let mut log = GitStr::new();
    checkout_message(&mut log, &current, new)?;
    reference_create(repo, GIT_HEAD_REF, &pid, true, Some(&cs(&log)))
}

/// `git_repository_set_head`.
pub(crate) fn set_head(repo: &GitRepository, name: &CStr) -> Result<()> {
    let current = lookup(repo, GIT_HEAD_REF)?;
    let mut log = GitStr::new();
    checkout_message(&mut log, &current, name)?;
    let log = cs(&log).into_owned();

    let r = match lookup(repo, name) {
        Ok(r) => Some(r),
        Err(ErrorCode::NOT_FOUND) => None,
        Err(e) => return Err(e),
    };

    if let Some(r) = &r
        && current.is_symbolic()
        && current.symbolic_target().map(CStr::to_bytes) != Some(r.name().to_bytes())
        && r.is_branch()
        && crate::sys::branch::is_checked_out(r) != 0
    {
        return Err(git_error_set!(
            ErrorClass::REPOSITORY,
            "cannot set HEAD to reference '{}' as it is the current HEAD of a linked repository.",
            Bytes(r.name().to_bytes())
        ));
    }

    match r {
        Some(r) => {
            if r.is_branch() {
                reference_symbolic_create(repo, GIT_HEAD_REF, r.name(), true, Some(&log))
            } else {
                let n = if r.is_tag() || r.is_remote() {
                    Some(name)
                } else {
                    None
                };
                detach(repo, r.target(), n)
            }
        }
        None if refname::is_branch(name) => {
            reference_symbolic_create(repo, GIT_HEAD_REF, name, true, Some(&log))
        }
        None => Err(ErrorCode::NOT_FOUND),
    }
}

/// `git_repository_set_head_detached`.
pub(crate) fn set_head_detached(repo: &GitRepository, id: Option<&Oid>) -> Result<()> {
    detach(repo, id, None)
}

/// `git_repository_set_head_detached_from_annotated`.
///
/// # Safety
///
/// `commit` is a live `git_annotated_commit`.
pub(crate) unsafe fn set_head_detached_from_annotated(
    repo: &GitRepository,
    commit: NonNull<c_void>,
) -> Result<()> {
    // SAFETY: the caller's contract.
    let id = unsafe { crate::sys::annotated_commit::id(commit) };
    // SAFETY: the caller's contract.
    let desc = unsafe { crate::sys::annotated_commit::description(commit) };
    detach(repo, Some(id), desc)
}

/// `git_repository_detach_head`.
pub(crate) fn detach_head(repo: &GitRepository) -> Result<()> {
    let current = lookup(repo, GIT_HEAD_REF)?;
    let old_head = head(repo)?;
    let target = old_head
        .target()
        .copied()
        .ok_or_else(|| errors::assert_arg_failed("id"))?;
    let object = object_lookup(repo, &target, GIT_OBJECT_COMMIT)?;
    let idstr = fs_path::to_cstring(object.id().hex().as_bytes());
    let mut log = GitStr::new();
    checkout_message(&mut log, &current, &idstr)?;
    reference_create(repo, GIT_HEAD_REF, &target, true, Some(&cs(&log)))
}

/// `git_repository_state_t`.
pub mod state {
    use core::ffi::c_int;
    /// `GIT_REPOSITORY_STATE_NONE`.
    pub const NONE: c_int = 0;
    /// `GIT_REPOSITORY_STATE_MERGE`.
    pub const MERGE: c_int = 1;
    /// `GIT_REPOSITORY_STATE_REVERT`.
    pub const REVERT: c_int = 2;
    /// `GIT_REPOSITORY_STATE_REVERT_SEQUENCE`.
    pub const REVERT_SEQUENCE: c_int = 3;
    /// `GIT_REPOSITORY_STATE_CHERRYPICK`.
    pub const CHERRYPICK: c_int = 4;
    /// `GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE`.
    pub const CHERRYPICK_SEQUENCE: c_int = 5;
    /// `GIT_REPOSITORY_STATE_BISECT`.
    pub const BISECT: c_int = 6;
    /// `GIT_REPOSITORY_STATE_REBASE`.
    pub const REBASE: c_int = 7;
    /// `GIT_REPOSITORY_STATE_REBASE_INTERACTIVE`.
    pub const REBASE_INTERACTIVE: c_int = 8;
    /// `GIT_REPOSITORY_STATE_REBASE_MERGE`.
    pub const REBASE_MERGE: c_int = 9;
    /// `GIT_REPOSITORY_STATE_APPLY_MAILBOX`.
    pub const APPLY_MAILBOX: c_int = 10;
    /// `GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE`.
    pub const APPLY_MAILBOX_OR_REBASE: c_int = 11;
}

/// `git_repository_state`.
pub(crate) fn repo_state(repo: &GitRepository) -> Result<c_int> {
    let mut p = GitStr::new();
    p.puts(repo.gitdir().map(CStr::to_bytes).unwrap_or_default())?;
    let s = if fs_path::contains_file(&mut p, b"rebase-merge/interactive") {
        state::REBASE_INTERACTIVE
    } else if fs_path::contains_dir(&mut p, b"rebase-merge/") {
        state::REBASE_MERGE
    } else if fs_path::contains_file(&mut p, b"rebase-apply/rebasing") {
        state::REBASE
    } else if fs_path::contains_file(&mut p, b"rebase-apply/applying") {
        state::APPLY_MAILBOX
    } else if fs_path::contains_dir(&mut p, b"rebase-apply/") {
        state::APPLY_MAILBOX_OR_REBASE
    } else if fs_path::contains_file(&mut p, b"MERGE_HEAD") {
        state::MERGE
    } else if lookup(repo, GIT_REVERT_HEAD_REF).is_ok() {
        if fs_path::contains_file(&mut p, b"sequencer/todo") {
            state::REVERT_SEQUENCE
        } else {
            state::REVERT
        }
    } else if lookup(repo, GIT_CHERRYPICK_HEAD_REF).is_ok() {
        if fs_path::contains_file(&mut p, b"sequencer/todo") {
            state::CHERRYPICK_SEQUENCE
        } else {
            state::CHERRYPICK
        }
    } else if fs_path::contains_file(&mut p, b"BISECT_LOG") {
        state::BISECT
    } else {
        state::NONE
    };
    Ok(s)
}

/// `p_unlink`: 0 or -1 (errno set, no error message).
fn p_unlink(path: &CStr) -> c_int {
    // SAFETY: a NUL-terminated path.
    unsafe { libc::unlink(path.as_ptr()) }
}

/// `git_repository__cleanup_files`.
pub(crate) fn cleanup_files(repo: &GitRepository, files: &[&[u8]]) -> c_int {
    let gitdir = repo.gitdir().map(CStr::to_bytes).unwrap_or_default();
    let mut buf = GitStr::new();
    let mut error = 0;
    for f in files {
        if error != 0 {
            break;
        }
        if buf.joinpath(gitdir, f).is_err() {
            return -1;
        }
        let path = cs(&buf).into_owned();
        if fs_path::isfile(&path) {
            error = p_unlink(&path);
        } else if fs_path::isdir(&path) {
            error = match futils::rmdir_r(
                path.to_bytes(),
                None,
                futils::GIT_RMDIR_REMOVE_FILES | futils::GIT_RMDIR_REMOVE_BLOCKERS,
            ) {
                Ok(()) => 0,
                Err(e) => e.raw(),
            };
        }
        buf.clear();
    }
    error
}

const STATE_FILES: [&[u8]; 7] = [
    b"MERGE_HEAD",
    b"MERGE_MODE",
    b"MERGE_MSG",
    b"BISECT_LOG",
    b"rebase-merge/",
    b"rebase-apply/",
    b"sequencer/",
];

/// `git_repository_state_cleanup`.
pub(crate) fn state_cleanup(repo: &GitRepository) -> Result<()> {
    ErrorCode::check(cleanup_files(repo, &STATE_FILES))?;
    for r in [GIT_REVERT_HEAD_REF, GIT_CHERRYPICK_HEAD_REF] {
        // SAFETY: a live repository and a NUL-terminated name.
        match check(unsafe { crate::refs::ffi::git_reference_remove(rp(repo), r.into()) }) {
            Ok(_) | Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// `git_repository__message`.
pub(crate) fn message(out: &mut GitStr, repo: &GitRepository) -> Result<()> {
    let mut path = GitStr::new();
    path.joinpath(
        repo.gitdir().map(CStr::to_bytes).unwrap_or_default(),
        GIT_MERGE_MSG_FILE,
    )
    .map_err(|_| ErrorCode::GENERIC)?;
    let p = cs(&path).into_owned();
    match posix::stat(&p) {
        Err(posix::Errno(e)) => {
            let code = if e == libc::ENOENT {
                ErrorCode::NOT_FOUND
            } else {
                ErrorCode::GENERIC
            };
            let _ = git_error_set!(ErrorClass::OS, "could not access message file");
            Err(code)
        }
        Ok(_) => futils::readbuffer(out, &p),
    }
}

/// `git_repository_message_remove`: 0 or -1.
pub(crate) fn message_remove(repo: &GitRepository) -> c_int {
    let mut path = GitStr::new();
    if path
        .joinpath(
            repo.gitdir().map(CStr::to_bytes).unwrap_or_default(),
            GIT_MERGE_MSG_FILE,
        )
        .is_err()
    {
        return -1;
    }
    p_unlink(&cs(&path))
}

/// `git_repository_hashfile`.
pub(crate) fn hashfile(
    repo: &GitRepository,
    path: &CStr,
    object_type: c_int,
    as_path: Option<&CStr>,
) -> Result<Oid> {
    let workdir = repo.workdir();
    let mut full_path = GitStr::new();
    fs_path::join_unrooted(&mut full_path, path.to_bytes(), workdir.map(CStr::to_bytes))?;
    crate::sys::path::validate_str_length(Some(repo), &full_path)?;

    let full = cs(&full_path).into_owned();
    let as_path: &[u8] = match as_path {
        Some(a) => a.to_bytes(),
        None => match workdir {
            Some(w) if u::prefixcmp(full.to_bytes(), w.to_bytes()) == 0 => {
                &full.to_bytes()[w.to_bytes().len()..]
            }
            _ => b"",
        },
    };

    let fl = if !as_path.is_empty() {
        let ap = fs_path::to_cstring(as_path);
        crate::sys::filter::FilterList::load(
            repo,
            &ap,
            crate::sys::filter::GIT_FILTER_TO_ODB,
            crate::sys::filter::GIT_FILTER_DEFAULT,
        )?
    } else {
        crate::sys::filter::FilterList::none()
    };

    let fd = futils::open_ro(&full)?;
    struct Fd(c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: the descriptor this guard opened and owns.
            let _ = unsafe { posix::close(self.0) };
        }
    }
    let fd = Fd(fd);
    let len = futils::filesize(fd.0)?;
    let Ok(len) = usize::try_from(len) else {
        return Err(git_error_set!(
            ErrorClass::OS,
            "file size overflow for 32-bit systems"
        ));
    };
    let opts = crate::object::GitObjectIdOptions {
        version: crate::object::GIT_OBJECT_ID_OPTIONS_VERSION,
        object_type,
        oid_type: repo.oid_type_raw(),
        filters: fl.as_ptr().cast(),
    };
    let mut out = Oid::zero(repo.oid_type());
    // SAFETY: an open descriptor of `len` bytes, valid options whose filter
    // list lives across the call, and a local out-parameter.
    check(unsafe {
        crate::object::ffi::git_object_id_from_fd(Out::from(&mut out), fd.0, len, Some(&opts))
    })?;
    Ok(out)
}

/// `git_repository_head_commit` / `_tree`: HEAD peeled to `t`.
pub(crate) fn head_peeled(repo: &GitRepository, t: c_int) -> Result<Object<'_>> {
    let h = head(repo)?;
    h.peel(t)
}

/// `GIT_OBJECT_COMMIT` / `GIT_OBJECT_TREE`, for [`head_peeled`].
pub(crate) const PEEL_COMMIT: c_int = GIT_OBJECT_COMMIT;
/// See [`PEEL_COMMIT`].
pub(crate) const PEEL_TREE: c_int = GIT_OBJECT_TREE;

/// `git_repository__set_orig_head`.
pub(crate) fn set_orig_head(repo: &GitRepository, id: &Oid) -> Result<()> {
    reference_create(repo, GIT_ORIG_HEAD_REF, id, true, None)
}

/// `git_repository__shallow_roots`: an array of `git__allocator` memory.
pub(crate) fn shallow_roots(repo: &GitRepository) -> Result<(*mut Oid, usize)> {
    if repo.shallow_grafts_ptr().is_null() {
        open::load_grafts(repo)?;
    }
    // SAFETY: the repository's live shallow grafts.
    check(unsafe { crate::grafts::ffi::git_grafts_refresh(repo.shallow_grafts_ptr()) })?;
    let (mut out, mut len) = (ptr::null_mut(), 0usize);
    // SAFETY: as above, and local outs.
    check(unsafe {
        crate::grafts::ffi::git_grafts_oids(
            &raw mut out,
            Out::from(&mut len),
            repo.shallow_grafts_ptr(),
        )
    })?;
    Ok((out, len))
}

/// `git_repository__shallow_roots_write`.
pub(crate) fn shallow_roots_write(repo: &GitRepository, roots: &[Oid]) -> Result<()> {
    use crate::util::filebuf::{Filebuf, GIT_FILEBUF_HASH_SHA1, GIT_FILEBUF_HASH_SHA256};
    let hash_flags = match repo.oid_type() {
        OidType::Sha1 => GIT_FILEBUF_HASH_SHA1,
        OidType::Sha256 => GIT_FILEBUF_HASH_SHA256,
    };
    let mut path = GitStr::new();
    path.joinpath(
        repo.gitdir().map(CStr::to_bytes).unwrap_or_default(),
        b"shallow",
    )?;
    let p = cs(&path).into_owned();
    let mut file = Filebuf::open(p.to_bytes(), hash_flags, 0o666).map_err(|e| e.raw_code())?;
    let hexsize = repo.oid_type().hexsize();
    for r in roots {
        // `git_oid_tostr(buf, sizeof(buf), id)`, then `hexsize` bytes of it.
        let hex = r.hex();
        let mut b = [0u8; 64];
        let h = hex.as_bytes();
        b[..h.len()].copy_from_slice(h);
        let _ = file.write(&b[..hexsize]);
        let _ = file.write(b"\n");
    }
    // As C, a failed commit is not reported (it cleans the filebuf up).
    let _ = file.commit();
    if open::load_grafts(repo).is_err() {
        return Err(ErrorCode::GENERIC);
    }
    if roots.is_empty() {
        // SAFETY: a NUL-terminated path.
        unsafe { libc::remove(p.as_ptr()) };
    }
    Ok(())
}

/// `git_repository_is_shallow`.
pub(crate) fn is_shallow(repo: &GitRepository) -> Result<bool> {
    let mut path = GitStr::new();
    path.joinpath(
        repo.commondir().map(CStr::to_bytes).unwrap_or_default(),
        b"shallow",
    )?;
    match fs_path::lstat(&cs(&path)) {
        Err(ErrorCode::NOT_FOUND) => {
            errors::clear();
            Ok(false)
        }
        Err(e) => Err(e),
        Ok(st) => Ok(st.st_size != 0),
    }
}

/// `git_repository__abbrev_length`.
pub(crate) fn abbrev_length(repo: &GitRepository) -> Result<c_int> {
    let hexsize = crate::oid::oid_hexsize(repo.oid_type_raw());
    let len = repo.configmap_lookup(GIT_CONFIGMAP_ABBREV)?;
    if len < GIT_ABBREV_MINIMUM {
        return Err(git_error_set!(
            ErrorClass::CONFIG,
            "invalid oid abbreviation setting: '{}'",
            len
        ));
    }
    if len == GIT_ABBREV_FALSE || len as usize > hexsize {
        return Ok(hexsize as c_int);
    }
    Ok(len)
}

/// `git_repository__set_objectformat`.
///
/// # Safety
///
/// When the type changes, the index, odb and refdb are released: no weak
/// pointer to them is used after the call.
pub(crate) unsafe fn set_objectformat(
    repo: &GitRepository,
    oid_type: crate::oid::git_oid_t,
) -> Result<()> {
    if oid_type == GIT_OID_DEFAULT {
        return Ok(());
    }
    if is_empty(repo) == 0 && repo.oid_type_raw() != oid_type {
        return Err(git_error_set!(
            ErrorClass::REPOSITORY,
            "cannot change object id type of existing repository"
        ));
    }
    let cfg = repo.config_weak().map_err(|_| ErrorCode::GENERIC)?;
    let name = OidType::from_raw(oid_type).map_or("unknown", OidType::name);
    let name = fs_path::to_cstring(name.as_bytes());
    if cfg.set_int32(c"core.repositoryformatversion", 1).is_err()
        || cfg
            .set_string(c"extensions.objectformat", Some(&name))
            .is_err()
    {
        return Err(ErrorCode::GENERIC);
    }
    if repo.oid_type_raw() != oid_type {
        // SAFETY: the caller's contract.
        unsafe { repo.set_index(None) };
        // SAFETY: as above.
        unsafe { repo.set_odb(None) };
        // SAFETY: as above.
        unsafe { repo.set_refdb(None) };
        repo.set_oid_type_raw(oid_type);
    }
    Ok(())
}

/// `git_repository_workdir_path`.
pub(crate) fn workdir_path(out: &mut GitStr, repo: &GitRepository, path: &[u8]) -> Result<()> {
    let Some(wd) = repo.workdir_field() else {
        let _ = git_error_set!(
            ErrorClass::REPOSITORY,
            "repository has no working directory"
        );
        return Err(ErrorCode::BARE_REPO);
    };
    out.joinpath(wd.to_bytes(), path)?;
    crate::sys::path::validate_str_length(Some(repo), out)
}

/// `git_repository_commit_parents`: the commits (`git_commit *`, owned).
pub(crate) fn commit_parents(
    repo: &GitRepository,
) -> Result<Vec<NonNull<c_void>, crate::util::alloc::GitAlloc>> {
    let mut parents: Vec<NonNull<c_void>, crate::util::alloc::GitAlloc> =
        Vec::new_in(crate::util::alloc::GIT_ALLOC);
    let (e, first, head_ref) = crate::sys::revparse::ext(repo, c"HEAD");
    drop(head_ref);
    if e != 0 {
        drop(first);
        return if e == ErrorCode::NOT_FOUND.raw() {
            Ok(parents)
        } else {
            Err(ErrorCode::check(e).err().unwrap_or(ErrorCode::GENERIC))
        };
    }
    let Some(first) = first else {
        return Err(ErrorCode::GENERIC);
    };
    if parents.try_reserve(1).is_err() {
        return Err(errors::oom());
    }
    // SAFETY: an object's pointer is not NULL.
    parents.push(unsafe { NonNull::new_unchecked(first.into_raw().cast()) });

    struct Data<'a> {
        repo: &'a GitRepository,
        parents: &'a mut Vec<NonNull<c_void>, crate::util::alloc::GitAlloc>,
    }
    unsafe extern "C" fn insert_mergehead(oid: *const Oid, payload: *mut c_void) -> c_int {
        crate::util::export::barrier(|| {
            // SAFETY: the `Data` of the running call, used by nothing else
            // during the callback.
            let data = unsafe { &mut *payload.cast::<Data<'_>>() };
            // SAFETY: C's valid id.
            let oid = unsafe { &*oid };
            let mut commit: *mut crate::object::GitObject = ptr::null_mut();
            // SAFETY: `rp(data.repo)` is NULL or a live repository (as the call below requires),
            // only read through `&` (its state is interior-mutable).
            let rp_data_repo_ref = unsafe { rp(data.repo).as_ref() };
            // SAFETY: a live repository, a valid id, a local out.
            if unsafe {
                crate::object_api::ffi::git_commit_lookup(
                    Out::from(&mut commit),
                    rp_data_repo_ref,
                    Some(oid),
                )
            } < 0
            {
                return -1;
            }
            let Some(commit) = NonNull::new(commit.cast::<c_void>()) else {
                return -1;
            };
            if data.parents.try_reserve(1).is_err() {
                // C leaks here too; as C, the commit is not freed.
                let _ = errors::oom();
                return -1;
            }
            data.parents.push(commit);
            0
        })
    }
    let mut data = Data {
        repo,
        parents: &mut parents,
    };
    // SAFETY: `insert_mergehead` treats the payload as the `Data` that
    // outlives the call.
    let e = unsafe {
        crate::sys::merge::mergehead_foreach(repo, insert_mergehead, (&raw mut data).cast())
    };
    if e != 0 && e != ErrorCode::NOT_FOUND.raw() {
        // C frees the commits with `git__free`, which corrupts the object
        // cache (DEFECTS.md); they are released with `git_object_free`.
        while let Some(p) = parents.pop() {
            // SAFETY: the commits we looked up, each released once.
            unsafe { crate::object::ffi::git_object_free(p.as_ptr().cast()) };
        }
        return Err(ErrorCode::check(e).err().unwrap_or(ErrorCode::GENERIC));
    }
    Ok(parents)
}
