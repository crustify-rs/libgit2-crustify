//! Initializing repositories (`git_repository_init_ext` and its helpers),
//! the filesystem probes of `core.*`, and the setters that rewrite the
//! configuration (`set_workdir`, `set_bare`, `reinit_filesystem`).

use core::ffi::{CStr, c_int, c_void};
use core::ptr::NonNull;

use super::handles::{Config, ConfigRef, Refdb};
use super::head::OwnedRepo;
use super::*;
use crate::git_error_set;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::fs_path::{self, str_c_bytes};
use crate::util::futils::{self, MkdirOpts};
use crate::util::posix;
use crate::util::str::GitStr;
use crate::util::util as u;

const GIT_CONFIG_FILENAME_INREPO: &[u8] = b"config";
const GIT_CONFIG_FILE_MODE: u32 = 0o666;
const GIT_FILE_CONTENT_PREFIX: &str = "gitdir:";
const GIT_HEAD_FILE: &[u8] = b"HEAD";
const GIT_REFS_DIR: &[u8] = b"refs/";
const GIT_REFS_HEADS_DIR: &[u8] = b"refs/heads/";
const GIT_REFS_FILE_MODE: u32 = 0o666;
const GIT_BRANCH_DEFAULT: &[u8] = b"master";
const GIT_INVALID_HEAD_CONTENT: &[u8] = b"ref: refs/heads/.invalid";
const GIT_DESC_FILE: &[u8] = b"description";
const GIT_REFDB_BACKEND_INIT_FORCE_HEAD: u32 = 1 << 1;

/// `git_repository_init_options`, as the Rust uses it (the export reads
/// the C struct into one).
#[derive(Clone, Debug, Default)]
pub struct InitOptions {
    /// `flags` (with the internal `__` bits this module adds).
    pub flags: u32,
    /// `mode`.
    pub mode: u32,
    /// `workdir_path`.
    pub workdir_path: Option<CString>,
    /// `description`.
    pub description: Option<CString>,
    /// `template_path`.
    pub template_path: Option<CString>,
    /// `initial_head`.
    pub initial_head: Option<CString>,
    /// `origin_url`.
    pub origin_url: Option<CString>,
    /// `oid_type` (0 for the default).
    pub oid_type: crate::oid::git_oid_t,
    /// `refdb_type` (0 for the default).
    pub refdb_type: c_int,
}

use std::ffi::CString;

/// The internal template (`repo_template.h`): path, mode, content.
type TemplateItem = (&'static [u8], u32, Option<&'static [u8]>);

const REPO_TEMPLATE: [TemplateItem; 7] = [
    (b"objects/info/", 0o777, None),
    (b"objects/pack/", 0o777, None),
    (b"hooks/", 0o777, None),
    (b"info/", 0o777, None),
    (
        b"description",
        0o666,
        Some(b"Unnamed repository; edit this file 'description' to name the repository.\n"),
    ),
    (
        b"hooks/README.sample",
        0o777,
        Some(
            b"#!/bin/sh\n#\n# Place appropriately named executable hook scripts into this directory\n# to intercept various actions that git takes.  See `git help hooks` for\n# more information.\n",
        ),
    ),
    (
        b"info/exclude",
        0o666,
        Some(
            b"# File patterns to ignore; see `git help ignore` for more information.\n# Lines that start with '#' are comments.\n",
        ),
    ),
];

/// `is_chmod_supported`.
fn is_chmod_supported(file_path: &CStr) -> bool {
    let Ok(st1) = posix::stat(file_path) else {
        return false;
    };
    // SAFETY: a NUL-terminated path.
    if unsafe { libc::chmod(file_path.as_ptr(), st1.st_mode ^ libc::S_IXUSR) } < 0 {
        return false;
    }
    let Ok(st2) = posix::stat(file_path) else {
        return false;
    };
    st1.st_mode != st2.st_mode
}

/// `is_filesystem_case_insensitive`.
fn is_filesystem_case_insensitive(gitdir_path: &[u8]) -> bool {
    let mut path = GitStr::new();
    if path.joinpath(gitdir_path, b"CoNfIg").is_err() {
        // C: -1, which is true.
        return true;
    }
    fs_path::exists(&cs(&path))
}

/// `are_symlinks_supported` (Unix: probe the filesystem).
fn are_symlinks_supported(wd_path: &[u8], _use_env: bool) -> bool {
    fs_path::supports_symlinks(wd_path)
}

/// `create_empty_file`.
fn create_empty_file(path: &CStr, mode: u32) -> Result<()> {
    let fd = match posix::creat(path, mode as libc::mode_t) {
        Ok(fd) => fd,
        Err(_) => {
            return Err(git_error_set!(
                ErrorClass::OS,
                "error while creating '{}'",
                Bytes(path.to_bytes())
            ));
        }
    };
    // SAFETY: the descriptor just created, ours.
    if unsafe { posix::close(fd) }.is_err() {
        return Err(git_error_set!(
            ErrorClass::OS,
            "error while closing '{}'",
            Bytes(path.to_bytes())
        ));
    }
    Ok(())
}

/// `repo_local_config`: the repository's own configuration file (made if
/// missing), opened alone (no repository) or as the local level of the
/// repository's configuration.
fn repo_local_config<'r>(
    config_dir: &mut GitStr,
    repo: Option<&'r GitRepository>,
    repo_dir: &[u8],
) -> Result<Config<'r>> {
    config_dir
        .joinpath(repo_dir, GIT_CONFIG_FILENAME_INREPO)
        .map_err(|_| ErrorCode::GENERIC)?;
    let cfg_path = cs(config_dir).into_owned();

    if !fs_path::isfile(&cfg_path) {
        create_empty_file(&cfg_path, GIT_CONFIG_FILE_MODE)?;
    }

    let Some(repo) = repo else {
        return Config::open_ondisk(&cfg_path);
    };

    let parent = repo.config_weak()?;
    // C frees the weak pointer it got here (`git_config_free(parent)`),
    // dropping a reference it does not own (DEFECTS.md); the Rust does not.
    match parent.open_level(GIT_CONFIG_LEVEL_LOCAL) {
        Ok(c) => Ok(c),
        Err(_) => {
            errors::clear();
            parent.add_file_ondisk(&cfg_path, GIT_CONFIG_LEVEL_LOCAL, repo.as_ptr(), false)?;
            parent.open_level(GIT_CONFIG_LEVEL_LOCAL)
        }
    }
}

/// `repo_init_fs_configs`.
fn repo_init_fs_configs(
    cfg: ConfigRef<'_>,
    cfg_path: &CStr,
    repo_dir: &[u8],
    work_dir: Option<&[u8]>,
    update_ignorecase: bool,
    use_env: bool,
) -> Result<()> {
    let work_dir = work_dir.unwrap_or(repo_dir);
    cfg.set_bool(c"core.filemode", is_chmod_supported(cfg_path))?;

    if !are_symlinks_supported(work_dir, use_env) {
        cfg.set_bool(c"core.symlinks", false)?;
    } else if cfg.delete_entry(c"core.symlinks").is_err() {
        errors::clear();
    }

    if update_ignorecase {
        if is_filesystem_case_insensitive(repo_dir) {
            cfg.set_bool(c"core.ignorecase", true)?;
        } else if cfg.delete_entry(c"core.ignorecase").is_err() {
            errors::clear();
        }
    }
    // No `GIT_I18N_ICONV` in this configuration: `core.precomposeunicode`
    // is not set.
    Ok(())
}

/// `repo_init_config`.
fn repo_init_config(
    repo_dir: &[u8],
    work_dir: Option<&[u8]>,
    flags: u32,
    mode: u32,
    oid_type: crate::oid::git_oid_t,
    refdb_type: c_int,
) -> Result<()> {
    let is_bare = flags & GIT_REPOSITORY_INIT_BARE != 0;
    let is_reinit = flags & GIT_REPOSITORY_INIT__IS_REINIT != 0;
    let use_env = flags & GIT_REPOSITORY_OPEN_FROM_ENV != 0;
    let mut version = 0;

    let mut cfg_path = GitStr::new();
    let config = repo_local_config(&mut cfg_path, None, repo_dir)?;
    let c = config.borrow();

    if is_reinit {
        open::check_repositoryformatversion(&mut version, c)?;
    }
    open::check_extensions(Some(c), version)?;

    c.set_bool(c"core.bare", is_bare)?;
    c.set_int32(c"core.repositoryformatversion", version)?;

    repo_init_fs_configs(c, &cs(&cfg_path), repo_dir, work_dir, !is_reinit, use_env)?;

    if !is_bare {
        c.set_bool(c"core.logallrefupdates", true)?;
        if flags & GIT_REPOSITORY_INIT__NATURAL_WD == 0 {
            let mut worktree_path = GitStr::new();
            worktree_path.sets(work_dir.unwrap_or_default())?;
            if flags & GIT_REPOSITORY_INIT_RELATIVE_GITLINK != 0 {
                fs_path::make_relative(&mut worktree_path, repo_dir)?;
            }
            c.set_string(c"core.worktree", &cs(&worktree_path))?;
        } else if is_reinit && c.delete_entry(c"core.worktree").is_err() {
            errors::clear();
        }
    }

    if mode == GIT_REPOSITORY_INIT_SHARED_GROUP {
        c.set_int32(c"core.sharedrepository", 1)?;
        c.set_bool(c"receive.denyNonFastforwards", true)?;
    } else if mode == GIT_REPOSITORY_INIT_SHARED_ALL {
        c.set_int32(c"core.sharedrepository", 2)?;
        c.set_bool(c"receive.denyNonFastforwards", true)?;
    }

    if oid_type != GIT_OID_DEFAULT {
        c.set_int32(c"core.repositoryformatversion", 1)?;
        let name =
            crate::oid::OidType::from_raw(oid_type).map_or("unknown", crate::oid::OidType::name);
        c.set_string(
            c"extensions.objectformat",
            &fs_path::to_cstring(name.as_bytes()),
        )?;
    }

    if refdb_type != GIT_REFDB_FILES {
        c.set_int32(c"core.repositoryformatversion", 1)?;
        c.set_string(c"extensions.refstorage", open::refdb_type_name(refdb_type))?;
    }
    Ok(())
}

/// `git_repository_reinit_filesystem`.
pub(crate) fn reinit_filesystem(repo: &GitRepository, recurse: bool) -> Result<()> {
    let repo_dir = repo
        .gitdir()
        .map(CStr::to_bytes)
        .unwrap_or_default()
        .to_vec();
    let mut path = GitStr::new();
    let r = repo_local_config(&mut path, Some(repo), &repo_dir).and_then(|config| {
        let wd = repo.workdir().map(|w| w.to_bytes().to_vec());
        repo_init_fs_configs(
            config.borrow(),
            &cs(&path),
            &repo_dir,
            wd.as_deref(),
            true,
            repo.use_env(),
        )
    });
    repo.configmap_cache_clear();

    if !repo.is_bare() && recurse {
        unsafe extern "C" fn reinit_submodule_fs(
            sm: *mut c_void,
            _name: *const core::ffi::c_char,
            _payload: *mut c_void,
        ) -> c_int {
            crate::util::export::barrier(|| {
                let mut smrepo = core::ptr::null_mut();
                // SAFETY: the live submodule C passes, a local out.
                let opened = unsafe { crate::sys::submodule::open(&mut smrepo, sm) } >= 0;
                let smrepo = NonNull::new(smrepo.cast::<GitRepository>()).map(OwnedRepo::new);
                match (&smrepo, opened) {
                    (Some(r), true) => {
                        if reinit_filesystem(r.get(), true).is_err() {
                            errors::clear();
                        }
                    }
                    _ => errors::clear(),
                }
                0
            })
        }
        // SAFETY: the callback only uses the submodule C passes.
        let _ = unsafe {
            crate::sys::submodule::foreach(repo, reinit_submodule_fs, core::ptr::null_mut())
        };
    }
    r
}

/// `repo_write_template`.
fn repo_write_template(
    git_dir: &[u8],
    allow_overwrite: bool,
    file: &[u8],
    mode: u32,
    content: &[u8],
) -> Result<()> {
    let mut path = GitStr::new();
    path.joinpath(git_dir, file)
        .map_err(|_| ErrorCode::GENERIC)?;
    let flags = if allow_overwrite {
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC
    } else {
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
    };
    let mut error = false;
    match posix::open(&cs(&path), flags, mode as libc::mode_t) {
        Ok(fd) => {
            error = posix::write(fd, content).is_err();
            // SAFETY: the descriptor just opened, ours.
            let _ = unsafe { posix::close(fd) };
        }
        Err(posix::Errno(e)) => {
            if e != libc::EEXIST {
                error = true;
            }
        }
    }
    if error {
        return Err(git_error_set!(
            ErrorClass::OS,
            "failed to initialize repository with template '{}'",
            Bytes(file)
        ));
    }
    Ok(())
}

/// `repo_write_gitlink`: write `in_dir/.git` pointing to `to_repo`;
/// `GIT_PASSTHROUGH` when it would be the natural `.git` directory.
fn repo_write_gitlink(in_dir: &[u8], to_repo: &[u8], use_relative_path: bool) -> Result<()> {
    let mut buf = GitStr::new();
    let _ = fs_path::dirname_r(Some(&mut buf), Some(to_repo));
    let _ = fs_path::to_dir(&mut buf);
    if buf.is_oom() {
        return Err(ErrorCode::GENERIC);
    }

    if u::suffixcmp(to_repo, b"/.git/") == 0 && in_dir == str_c_bytes(&buf) {
        return Err(ErrorCode::PASSTHROUGH);
    }

    buf.joinpath(in_dir, DOT_GIT)?;
    if let Ok(st) = posix::stat(&cs(&buf))
        && st.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        let _ = git_error_set!(
            ErrorClass::REPOSITORY,
            "cannot overwrite gitlink file into path '{}'",
            Bytes(in_dir)
        );
        return Err(ErrorCode::EXISTS);
    }
    buf.clear();

    let mut path_to_repo = GitStr::new();
    path_to_repo.sets(to_repo)?;
    if use_relative_path {
        fs_path::make_relative(&mut path_to_repo, in_dir)?;
    }
    let p = str_c_bytes(&path_to_repo);
    buf.printf(
        6,
        format_args!("{} {}\n", GIT_FILE_CONTENT_PREFIX, Bytes(p)),
    )?;
    repo_write_template(in_dir, true, DOT_GIT, 0o666, str_c_bytes(&buf))
}

/// `pick_dir_mode`.
fn pick_dir_mode(opts: &InitOptions) -> u32 {
    match opts.mode {
        GIT_REPOSITORY_INIT_SHARED_UMASK => 0o777,
        GIT_REPOSITORY_INIT_SHARED_GROUP => 0o775 | libc::S_ISGID,
        GIT_REPOSITORY_INIT_SHARED_ALL => 0o777 | libc::S_ISGID,
        m => m,
    }
}

/// `repo_init_structure`.
fn repo_init_structure(repo_dir: &[u8], work_dir: Option<&[u8]>, opts: &InitOptions) -> Result<()> {
    let mut external_tpl =
        opts.template_path.is_some() || opts.flags & GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE != 0;
    let dmode = pick_dir_mode(opts);
    let chmod = opts.mode != GIT_REPOSITORY_INIT_SHARED_UMASK;

    if opts.flags & GIT_REPOSITORY_INIT_BARE == 0
        && opts.flags & GIT_REPOSITORY_INIT__NATURAL_WD == 0
    {
        match repo_write_gitlink(
            work_dir.unwrap_or_default(),
            repo_dir,
            opts.flags & GIT_REPOSITORY_INIT_RELATIVE_GITLINK != 0,
        ) {
            Ok(()) => {}
            Err(_) => return Err(ErrorCode::GENERIC),
        }
    }

    let mut error: Result<()> = Ok(());
    if external_tpl {
        let mut template_buf = GitStr::new();
        let mut default_template = false;
        let mut tdir: Option<Vec<u8>> = None;
        if let Some(t) = &opts.template_path {
            tdir = Some(t.to_bytes().to_vec());
        } else {
            match Config::open_default() {
                Ok(cfg) => {
                    if cfg
                        .borrow()
                        .get_path(&mut template_buf, c"init.templatedir")
                        .is_ok()
                    {
                        tdir = Some(str_c_bytes(&template_buf).to_vec());
                    }
                    errors::clear();
                }
                Err(e) => error = Err(e),
            }
        }
        if tdir.is_none() {
            error = crate::sys::sysdir::find_template_dir(&mut template_buf);
            if error.is_ok() {
                tdir = Some(str_c_bytes(&template_buf).to_vec());
            }
            default_template = true;
        }
        if let Some(t) = tdir.as_deref()
            && !t.is_empty()
        {
            let mut cpflags = futils::GIT_CPDIR_COPY_SYMLINKS
                | futils::GIT_CPDIR_SIMPLE_TO_MODE
                | futils::GIT_CPDIR_COPY_DOTFILES;
            if opts.mode != GIT_REPOSITORY_INIT_SHARED_UMASK {
                cpflags |= futils::GIT_CPDIR_CHMOD_DIRS;
            }
            error = futils::cp_r(t, repo_dir, cpflags, dmode as libc::mode_t);
        }
        if let Err(e) = error {
            if !default_template && e != ErrorCode::NOT_FOUND {
                return Err(e);
            }
            errors::clear();
            external_tpl = false;
            error = Ok(());
        }
    }

    for (path, mode, content) in REPO_TEMPLATE {
        error?;
        match content {
            None => {
                let mut mkdir_flags = futils::GIT_MKDIR_PATH;
                if chmod {
                    mkdir_flags |= futils::GIT_MKDIR_CHMOD;
                }
                error = futils::mkdir_relative(
                    path,
                    Some(repo_dir),
                    dmode as libc::mode_t,
                    mkdir_flags,
                    &mut MkdirOpts::default(),
                );
            }
            Some(content) if !external_tpl => {
                let content = match &opts.description {
                    Some(d) if path == GIT_DESC_FILE => d.to_bytes(),
                    _ => content,
                };
                error = repo_write_template(repo_dir, false, path, mode, content);
            }
            Some(_) => {}
        }
    }
    error
}

/// `mkdir_parent`.
fn mkdir_parent(buf: &GitStr, mode: u32, skip2: bool) -> Result<()> {
    futils::mkdir(
        str_c_bytes(buf),
        (mode & !(libc::S_ISGID | 0o002)) as libc::mode_t,
        futils::GIT_MKDIR_PATH
            | futils::GIT_MKDIR_VERIFY_DIR
            | if skip2 {
                futils::GIT_MKDIR_SKIP_LAST2
            } else {
                futils::GIT_MKDIR_SKIP_LAST
            },
    )
}

/// `repo_init_directories`.
fn repo_init_directories(
    repo_path: &mut GitStr,
    wd_path: &mut GitStr,
    given_repo: &[u8],
    opts: &mut InitOptions,
) -> Result<()> {
    let is_bare = opts.flags & GIT_REPOSITORY_INIT_BARE != 0;
    let add_dotgit = !is_bare
        && opts.workdir_path.is_none()
        && u::suffixcmp(given_repo, b"/.git") != 0
        && u::suffixcmp(given_repo, b"/.git/") != 0;

    repo_path
        .joinpath(given_repo, if add_dotgit { GIT_DIR } else { b"" })
        .map_err(|_| ErrorCode::GENERIC)?;

    let has_dotgit = u::suffixcmp(str_c_bytes(repo_path), b"/.git/") == 0;
    if has_dotgit {
        opts.flags |= GIT_REPOSITORY_INIT__HAS_DOTGIT;
    }

    if !is_bare {
        if let Some(wp) = &opts.workdir_path {
            let base = str_c_bytes(repo_path).to_vec();
            fs_path::join_unrooted(wd_path, wp.to_bytes(), Some(&base))
                .map_err(|_| ErrorCode::GENERIC)?;
        } else if has_dotgit {
            let rp = str_c_bytes(repo_path).to_vec();
            fs_path::dirname_r(Some(wd_path), Some(&rp)).map_err(|_| ErrorCode::GENERIC)?;
        } else {
            return Err(git_error_set!(
                ErrorClass::REPOSITORY,
                "cannot pick working directory for non-bare repository that isn't a '.git' directory"
            ));
        }
        fs_path::to_dir(wd_path).map_err(|_| ErrorCode::GENERIC)?;
    } else {
        wd_path.clear();
    }

    let natural_wd = has_dotgit
        && !wd_path.is_empty()
        && wd_path.len() + GIT_DIR.len() == repo_path.len()
        && repo_path.as_bytes()[..wd_path.len()] == *wd_path.as_bytes();
    if natural_wd {
        opts.flags |= GIT_REPOSITORY_INIT__NATURAL_WD;
    }

    let dirmode = pick_dir_mode(opts);

    if opts.flags & GIT_REPOSITORY_INIT_MKPATH != 0 {
        if !wd_path.is_empty() {
            mkdir_parent(wd_path, dirmode, false)?;
        }
        if !natural_wd {
            mkdir_parent(repo_path, dirmode, has_dotgit)?;
        }
    }

    if opts.flags & (GIT_REPOSITORY_INIT_MKDIR | GIT_REPOSITORY_INIT_MKPATH) != 0 {
        if !wd_path.is_empty() {
            futils::mkdir(
                str_c_bytes(wd_path),
                (dirmode & !libc::S_ISGID) as libc::mode_t,
                futils::GIT_MKDIR_VERIFY_DIR,
            )?;
        }
        if !natural_wd {
            futils::mkdir(
                str_c_bytes(repo_path),
                (dirmode & !libc::S_ISGID) as libc::mode_t,
                futils::GIT_MKDIR_VERIFY_DIR | futils::GIT_MKDIR_SKIP_LAST,
            )?;
        }
    }

    let mut error = Ok(());
    if opts.flags & (GIT_REPOSITORY_INIT_MKDIR | GIT_REPOSITORY_INIT_MKPATH) != 0 || has_dotgit {
        error = futils::mkdir(
            str_c_bytes(repo_path),
            dirmode as libc::mode_t,
            futils::GIT_MKDIR_VERIFY_DIR
                | if dirmode & libc::S_ISGID != 0 {
                    futils::GIT_MKDIR_CHMOD
                } else {
                    0
                },
        );
    }

    if error.is_ok() {
        let rp = cs(repo_path).into_owned();
        error = fs_path::prettify_dir(repo_path, &rp, None);
        if error.is_ok() && !wd_path.is_empty() {
            let wp = cs(wd_path).into_owned();
            error = fs_path::prettify_dir(wd_path, &wp, None);
        }
    }
    error
}

/// `get_initial_head`.
fn get_initial_head(flags: &mut u32, opts: &InitOptions) -> Result<CString> {
    let mut cfg_branch = GitStr::new();
    let initial: Vec<u8> = if let Some(h) = &opts.initial_head {
        *flags |= GIT_REFDB_BACKEND_INIT_FORCE_HEAD;
        h.to_bytes().to_vec()
    } else {
        let from_cfg = Config::open_default().ok().and_then(|cfg| {
            cfg.borrow()
                .get_string_buf(&mut cfg_branch, c"init.defaultbranch")
                .ok()
                .filter(|()| !str_c_bytes(&cfg_branch).is_empty())
                .map(|()| str_c_bytes(&cfg_branch).to_vec())
        });
        from_cfg.unwrap_or_else(|| GIT_BRANCH_DEFAULT.to_vec())
    };
    let initial = if u::prefixcmp(&initial, GIT_REFS_DIR) != 0 {
        let mut v = GIT_REFS_HEADS_DIR.to_vec();
        v.extend_from_slice(&initial);
        v
    } else {
        initial
    };
    Ok(fs_path::to_cstring(&initial))
}

/// `git_repository_init_ext`.
///
/// The outcome is C's: the status, and what `*out` became. `*out` is left
/// alone (`Unset`) when the steps before opening the repository fail; it is
/// NULL when opening fails; and it is the repository when a later step
/// (the refdb's init, the origin remote) fails, which the caller frees.
pub(crate) fn init_ext(given_repo: &CStr, opts: &mut InitOptions) -> (Result<()>, InitOut) {
    match init_prepare(given_repo, opts) {
        Err(e) => (Err(e), InitOut::Unset),
        Ok((rp, initial_head, refdb_init_flags)) => {
            init_open(&rp, initial_head, refdb_init_flags, opts)
        }
    }
}

/// What `git_repository_init_ext` leaves in `*out`.
pub(crate) enum InitOut {
    /// Not written.
    Unset,
    /// NULL.
    Null,
    /// The repository (the caller's).
    Repo(NonNull<GitRepository>),
}

fn init_prepare(
    given_repo: &CStr,
    opts: &mut InitOptions,
) -> Result<(Vec<u8>, Option<CString>, u32)> {
    let oid_type = if opts.oid_type != 0 {
        opts.oid_type
    } else {
        GIT_OID_DEFAULT
    };
    let refdb_type = if opts.refdb_type != 0 {
        opts.refdb_type
    } else {
        GIT_REFDB_FILES
    };

    let mut repo_path = GitStr::new();
    let mut wd_path = GitStr::new();
    let mut common_path = GitStr::new();
    let mut initial_head: Option<CString> = None;
    let mut refdb_init_flags = 0u32;

    repo_init_directories(&mut repo_path, &mut wd_path, given_repo.to_bytes(), opts)?;

    let rp = str_c_bytes(&repo_path).to_vec();
    let wd: Option<Vec<u8>> = if opts.flags & GIT_REPOSITORY_INIT_BARE != 0 {
        None
    } else {
        Some(str_c_bytes(&wd_path).to_vec())
    };

    if open::is_valid_repository_path(&mut repo_path, &mut common_path, opts.flags)? {
        if opts.flags & GIT_REPOSITORY_INIT_NO_REINIT != 0 {
            let _ = git_error_set!(
                ErrorClass::REPOSITORY,
                "attempt to reinitialize '{}'",
                Bytes(given_repo.to_bytes())
            );
            return Err(ErrorCode::EXISTS);
        }
        opts.flags |= GIT_REPOSITORY_INIT__IS_REINIT;
        repo_init_config(
            &rp,
            wd.as_deref(),
            opts.flags,
            opts.mode,
            oid_type,
            refdb_type,
        )?;
        if opts.initial_head.is_some() {
            initial_head = Some(get_initial_head(&mut refdb_init_flags, opts)?);
        }
    } else {
        let dmode = pick_dir_mode(opts);
        repo_init_structure(&rp, wd.as_deref(), opts)?;
        repo_init_config(
            &rp,
            wd.as_deref(),
            opts.flags,
            opts.mode,
            oid_type,
            refdb_type,
        )?;

        let mut path = GitStr::new();
        path.joinpath(&rp, GIT_HEAD_FILE)?;
        let p = cs(&path).into_owned();
        if !fs_path::exists(&p) {
            futils::writebuffer(
                GIT_INVALID_HEAD_CONTENT,
                &p,
                0,
                GIT_REFS_FILE_MODE as libc::mode_t,
            )?;
            refdb_init_flags |= GIT_REFDB_BACKEND_INIT_FORCE_HEAD;
        }
        path.joinpath(&rp, GIT_REFS_DIR)?;
        futils::mkdir(str_c_bytes(&path), dmode as libc::mode_t, 0)?;
        initial_head = Some(get_initial_head(&mut refdb_init_flags, opts)?);
    }

    Ok((rp, initial_head, refdb_init_flags))
}

fn init_open(
    rp: &[u8],
    initial_head: Option<CString>,
    refdb_init_flags: u32,
    opts: &InitOptions,
) -> (Result<()>, InitOut) {
    let rpc = fs_path::to_cstring(rp);
    let repo = match open::open_ext(Some(&rpc), GIT_REPOSITORY_OPEN_NO_SEARCH, None, true) {
        Ok(Some(r)) => r,
        Ok(None) => return (Err(ErrorCode::GENERIC), InitOut::Null),
        Err(e) => return (Err(e), InitOut::Null),
    };
    // SAFETY: the repository we just opened, handed to the caller below.
    let r = unsafe { repo.as_ref() };
    let result = (|| {
        let refdb = r.refdb_weak()?;
        // SAFETY: the repository's live refdb; a new reference for the call.
        let refdb = unsafe { Refdb::new_ref(refdb) };
        refdb.init(initial_head.as_deref(), opts.mode, refdb_init_flags)?;
        if let Some(url) = &opts.origin_url {
            crate::sys::remote::create(r, c"origin", url)?;
        }
        Ok(())
    })();
    (result, InitOut::Repo(repo))
}

/// `git_repository_set_workdir`.
///
/// # Safety
///
/// No borrow of the repository's workdir (a `&CStr` from
/// [`GitRepository::workdir`], or C's `git_repository_workdir`) is used
/// after the call, and nothing uses the repository concurrently.
pub(crate) unsafe fn set_workdir(
    repo: &GitRepository,
    workdir: &CStr,
    update_gitlink: bool,
) -> Result<()> {
    let mut path = GitStr::new();
    fs_path::prettify_dir(&mut path, workdir, None).map_err(|_| ErrorCode::GENERIC)?;

    if repo.workdir_field().map(CStr::to_bytes) == Some(str_c_bytes(&path)) {
        return Ok(());
    }

    if update_gitlink {
        let config = repo.config_weak().map_err(|_| ErrorCode::GENERIC)?;
        let gitdir = repo
            .gitdir()
            .map(|g| g.to_bytes().to_vec())
            .unwrap_or_default();
        let r = match repo_write_gitlink(str_c_bytes(&path), &gitdir, false) {
            Err(ErrorCode::PASSTHROUGH) => config.delete_entry(c"core.worktree"),
            Ok(()) => config.set_string(c"core.worktree", &cs(&path)),
            Err(e) => Err(e),
        };
        r?;
        config.set_bool(c"core.bare", false)?;
    }

    let new = detach(&mut path)?;
    // SAFETY: the caller's contract: no borrow of the old workdir is used
    // after this call.
    unsafe { repo.replace_string(StrField::Workdir, Some(new)) };
    repo.set_is_bare(false);
    Ok(())
}

/// `git_repository_set_bare`.
///
/// # Safety
///
/// As [`set_workdir`].
pub(crate) unsafe fn set_bare(repo: &GitRepository) -> Result<()> {
    if repo.is_bare() {
        return Ok(());
    }
    let config = repo.config_weak()?;
    config.set_bool(c"core.bare", true)?;
    config.update_entry(c"core.worktree", None, true, true)?;
    // SAFETY: the caller's contract.
    unsafe { repo.replace_string(StrField::Workdir, None) };
    repo.set_is_bare(true);
    Ok(())
}
