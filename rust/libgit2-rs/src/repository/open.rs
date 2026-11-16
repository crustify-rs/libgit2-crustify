//! Opening and discovering repositories (`repository.c`'s
//! `find_repo`, `git_repository_open_ext`, ownership validation, the
//! repository format and its extensions, and the configuration a
//! repository loads).

use core::ffi::{CStr, c_int, c_void};
use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::sync::Mutex;

use super::handles::{Config, ConfigEntryView, ConfigRef, Odb};
use super::*;
use crate::git_error_set;
use crate::sys::pending::l_repo as c;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::fs_path::{self, GIT_PATH_LIST_SEPARATOR, GIT_PATH_MAX, c_str, str_c_bytes};
use crate::util::futils;
use crate::util::posix;
use crate::util::str::GitStr;
use crate::util::util as u;

const GIT_COMMONDIR_FILE: &[u8] = b"commondir";
const GIT_GITDIR_FILE: &CStr = c"gitdir";
const GIT_FILE_CONTENT_PREFIX: &[u8] = b"gitdir:";
const GIT_HEAD_FILE: &[u8] = b"HEAD";
const GIT_OBJECTS_DIR: &[u8] = b"objects/";
const GIT_REFS_DIR: &[u8] = b"refs/";

/// `GIT_REPO_VERSION_MAX`.
pub(crate) const GIT_REPO_VERSION_MAX: i32 = 1;

/// The paths `find_repo` finds (`struct repo_paths`).
#[derive(Default)]
pub(crate) struct RepoPaths {
    pub gitdir: GitStr,
    pub workdir: GitStr,
    pub gitlink: GitStr,
    pub commondir: GitStr,
}

fn getenv(out: &mut GitStr, name: &CStr) -> Result<()> {
    u::getenv(out, name)
}

/// `lookup_commondir`: whether the repository at `repository_path` has a
/// separate common directory (a worktree's), and its path in `commondir`.
fn lookup_commondir(
    separate: &mut bool,
    commondir: &mut GitStr,
    repository_path: &mut GitStr,
    flags: u32,
) -> Result<()> {
    if flags & GIT_REPOSITORY_OPEN_FROM_ENV != 0 {
        match getenv(commondir, c"GIT_COMMON_DIR") {
            Err(ErrorCode::NOT_FOUND) => {}
            r => return r,
        }
    }

    if !fs_path::contains_file(repository_path, GIT_COMMONDIR_FILE) {
        let r = commondir
            .set(repository_path.as_bytes())
            .and_then(|()| fs_path::to_dir(commondir));
        *separate = false;
        return r;
    }

    *separate = true;
    let mut common_link = GitStr::new();
    common_link.joinpath(str_c_bytes(repository_path), GIT_COMMONDIR_FILE)?;
    let p = CString::new(str_c_bytes(&common_link)).unwrap_or_default();
    futils::readbuffer(&mut common_link, &p)?;
    common_link.rtrim();
    if fs_path::is_relative(str_c_bytes(&common_link)) {
        commondir.joinpath(str_c_bytes(repository_path), str_c_bytes(&common_link))?;
    } else {
        commondir.swap(&mut common_link);
    }
    // Make sure the commondir path always has a trailing slash.
    let src = fs_path::to_cstring(commondir.as_bytes());
    fs_path::prettify_dir(commondir, &src, None)
}

/// `is_valid_repository_path`: whether `repository_path` holds a
/// repository (`HEAD`, and `objects/` and `refs/` in its common directory,
/// which is left in `common_path`).
pub(crate) fn is_valid_repository_path(
    repository_path: &mut GitStr,
    common_path: &mut GitStr,
    flags: u32,
) -> Result<bool> {
    let mut separate = false;
    lookup_commondir(&mut separate, common_path, repository_path, flags)?;

    if !fs_path::contains_file(repository_path, GIT_HEAD_FILE) {
        return Ok(false);
    }
    if !fs_path::contains_dir(common_path, GIT_OBJECTS_DIR) {
        return Ok(false);
    }
    if !fs_path::contains_dir(common_path, GIT_REFS_DIR) {
        return Ok(false);
    }
    // `validate_repo_path`: `git_fs_path_validate_str_length_with_suffix`
    // accepts every length on Unix.
    Ok(true)
}

/// `load_config_data`: `core.bare`.
fn load_config_data(repo: &GitRepository, config: ConfigRef<'_>) -> Result<()> {
    match config.get_bool(c"core.bare") {
        Ok(b) => repo.set_is_bare(b && !repo.is_worktree()),
        Err(ErrorCode::NOT_FOUND) => repo.set_is_bare(false),
        Err(e) => return Err(e),
    }
    Ok(())
}

/// `load_workdir`.
fn load_workdir(
    repo: &GitRepository,
    config: ConfigRef<'_>,
    parent_path: &mut GitStr,
) -> Result<()> {
    if repo.is_bare() {
        return Ok(());
    }

    let mut workdir_env = GitStr::new();
    let mut value: Option<Vec<u8>> = None;
    if repo.use_env() {
        match getenv(&mut workdir_env, c"GIT_WORK_TREE") {
            Ok(()) => value = Some(str_c_bytes(&workdir_env).to_vec()),
            Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
    }

    let entry;
    if value.is_none() {
        entry = config.lookup_entry(c"core.worktree", false)?;
        if let Some(v) = entry.as_ref().and_then(|e| e.value()) {
            value = Some(v.to_bytes().to_vec());
        }
    }

    let gitdir = repo.gitdir().map(CStr::to_bytes).unwrap_or_default();
    let mut worktree = GitStr::new();
    let workdir = if repo.is_worktree() {
        let gitdir_c = repo.gitdir().unwrap_or(c"");
        let Some(gitlink) = crate::sys::worktree::read_link(gitdir_c, GIT_GITDIR_FILE) else {
            return Err(ErrorCode::GENERIC);
        };
        if fs_path::dirname_r(Some(&mut worktree), Some(gitlink.as_bytes())).is_err()
            || fs_path::to_dir(&mut worktree).is_err()
        {
            return Err(ErrorCode::GENERIC);
        }
        detach(&mut worktree)?
    } else if let Some(value) = value {
        if value.is_empty() {
            return Err(git_error_set!(
                ErrorClass::NET,
                "working directory cannot be set to empty path"
            ));
        }
        let v = fs_path::to_cstring(&value);
        fs_path::prettify_dir(&mut worktree, &v, Some(gitdir))?;
        detach(&mut worktree)?
    } else if !parent_path.is_empty() && fs_path::isdir(&cs(parent_path)) {
        detach(parent_path)?
    } else {
        if fs_path::dirname_r(Some(&mut worktree), Some(gitdir)).is_err()
            || fs_path::to_dir(&mut worktree).is_err()
        {
            return Err(ErrorCode::GENERIC);
        }
        detach(&mut worktree)?
    };
    repo.init_string(StrField::Workdir, workdir);
    Ok(())
}

/// `find_ceiling_dir_offset`: the furthest offset into `path` where a
/// ceiling directory is found (at least the root's length).
pub(crate) fn find_ceiling_dir_offset(path: &[u8], ceiling_directories: Option<&[u8]>) -> usize {
    let min_len = (fs_path::root(path) + 1) as usize;
    let Some(ceil_dirs) = ceiling_directories else {
        return min_len;
    };
    if min_len == 0 {
        return min_len;
    }
    let ceil_dirs = c_str(ceil_dirs);
    let mut max_len = 0usize;
    let mut start = 0usize;
    loop {
        let sep = ceil_dirs[start..]
            .iter()
            .position(|&b| b == GIT_PATH_LIST_SEPARATOR)
            .map_or(ceil_dirs.len(), |i| start + i);
        let len = sep - start;
        // C tests the root of `ceil`, which runs on past the separator.
        if !(len == 0 || len > GIT_PATH_MAX || fs_path::root(&ceil_dirs[start..]) == -1) {
            let buf = fs_path::to_cstring(&ceil_dirs[start..sep]);
            if let Ok(real) = posix::realpath(&buf) {
                let mut len = real.len();
                if len > 0 && real[len - 1] == b'/' {
                    len -= 1;
                }
                let at = |i: usize| path.get(i).copied().unwrap_or(0);
                if u::strncmp(path, &real, len) == 0
                    && (at(len) == b'/' || at(len) == 0)
                    && len > max_len
                {
                    max_len = len;
                }
            }
        }
        if sep >= ceil_dirs.len() {
            break;
        }
        start = sep + 1;
        if start >= ceil_dirs.len() {
            break;
        }
    }
    if max_len <= min_len { min_len } else { max_len }
}

/// `read_gitfile`: set `path_out` to the repository a `.git` file at
/// `file_path` points to.
fn read_gitfile(path_out: &mut GitStr, file_path: &[u8]) -> Result<()> {
    let mut file = GitStr::new();
    let fp = fs_path::to_cstring(file_path);
    futils::readbuffer(&mut file, &fp).map_err(|_| ErrorCode::GENERIC)?;
    file.rtrim();
    let bytes = str_c_bytes(&file);
    if file.len() <= GIT_FILE_CONTENT_PREFIX.len() || !bytes.starts_with(GIT_FILE_CONTENT_PREFIX) {
        return Err(git_error_set!(
            ErrorClass::REPOSITORY,
            "the `.git` file at '{}' is malformed",
            Bytes(file_path)
        ));
    }
    fs_path::dirname_r(Some(path_out), Some(file_path))?;
    let mut gitlink = &bytes[GIT_FILE_CONTENT_PREFIX.len()..];
    while let [c, rest @ ..] = gitlink {
        if !crate::util::ctype_compat::isspace(*c) {
            break;
        }
        gitlink = rest;
    }
    let gl = fs_path::to_cstring(gitlink);
    let base = str_c_bytes(path_out).to_vec();
    fs_path::prettify_dir(path_out, &gl, Some(&base))
}

/// `validate_ownership_cb`'s match of one `safe.directory` value.
pub(crate) fn safe_directory_matches(
    value: Option<&[u8]>,
    repo_path: &[u8],
    is_safe: &mut bool,
) -> Result<()> {
    let value = match value {
        None | Some(b"") => {
            *is_safe = false;
            return Ok(());
        }
        Some(v) => v,
    };
    if value == b"*" {
        *is_safe = true;
        return Ok(());
    }
    let mut tmp = GitStr::new();
    tmp.sets(value)?;
    let mut is_prefix = false;
    if u::suffixcmp(str_c_bytes(&tmp), b"/*") == 0 {
        is_prefix = true;
        tmp.shorten(1);
    }
    if !fs_path::is_root(str_c_bytes(&tmp)) {
        let b = tmp.as_bytes();
        if b.is_empty() || (!is_prefix && b[b.len() - 1] == b'/') {
            return Ok(());
        }
        if !is_prefix {
            fs_path::to_dir(&mut tmp)?;
        }
    }
    let mut test_path = str_c_bytes(&tmp);
    if test_path.starts_with(b"%(prefix)//") {
        test_path = &test_path[b"%(prefix)/".len()..];
    }
    let matched = if is_prefix {
        u::prefixcmp(repo_path, test_path) == 0
    } else {
        test_path == repo_path
    };
    if matched {
        *is_safe = true;
    }
    Ok(())
}

/// `validate_ownership_config`.
fn validate_ownership_config(is_safe: &mut bool, path: &[u8], use_env: bool) -> Result<()> {
    let Ok(config) = load_global_config(use_env) else {
        return Ok(());
    };
    let r = config
        .borrow()
        .multivar_foreach(c"safe.directory", &mut |e: &ConfigEntryView<'_>| {
            safe_directory_matches(e.value().map(CStr::to_bytes), path, is_safe)
        });
    match r {
        Err(ErrorCode::NOT_FOUND) => Ok(()),
        r => r,
    }
}

/// `validate_ownership_path`.
fn validate_ownership_path(is_safe: &mut bool, path: Option<&CStr>) -> Result<()> {
    const OWNER_LEVEL: core::ffi::c_uint = fs_path::GIT_FS_PATH_OWNER_CURRENT_USER
        | fs_path::GIT_FS_PATH_USER_IS_ADMINISTRATOR
        | fs_path::GIT_FS_PATH_OWNER_RUNNING_SUDO;
    let Some(path) = path else {
        return Ok(());
    };
    match fs_path::owner_is(path, OWNER_LEVEL) {
        Ok(v) => {
            *is_safe = v;
            Ok(())
        }
        Err(ErrorCode::NOT_FOUND) => {
            *is_safe = true;
            Ok(())
        }
        Err(ErrorCode::INVALID) => {
            *is_safe = false;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// `validate_ownership`.
fn validate_ownership(repo: &GitRepository) -> Result<()> {
    let mut paths: [Option<&CStr>; 3] = [None; 3];
    let mut n = 0;
    if let Some(w) = repo.workdir_field() {
        paths[n] = Some(w);
        n += 1;
    }
    if let Some(g) = repo.gitlink() {
        paths[n] = Some(g);
        n += 1;
    }
    paths[n] = repo.gitdir();
    n += 1;

    let mut is_safe = false;
    let mut path: Option<&CStr> = None;
    for p in &paths[..n] {
        path = *p;
        validate_ownership_path(&mut is_safe, path)?;
        if !is_safe {
            break;
        }
    }

    if is_safe {
        return Ok(());
    }
    let first = paths[0].map(CStr::to_bytes).unwrap_or_default();
    validate_ownership_config(&mut is_safe, first, repo.use_env())?;

    if !is_safe {
        let p = path.map(CStr::to_bytes).unwrap_or_default();
        let path_len = if fs_path::is_root(p) {
            p.len()
        } else {
            fs_path::dirlen(p)
        };
        let _ = git_error_set!(
            ErrorClass::CONFIG,
            "repository path '{}' is not owned by current user",
            Bytes(&p[..path_len.min(i32::MAX as usize)])
        );
        return Err(ErrorCode::OWNER);
    }
    Ok(())
}

/// `find_repo_traverse`.
fn find_repo_traverse(
    out: &mut RepoPaths,
    start_path: &CStr,
    ceiling_dirs: Option<&[u8]>,
    flags: u32,
) -> Result<()> {
    let r = find_repo_traverse_inner(out, start_path, ceiling_dirs, flags);
    if r.is_err() {
        *out = RepoPaths::default();
    }
    r
}

fn find_repo_traverse_inner(
    out: &mut RepoPaths,
    start_path: &CStr,
    ceiling_dirs: Option<&[u8]>,
    flags: u32,
) -> Result<()> {
    let mut path = GitStr::new();
    let mut repo_link = GitStr::new();
    let mut common_link = GitStr::new();
    let mut initial_device: libc::dev_t = 0;
    let mut ceiling_offset = 0usize;

    out.gitdir.clear();
    fs_path::prettify(&mut path, start_path, None)?;

    let (mut in_dot_git, mut min_iterations) =
        if flags & (GIT_REPOSITORY_OPEN_BARE | GIT_REPOSITORY_OPEN_NO_DOTGIT) != 0 {
            (true, 1)
        } else {
            (false, 2)
        };

    loop {
        if flags & GIT_REPOSITORY_OPEN_NO_DOTGIT == 0 {
            if !in_dot_git {
                path.joinpath_self(DOT_GIT)?;
            }
            in_dot_git = !in_dot_git;
        }

        if let Ok(st) = posix::stat(&cs(&path)) {
            if initial_device == 0 {
                initial_device = st.st_dev;
            } else if st.st_dev != initial_device && flags & GIT_REPOSITORY_OPEN_CROSS_FS == 0 {
                break;
            }

            let fmt = st.st_mode & libc::S_IFMT;
            if fmt == libc::S_IFDIR {
                if is_valid_repository_path(&mut path, &mut common_link, flags)? {
                    fs_path::to_dir(&mut path)?;
                    out.gitdir.set(path.as_bytes())?;
                    out.gitlink = GitStr::new();
                    if let Some(link) = crate::sys::worktree::read_link(&cs(&path), GIT_GITDIR_FILE)
                    {
                        out.gitlink.sets(link.as_bytes())?;
                    }
                    common_link.swap(&mut out.commondir);
                    break;
                }
            } else if fmt == libc::S_IFREG && u::suffixcmp(str_c_bytes(&path), b"/.git") == 0 {
                read_gitfile(&mut repo_link, str_c_bytes(&path))?;
                if is_valid_repository_path(&mut repo_link, &mut common_link, flags)? {
                    out.gitdir.swap(&mut repo_link);
                    out.gitlink.put(path.as_bytes())?;
                    common_link.swap(&mut out.commondir);
                }
                break;
            }
        }

        fs_path::dirname_self(&mut path)?;

        if min_iterations > 0 {
            min_iterations -= 1;
            if min_iterations == 0 {
                ceiling_offset = find_ceiling_dir_offset(str_c_bytes(&path), ceiling_dirs);
            }
        }

        if min_iterations == 0
            && (path.as_bytes().get(ceiling_offset).copied().unwrap_or(0) == 0
                || flags & GIT_REPOSITORY_OPEN_NO_SEARCH != 0)
        {
            break;
        }
    }

    if flags & GIT_REPOSITORY_OPEN_BARE == 0 {
        if out.gitdir.is_empty() {
            out.workdir.clear();
        } else {
            fs_path::dirname_r(Some(&mut out.workdir), Some(str_c_bytes(&path)))?;
            fs_path::to_dir(&mut out.workdir)?;
        }
    }

    if out.gitdir.is_empty() {
        let _ = git_error_set!(
            ErrorClass::REPOSITORY,
            "could not find repository at '{}'",
            Bytes(start_path.to_bytes())
        );
        return Err(ErrorCode::NOT_FOUND);
    }
    Ok(())
}

/// `git_config_parse_bool` (PENDING(l-config)), with its side effect on
/// the error state.
fn config_parse_bool(value: &CStr) -> Result<bool> {
    let mut v: c_int = 0;
    // SAFETY: a NUL-terminated value and a local out-parameter.
    let r = unsafe { c::git_config_parse_bool(&raw mut v, value.as_ptr()) };
    if r < 0 {
        return Err(ErrorCode::check(r).err().unwrap_or(ErrorCode::GENERIC));
    }
    Ok(v != 0)
}

/// `find_repo`: `find_repo_traverse`, after the environment of
/// `GIT_REPOSITORY_OPEN_FROM_ENV`.
pub(crate) fn find_repo(
    out: &mut RepoPaths,
    start_path: Option<&CStr>,
    ceiling_dirs: Option<&CStr>,
    mut flags: u32,
) -> Result<()> {
    let use_env = flags & GIT_REPOSITORY_OPEN_FROM_ENV != 0;
    let mut gitdir_buf = GitStr::new();
    let mut ceiling_dirs_buf = GitStr::new();
    let mut across_fs_buf = GitStr::new();

    let gitdir_c;
    let mut start: Option<&CStr> = start_path;
    if use_env && start.is_none() {
        match getenv(&mut gitdir_buf, c"GIT_DIR") {
            Ok(()) => {
                gitdir_c = cs(&gitdir_buf).into_owned();
                start = Some(&gitdir_c);
                flags |= GIT_REPOSITORY_OPEN_NO_SEARCH | GIT_REPOSITORY_OPEN_NO_DOTGIT;
            }
            Err(ErrorCode::NOT_FOUND) => start = Some(c"."),
            Err(e) => return Err(e),
        }
    }

    let mut ceiling: Option<Vec<u8>> = ceiling_dirs.map(|c| c.to_bytes().to_vec());
    if use_env && ceiling.is_none() {
        match getenv(&mut ceiling_dirs_buf, c"GIT_CEILING_DIRECTORIES") {
            Ok(()) => ceiling = Some(str_c_bytes(&ceiling_dirs_buf).to_vec()),
            Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
    }

    if use_env {
        match getenv(&mut across_fs_buf, c"GIT_DISCOVERY_ACROSS_FILESYSTEM") {
            Ok(()) => {
                if config_parse_bool(&cs(&across_fs_buf))? {
                    flags |= GIT_REPOSITORY_OPEN_CROSS_FS;
                }
            }
            Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
    }

    // `git_fs_path_prettify`'s `GIT_ASSERT_ARG(path)`.
    let Some(start) = start else {
        return Err(errors::assert_arg_failed("path"));
    };
    find_repo_traverse(out, start, ceiling.as_deref(), flags)
}

/// `check_repositoryformatversion`.
pub(crate) fn check_repositoryformatversion(
    version: &mut i32,
    config: ConfigRef<'_>,
) -> Result<()> {
    match config.get_int32(c"core.repositoryformatversion") {
        Err(ErrorCode::NOT_FOUND) => return Ok(()),
        Err(_) => return Err(ErrorCode::GENERIC),
        Ok(v) => *version = v,
    }
    if *version < 0 {
        let _ = git_error_set!(
            ErrorClass::REPOSITORY,
            "invalid repository version {}",
            *version
        );
    }
    if GIT_REPO_VERSION_MAX < *version {
        return Err(git_error_set!(
            ErrorClass::REPOSITORY,
            "unsupported repository version {}; only versions up to {} are supported",
            *version,
            GIT_REPO_VERSION_MAX
        ));
    }
    Ok(())
}

/// `builtin_extensions`.
pub(crate) const BUILTIN_EXTENSIONS: [&[u8]; 6] = [
    b"noop",
    b"objectformat",
    b"worktreeconfig",
    b"preciousobjects",
    b"refstorage",
    b"relativeworktrees",
];

/// `user_extensions`: sorted (`git__strcmp_cb`), without duplicates.
static USER_EXTENSIONS: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

fn user_extensions() -> std::sync::MutexGuard<'static, Vec<Vec<u8>>> {
    USER_EXTENSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `check_valid_extension` of the entry named `name`.
pub(crate) fn check_valid_extension(name: &[u8]) -> Result<()> {
    let matches = |ext: &[u8]| {
        name.len() == 11 + ext.len() && name.starts_with(b"extensions.") && &name[11..] == ext
    };
    let fail = || {
        Err(git_error_set!(
            ErrorClass::REPOSITORY,
            "unsupported extension name {}",
            Bytes(name)
        ))
    };
    for ext in user_extensions().iter() {
        let (reject, ext) = match ext.split_first() {
            Some((b'!', rest)) => (true, rest),
            _ => (false, &ext[..]),
        };
        if matches(ext) {
            return if reject { fail() } else { Ok(()) };
        }
    }
    if BUILTIN_EXTENSIONS.iter().any(|b| matches(b)) {
        return Ok(());
    }
    fail()
}

/// `check_extensions`.
pub(crate) fn check_extensions(config: Option<ConfigRef<'_>>, version: i32) -> Result<()> {
    if version < 1 {
        return Ok(());
    }
    // C passes a NULL config to `git_config_foreach_match` here when the
    // repository has no config file (and version cannot then be 1).
    let Some(config) = config else {
        return Ok(());
    };
    config.foreach_match(c"^extensions\\.", &mut |e: &ConfigEntryView<'_>| {
        check_valid_extension(e.name().to_bytes())
    })
}

/// `load_objectformat`.
fn load_objectformat(repo: &GitRepository, config: ConfigRef<'_>) -> Result<()> {
    match config.get_entry(c"extensions.objectformat") {
        Err(ErrorCode::NOT_FOUND) => {
            repo.set_oid_type_raw(GIT_OID_DEFAULT);
            errors::clear();
            Ok(())
        }
        Err(e) => Err(e),
        Ok(entry) => {
            let v = entry.value().map(CStr::to_bytes).unwrap_or_default();
            match OidType::from_name(v) {
                Some(t) => {
                    repo.set_oid_type_raw(t.raw());
                    Ok(())
                }
                None => {
                    repo.set_oid_type_raw(0);
                    let _ = git_error_set!(
                        ErrorClass::REPOSITORY,
                        "unknown object format '{}'",
                        Bytes(v)
                    );
                    Err(ErrorCode::INVALID)
                }
            }
        }
    }
}

/// `git_refdb_type_fromstr` (`refdb.h`'s inline).
pub(crate) fn refdb_type_fromstr(name: &[u8]) -> c_int {
    match name {
        b"files" => GIT_REFDB_FILES,
        b"reftable" => GIT_REFDB_REFTABLE,
        _ => 0,
    }
}

/// `git_refdb_type_name` (`refdb.h`'s inline).
pub(crate) fn refdb_type_name(t: c_int) -> &'static CStr {
    match t {
        GIT_REFDB_FILES => c"files",
        GIT_REFDB_REFTABLE => c"reftable",
        _ => c"unknown",
    }
}

/// `load_refstorage_format`.
fn load_refstorage_format(repo: &GitRepository, config: ConfigRef<'_>) -> Result<()> {
    match config.get_entry(c"extensions.refstorage") {
        Err(ErrorCode::NOT_FOUND) => {
            repo.set_refdb_type(GIT_REFDB_FILES);
            errors::clear();
            Ok(())
        }
        Err(e) => Err(e),
        Ok(entry) => {
            let v = entry.value().map(CStr::to_bytes).unwrap_or_default();
            let t = refdb_type_fromstr(v);
            repo.set_refdb_type(t);
            if t == 0 {
                let _ = git_error_set!(
                    ErrorClass::REPOSITORY,
                    "unknown refstorage format '{}'",
                    Bytes(v)
                );
                return Err(ErrorCode::INVALID);
            }
            Ok(())
        }
    }
}

/// `read_repository_format`.
fn read_repository_format(repo: &GitRepository) -> Result<()> {
    let mut config_path = GitStr::new();
    repo.item_path(&mut config_path, ITEM_CONFIG)?;
    let config = match Config::open_ondisk(&cs(&config_path)) {
        Ok(c) => Some(c),
        Err(ErrorCode::NOT_FOUND) => None,
        Err(e) => return Err(e),
    };
    let mut version = 0;
    if let Some(c) = &config {
        check_repositoryformatversion(&mut version, c.borrow())?;
    }
    check_extensions(config.as_ref().map(Config::borrow), version)?;
    match (&config, version > 0) {
        (Some(c), true) => {
            load_objectformat(repo, c.borrow())?;
            load_refstorage_format(repo, c.borrow())?;
        }
        _ => {
            repo.set_oid_type_raw(GIT_OID_DEFAULT);
            repo.set_refdb_type(GIT_REFDB_FILES);
        }
    }
    Ok(())
}

/// `load_grafts`.
pub(crate) fn load_grafts(repo: &GitRepository) -> Result<()> {
    if !repo.grafts_ptr().is_null() && !repo.shallow_grafts_ptr().is_null() {
        // SAFETY: the repository's live grafts.
        check(unsafe { c::git_grafts_refresh(repo.grafts_ptr()) })?;
        // SAFETY: as above.
        check(unsafe { c::git_grafts_refresh(repo.shallow_grafts_ptr()) })?;
    }

    let mut path = GitStr::new();
    let oid_type = repo.oid_type_raw();
    if let Err(e) = repo.item_path(&mut path, ITEM_INFO) {
        if e != ErrorCode::NOT_FOUND {
            return Err(e);
        }
        if repo.grafts_ptr().is_null() {
            // SAFETY: the repository's grafts slot, which C fills.
            check(unsafe { c::git_grafts_new(repo.grafts_slot(), oid_type) })?;
        }
        if repo.shallow_grafts_ptr().is_null() {
            // SAFETY: as above.
            check(unsafe { c::git_grafts_new(repo.shallow_grafts_slot(), oid_type) })?;
        }
        return Ok(());
    }

    path.joinpath_self(b"grafts")?;
    // SAFETY: the grafts slot and a NUL-terminated path.
    check(unsafe {
        c::git_grafts_open_or_refresh(repo.grafts_slot(), cs(&path).as_ptr(), oid_type)
    })?;
    path.clear();
    path.joinpath(
        repo.gitdir().map(CStr::to_bytes).unwrap_or_default(),
        b"shallow",
    )?;
    // SAFETY: as above.
    check(unsafe {
        c::git_grafts_open_or_refresh(repo.shallow_grafts_slot(), cs(&path).as_ptr(), oid_type)
    })?;
    Ok(())
}

/// `git_repository_open_bare`.
pub(crate) fn open_bare(bare_path: &CStr) -> Result<NonNull<GitRepository>> {
    let mut path = GitStr::new();
    let mut common_path = GitStr::new();
    fs_path::prettify_dir(&mut path, bare_path, None)?;
    if !is_valid_repository_path(&mut path, &mut common_path, 0)? {
        return Err({
            let _ = git_error_set!(
                ErrorClass::REPOSITORY,
                "path is not a repository: {}",
                Bytes(bare_path.to_bytes())
            );
            ErrorCode::NOT_FOUND
        });
    }
    let repo = NonNull::new(GitRepository::alloc()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the fresh repository, ours alone.
    let r = unsafe { repo.as_ref() };
    r.init_string(StrField::Gitdir, detach(&mut path)?);
    r.init_string(StrField::Commondir, detach(&mut common_path)?);
    r.set_is_bare(true);
    r.set_is_worktree(false);
    // C leaks the repository on this failure (DEFECTS.md); it is freed.
    if let Err(e) = read_repository_format(r) {
        // SAFETY: ours alone, not handed out.
        unsafe { GitRepository::free(repo.as_ptr()) };
        return Err(e);
    }
    Ok(repo)
}

/// `repo_load_namespace`.
fn repo_load_namespace(repo: &GitRepository) -> Result<()> {
    if !repo.use_env() {
        return Ok(());
    }
    let mut buf = GitStr::new();
    match getenv(&mut buf, c"GIT_NAMESPACE") {
        Ok(()) => repo.init_string(StrField::Namespace, detach(&mut buf)?),
        Err(ErrorCode::NOT_FOUND) => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// `repo_is_worktree`.
fn repo_is_worktree(repo: &GitRepository) -> Result<bool> {
    if let (Some(c), Some(g)) = (repo.commondir(), repo.gitdir())
        && c == g
    {
        return Ok(false);
    }
    let mut link = GitStr::new();
    link.joinpath(
        repo.gitdir().map(CStr::to_bytes).unwrap_or_default(),
        b"gitdir",
    )
    .map_err(|_| ErrorCode::GENERIC)?;
    Ok(fs_path::exists(&cs(&link)))
}

/// `git_repository_open_ext`: `None` when `want_repo` is false (C's NULL
/// `repo_ptr`: only find the repository).
pub(crate) fn open_ext(
    start_path: Option<&CStr>,
    flags: u32,
    ceiling_dirs: Option<&CStr>,
    want_repo: bool,
) -> Result<Option<NonNull<GitRepository>>> {
    let mut paths = RepoPaths::default();
    find_repo(&mut paths, start_path, ceiling_dirs, flags)?;
    if !want_repo {
        return Ok(None);
    }
    let repo = NonNull::new(GitRepository::alloc()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the fresh repository, ours alone until returned.
    let r = unsafe { repo.as_ref() };
    match open_ext_fill(r, &mut paths, flags) {
        Ok(()) => Ok(Some(repo)),
        Err(e) => {
            // SAFETY: ours alone, not handed out.
            unsafe { GitRepository::free(repo.as_ptr()) };
            Err(e)
        }
    }
}

fn open_ext_fill(repo: &GitRepository, paths: &mut RepoPaths, flags: u32) -> Result<()> {
    repo.set_use_env(flags & GIT_REPOSITORY_OPEN_FROM_ENV != 0);
    repo.init_string(StrField::Gitdir, detach(&mut paths.gitdir)?);
    if !paths.gitlink.is_empty() {
        repo.init_string(StrField::Gitlink, detach(&mut paths.gitlink)?);
    }
    if !paths.commondir.is_empty() {
        repo.init_string(StrField::Commondir, detach(&mut paths.commondir)?);
    }

    let wt = repo_is_worktree(repo)?;
    repo.set_is_worktree(wt);

    read_repository_format(repo)?;
    load_grafts(repo)?;

    if flags & GIT_REPOSITORY_OPEN_BARE != 0 {
        repo.set_is_bare(true);
    } else {
        let config = match repo.config_weak().and_then(ConfigRef::snapshot) {
            Ok(c) => Some(c),
            Err(ErrorCode::NOT_FOUND) => None,
            Err(e) => return Err(e),
        };
        if let Some(config) = &config {
            load_config_data(repo, config.borrow())?;
            load_workdir(repo, config.borrow(), &mut paths.workdir)?;
        }
    }

    repo_load_namespace(repo)?;

    if git_repository__validate_ownership.load(core::sync::atomic::Ordering::Relaxed) {
        validate_ownership(repo)?;
    }
    Ok(())
}

/// `git_repository_new_ext`.
pub(crate) fn new_ext(oid_type: Option<OidType>) -> Result<NonNull<GitRepository>> {
    let repo = NonNull::new(GitRepository::alloc()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the fresh repository.
    let r = unsafe { repo.as_ref() };
    r.set_is_bare(true);
    r.set_is_worktree(false);
    r.set_oid_type_raw(oid_type.unwrap_or(OidType::DEFAULT).raw());
    r.set_refdb_type(GIT_REFDB_FILES);
    Ok(repo)
}

/// `git_repository_wrap_odb`.
pub(crate) fn wrap_odb(odb: &Odb) -> Result<NonNull<GitRepository>> {
    let repo = NonNull::new(GitRepository::alloc()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the fresh repository.
    let r = unsafe { repo.as_ref() };
    let t = odb.oid_type_raw();
    if OidType::from_raw(t).is_none() {
        // C leaks the repository here; it is freed.
        // SAFETY: ours alone.
        unsafe { GitRepository::free(repo.as_ptr()) };
        return Err(errors::assert_failed(
            "git_oid_type_is_valid(odb->options.oid_type)",
        ));
    }
    r.set_oid_type_raw(t);
    r.set_refdb_type(GIT_REFDB_FILES);
    // SAFETY: a live odb the caller holds; the fresh repository had none.
    unsafe { r.set_odb(Some(odb.as_nonnull())) };
    Ok(repo)
}

/// `git_repository_discover`: the git directory found from `start_path`.
pub(crate) fn discover(
    start_path: &CStr,
    across_fs: bool,
    ceiling_dirs: Option<&CStr>,
) -> Result<GitStr> {
    let mut paths = RepoPaths::default();
    let flags = if across_fs {
        GIT_REPOSITORY_OPEN_CROSS_FS
    } else {
        0
    };
    find_repo(&mut paths, Some(start_path), ceiling_dirs, flags)?;
    Ok(core::mem::take(&mut paths.gitdir))
}

/// `has_config_worktree`.
fn has_config_worktree(cfg: ConfigRef<'_>) -> Result<bool> {
    match cfg.get_bool(c"extensions.worktreeconfig") {
        Ok(v) => Ok(v),
        Err(ErrorCode::NOT_FOUND) => Ok(false),
        Err(e) => Err(e),
    }
}

/// `load_config`: a configuration of the repository's files (if any) and
/// the global, XDG and system ones.
pub(crate) fn load_config<'r>(
    repo: Option<&'r GitRepository>,
    global: Option<&CStr>,
    xdg: Option<&CStr>,
    system: Option<&CStr>,
) -> Result<Config<'r>> {
    let cfg = Config::new()?;
    let c = cfg.borrow();
    let rp: *const c_void = repo.map_or(ptr::null(), |r| r.as_ptr().cast_const());

    if let Some(repo) = repo {
        let mut config_path = GitStr::new();
        let dirs = repo.item_dirs();
        let r = item_path(&mut config_path, &dirs, ITEM_CONFIG)
            .and_then(|()| c.add_file_ondisk(&cs(&config_path), GIT_CONFIG_LEVEL_LOCAL, rp, false));
        match r {
            Ok(()) | Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
        let r = has_config_worktree(c).and_then(|has| {
            if has {
                item_path(&mut config_path, &dirs, ITEM_WORKTREE_CONFIG)?;
                c.add_file_ondisk(&cs(&config_path), GIT_CONFIG_LEVEL_WORKTREE, rp, false)
            } else {
                Ok(())
            }
        });
        match r {
            Ok(()) | Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
    }

    for (path, level) in [
        (global, GIT_CONFIG_LEVEL_GLOBAL),
        (xdg, GIT_CONFIG_LEVEL_XDG),
        (system, GIT_CONFIG_LEVEL_SYSTEM),
    ] {
        if let Some(p) = path {
            match c.add_file_ondisk(p, level, rp, false) {
                Ok(()) | Err(ErrorCode::NOT_FOUND) => {}
                Err(e) => return Err(e),
            }
        }
    }

    errors::clear();
    c.set_writeorder(GIT_CONFIG_LEVEL_LOCAL)?;
    Ok(cfg)
}

/// `config_path_system`.
pub(crate) fn config_path_system(out: &mut GitStr, use_env: bool) -> Result<()> {
    if use_env {
        let mut no_system_buf = GitStr::new();
        match getenv(&mut no_system_buf, c"GIT_CONFIG_NOSYSTEM") {
            Ok(()) | Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
        // As C: an unset variable parses "" (setting its error message,
        // which is then ignored).
        let no_system = config_parse_bool(&cs(&no_system_buf)).unwrap_or(false);
        if no_system {
            return Ok(());
        }
        match getenv(out, c"GIT_CONFIG_SYSTEM") {
            Err(ErrorCode::NOT_FOUND) => {}
            _ => return Ok(()),
        }
    }
    // SAFETY: an exclusive `git_str`.
    unsafe { c::git_config__find_system(out) };
    Ok(())
}

/// `config_path_global`.
pub(crate) fn config_path_global(out: &mut GitStr, use_env: bool) -> Result<()> {
    if use_env {
        match getenv(out, c"GIT_CONFIG_GLOBAL") {
            Err(ErrorCode::NOT_FOUND) => {}
            _ => return Ok(()),
        }
    }
    // SAFETY: an exclusive `git_str`.
    unsafe { c::git_config__find_global(out) };
    Ok(())
}

/// `load_global_config`: the global and system configuration only.
pub(crate) fn load_global_config(use_env: bool) -> Result<Config<'static>> {
    let mut global = GitStr::new();
    let mut xdg = GitStr::new();
    let mut system = GitStr::new();
    config_path_system(&mut system, use_env)?;
    config_path_global(&mut global, use_env)?;
    // SAFETY: an exclusive `git_str`.
    unsafe { c::git_config__find_xdg(&raw mut xdg) };
    load_config(
        None,
        path_unless_empty(&global).as_deref(),
        path_unless_empty(&xdg).as_deref(),
        path_unless_empty(&system).as_deref(),
    )
}

/// `repository_odb_path`.
pub(crate) fn repository_odb_path(out: &mut GitStr, repo: &GitRepository) -> Result<()> {
    if repo.use_env() {
        match getenv(out, c"GIT_OBJECT_DIRECTORY") {
            Err(ErrorCode::NOT_FOUND) => {}
            r => return r,
        }
    }
    repo.item_path(out, ITEM_OBJECTS)
}

/// `repository_odb_alternates`.
pub(crate) fn repository_odb_alternates(odb: &Odb, repo: &GitRepository) -> Result<()> {
    if !repo.use_env() {
        return Ok(());
    }
    let mut alternates = GitStr::new();
    match getenv(&mut alternates, c"GIT_ALTERNATE_OBJECT_DIRECTORIES") {
        Ok(()) => {}
        Err(ErrorCode::NOT_FOUND) => return Ok(()),
        Err(e) => return Err(e),
    }
    // As C, a failing alternate is ignored.
    for alt in str_c_bytes(&alternates).split(|&b| b == GIT_PATH_LIST_SEPARATOR) {
        let a = fs_path::to_cstring(alt);
        let _ = odb.add_disk_alternate(&a);
    }
    Ok(())
}

/// `repository_index_path`.
pub(crate) fn repository_index_path(out: &mut GitStr, repo: &GitRepository) -> Result<()> {
    if repo.use_env() {
        match getenv(out, c"GIT_INDEX_FILE") {
            Err(ErrorCode::NOT_FOUND) => {}
            r => return r,
        }
    }
    repo.item_path(out, ITEM_INDEX)
}

/// `git_repository__extensions`: the supported extensions, sorted.
pub(crate) fn extensions() -> Result<Vec<Vec<u8>>> {
    let user = user_extensions();
    let mut out: Vec<Vec<u8>> = Vec::new();
    for b in BUILTIN_EXTENSIONS {
        if user
            .iter()
            .any(|u| u.first() == Some(&b'!') && &u[1..] == b)
        {
            continue;
        }
        out.push(b.to_vec());
    }
    for u in user.iter() {
        if u.first() == Some(&b'!') {
            continue;
        }
        out.push(u.clone());
    }
    // `git_vector_sort` with `git__strcmp_cb` (a stable sort, as tsort).
    out.sort();
    Ok(out)
}

/// `git_repository__set_extensions`.
pub(crate) fn set_extensions(exts: &[&[u8]]) {
    let mut user = user_extensions();
    user.clear();
    for e in exts {
        if BUILTIN_EXTENSIONS.contains(e) {
            continue;
        }
        if let Err(i) = user.binary_search_by(|x| x.as_slice().cmp(e)) {
            user.insert(i, e.to_vec());
        }
    }
}

/// `git_repository__free_extensions`.
pub(crate) fn free_extensions() {
    let mut user = user_extensions();
    user.clear();
    user.shrink_to_fit();
}

/// `git_repository_open_from_worktree`'s path: the worktree's gitlink
/// without its `.git` suffix.
pub(crate) fn worktree_repo_path(gitlink_path: &[u8]) -> Option<&[u8]> {
    let len = gitlink_path.len();
    if len <= 4 || !gitlink_path[len - 4..].eq_ignore_ascii_case(b".git") {
        return None;
    }
    Some(&gitlink_path[..len - 4])
}
