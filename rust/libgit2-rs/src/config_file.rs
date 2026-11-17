//! Port of `src/libgit2/config_file.c`: the backend of configuration files
//! on disk, with includes (`include.path`, `includeIf.<condition>.path`),
//! reloads when the file changes, and writes that keep the rest of the
//! file byte for byte.
//!
//! The backend is a `CTable` over [`FileBackend`] (opaque state behind the
//! mirrored `git_config_backend` table); `submodule.c` calls its slots
//! directly, `config.c` through [`Backend`].
//!
//! # Concurrency
//!
//! Every slot takes `&FileBackend`: a repository's config is read from
//! several threads at once. The entries are an `Arc` behind a mutex (C's
//! `values_mutex`); the tree of files read (stamps, checksums, includes)
//! and the lock state have their own mutexes. None is held while the
//! includes are read: a conditional include (`onbranch:`) looks `HEAD` up,
//! which may read the repository's config, and so this backend, again.

use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ffibox::vtable::CTable;

use crate::config::{
    Backend, BackendCalls, BackendPtr, ConfigBackendCell, GIT_CONFIG_FILE_MODE, as_cstr, cstring,
    git_config_backend, git_config_backend_entry, git_config_iterator, normalize_name, slot_busy,
    slot_panicked,
};
use crate::config_list::{ConfigList, EntryMeta};
use crate::config_mem::{arg, entry_name};
use crate::config_parse::{ESCAPED, ESCAPES, ParseHandler, parse};
use crate::repository::GitRepository as git_repository;
use crate::sys::sysdir;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{Filebuf, GIT_FILEBUF_HASH_SHA256, GitFilebuf};
use crate::util::fs_path;
use crate::util::futils::{self, Filestamp};
use crate::util::hash::{self, HashAlgorithm};
use crate::util::regexp::Regexp;
use crate::util::str::GitStr;
use crate::util::util::{prefixcmp, strcasecmp};
use crate::util::wildmatch::{WM_CASEFOLD, WM_MATCH, WM_PATHNAME, wildmatch_raw as wildmatch};
use crate::{git_assert_arg, git_error_set};

/// `MAX_INCLUDE_DEPTH`.
const MAX_INCLUDE_DEPTH: u32 = 10;

/// `CONFIG_FILE_TYPE`.
const CONFIG_FILE_TYPE: &[u8] = b"file";

/// `config_file`: a file read, and the files it included.
#[derive(Clone, Debug, Default)]
pub struct ConfigFile {
    stamp: Filestamp,
    checksum: [u8; 32],
    /// The path, with a NUL (made once: every refresh `stat`s it).
    cpath: Vec<u8>,
    includes: Vec<ConfigFile>,
}

impl ConfigFile {
    fn new(path: &[u8]) -> Result<ConfigFile> {
        Ok(ConfigFile {
            cpath: cstring(path)?,
            ..ConfigFile::default()
        })
    }

    /// The path.
    fn path(&self) -> &[u8] {
        self.cpath.split_last().map_or(&[][..], |(_, p)| p)
    }

    /// This file without its includes (`config_file_clear_includes`).
    fn without_includes(&self) -> ConfigFile {
        ConfigFile {
            stamp: self.stamp,
            checksum: self.checksum,
            cpath: self.cpath.clone(),
            includes: Vec::new(),
        }
    }
}

/// The lock of `config_file_lock`: the lock file and the contents that
/// writes edit until `unlock`.
#[derive(Debug)]
struct Locked {
    buf: Filebuf,
    content: Vec<u8>,
}

/// `config_file_backend`.
#[derive(Debug)]
pub struct FileBackend {
    /// The entries (`values_mutex`, `config_list`); `None` before `open`
    /// and after a failed one.
    values: Mutex<Option<Arc<ConfigList>>>,
    /// The repository `open` was given (NULL for none), kept without a
    /// reference as C does, and reached only through
    /// [`live::with_live`](crate::repository::live::with_live) with
    /// `repo_serial`: it may have been freed since (C reads it anyway).
    repo: AtomicPtr<git_repository>,
    /// The repository's serial in the live registry (0 for none), written
    /// before `repo`.
    repo_serial: AtomicU64,
    level: AtomicI32,
    /// `file`: the tree of files read.
    file: Mutex<ConfigFile>,
    /// `locked`, `locked_buf`, `locked_content`.
    locked: Mutex<Option<Locked>>,
}

// Every field is `Send + Sync` by itself (no manual impl): checked here, so
// that a field added later that is not makes this fail to compile.
const _: fn() = || {
    fn thread_safe<T: Send + Sync>() {}
    thread_safe::<FileBackend>();
};

/// The table a slot runs for, as a handle: the methods read its
/// `readonly` (which C may set) when C did.
///
/// # Safety
///
/// `cfg` is the live table of the slot being run; the handle is used only
/// during that slot.
unsafe fn table(cfg: *mut git_config_backend) -> BackendPtr {
    let p = NonNull::new(cfg).unwrap_or_else(|| unreachable!("a slot's table is not NULL"));
    // SAFETY: the live table of the running slot (this function's
    // contract), for as long as the handle is used.
    unsafe { BackendPtr::new(p) }
}

/// `escape_value`: `value` with its special characters escaped.
fn escape_value(value: &[u8]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve(value.len() + 1)
        .map_err(|_| errors::oom())?;
    for &c in value {
        match ESCAPED.iter().position(|&e| e == c) {
            Some(k) => {
                buf.push(b'\\');
                buf.push(ESCAPES[k]);
            }
            None => buf.push(c),
        }
    }
    Ok(buf)
}

/// What a C string holds: up to its first NUL.
fn until_nul(s: &[u8]) -> &[u8] {
    match memchr::memchr(0, s) {
        Some(n) => &s[..n],
        None => s,
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// `config_file_parse_data` and `read_on_variable`.
struct Reader<'a> {
    repo: Option<&'a git_repository>,
    file: &'a mut ConfigFile,
    list: &'a mut ConfigList,
    level: c_int,
    depth: u32,
    /// The interned backend type and path of `file` (C interns both for
    /// each entry; the strings are the same).
    meta: Option<EntryMeta>,
}

impl Reader<'_> {
    fn meta(&mut self) -> Result<EntryMeta> {
        if let Some(m) = self.meta {
            return Ok(m);
        }
        let m = EntryMeta {
            backend_type: self.list.intern(CONFIG_FILE_TYPE)?,
            origin_path: self.list.intern(self.file.path())?,
            include_depth: self.depth,
            level: self.level,
        };
        self.meta = Some(m);
        Ok(m)
    }

    /// `parse_include`.
    fn parse_include(&mut self, value: Option<&[u8]>) -> Result<()> {
        let Some(file) = value else { return Ok(()) };
        let mut dir = GitStr::new();
        fs_path::dirname_r(Some(&mut dir), Some(self.file.path()))?;
        let mut path = GitStr::new();
        included_path(&mut path, dir.as_bytes(), file)?;
        drop(dir);
        self.file
            .includes
            .try_reserve(1)
            .map_err(|_| errors::oom())?;
        let include = ConfigFile::new(path.as_bytes())?;
        self.file.includes.push(include);
        let include = self
            .file
            .includes
            .last_mut()
            .unwrap_or_else(|| unreachable!());
        match read(self.list, self.repo, include, self.level, self.depth + 1) {
            Err(e) if e == ErrorCode::NOT_FOUND => {
                errors::clear();
                Ok(())
            }
            r => r,
        }
    }

    /// `parse_conditional_include`.
    fn parse_conditional_include(&mut self, section: &[u8], value: Option<&[u8]>) -> Result<()> {
        let Some(repo) = self.repo else {
            return Ok(());
        };
        if value.is_none() {
            return Ok(());
        }
        const PREFIX: usize = b"includeIf.".len();
        const SUFFIX: usize = b".path".len();
        if section.len() < PREFIX + SUFFIX {
            return Ok(());
        }
        let condition = &section[PREFIX..section.len() - SUFFIX];
        type Matcher = fn(&git_repository, &[u8], &[u8]) -> Result<bool>;
        let conditions: [(&[u8], Matcher); 3] = [
            (b"gitdir:", |r, f, c| match_gitdir(r, f, c, false)),
            (b"gitdir/i:", |r, f, c| match_gitdir(r, f, c, true)),
            (b"onbranch:", match_onbranch),
        ];
        for (prefix, matches) in conditions {
            if prefixcmp(condition, prefix) != 0 {
                continue;
            }
            if matches(repo, self.file.path(), &condition[prefix.len()..])? {
                self.parse_include(value)?;
            }
            break;
        }
        Ok(())
    }
}

impl ParseHandler for Reader<'_> {
    fn on_variable(
        &mut self,
        section: Option<&[u8]>,
        name: &[u8],
        value: Option<&[u8]>,
        _line: &[u8],
    ) -> Result<()> {
        let n = entry_name(section, name)?;
        let meta = self.meta()?;
        self.list.push(&n, value, meta)?;
        if n == b"include.path" {
            self.parse_include(value)
        } else if prefixcmp(&n, b"includeif.") == 0 && n.ends_with(b".path") {
            self.parse_conditional_include(&n, value)
        } else {
            Ok(())
        }
    }
}

/// `included_path`.
fn included_path(out: &mut GitStr, dir: &[u8], path: &[u8]) -> Result<()> {
    if path.starts_with(b"~/") {
        let rest = cstring(&path[1..])?;
        return sysdir::expand_homedir_file(out, Some(as_cstr(&rest)));
    }
    fs_path::join_unrooted(out, path, Some(dir)).map(drop)
}

/// Byte `i` of a C string, 0 past its end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `do_match_gitdir`.
fn match_gitdir(
    repo: &git_repository,
    cfg_file: &[u8],
    condition: &[u8],
    icase: bool,
) -> Result<bool> {
    let mut pattern = GitStr::new();
    // As in C, the failures of these are only seen as an out-of-memory
    // pattern below.
    if at(condition, 0) == b'.' && fs_path::is_dirsep(at(condition, 1)) {
        let _ = fs_path::dirname_r(Some(&mut pattern), Some(cfg_file));
        let _ = pattern.joinpath_self(&condition[2..]);
    } else if at(condition, 0) == b'~' && fs_path::is_dirsep(at(condition, 1)) {
        let rest = cstring(&condition[1..])?;
        let _ = sysdir::expand_homedir_file(&mut pattern, Some(as_cstr(&rest)));
    } else if !fs_path::is_absolute(condition) {
        let _ = pattern.joinpath(b"**", condition);
    } else {
        let _ = pattern.sets(condition);
    }
    if condition.last().is_some_and(|&c| fs_path::is_dirsep(c)) {
        let _ = pattern.puts(b"**");
    }
    if pattern.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    let mut gitdir = GitStr::new();
    repo.item_path(&mut gitdir, crate::repository::ITEM_GITDIR)?;
    if gitdir
        .as_bytes()
        .last()
        .is_some_and(|&c| fs_path::is_dirsep(c))
    {
        let n = gitdir.len() - 1;
        gitdir.truncate(n);
    }
    let flags = WM_PATHNAME | if icase { WM_CASEFOLD } else { 0 };
    Ok(wildmatch(until_nul(pattern.as_bytes()), gitdir.as_bytes(), flags) == WM_MATCH)
}

/// `conditional_match_onbranch`: `HEAD` is a branch matching `condition`.
///
/// `HEAD` is read from the refdb directly: the higher-level functions read
/// the repository's config, which is being read.
fn match_onbranch(repo: &git_repository, _cfg_file: &[u8], condition: &[u8]) -> Result<bool> {
    let target = repository_head_symbolic_target(repo)?;
    let Some(target) = target else {
        return Ok(false);
    };
    // C leaves the result unset (uninitialized) for a `HEAD` outside
    // `refs/heads/`: no match here (see DEFECTS.md).
    let Some(branch) = target.strip_prefix(b"refs/heads/") else {
        return Ok(false);
    };
    let mut buf = Vec::new();
    buf.try_reserve(condition.len() + 2)
        .map_err(|_| errors::oom())?;
    buf.extend_from_slice(condition);
    if condition.last().is_some_and(|&c| fs_path::is_dirsep(c)) {
        buf.extend_from_slice(b"**");
    }
    Ok(wildmatch(&buf, until_nul(branch), WM_PATHNAME) == WM_MATCH)
}

/// `config_file_read_buffer`.
fn read_buffer(
    list: &mut ConfigList,
    repo: Option<&git_repository>,
    file: &mut ConfigFile,
    level: c_int,
    depth: u32,
    buf: &[u8],
) -> Result<()> {
    if depth >= MAX_INCLUDE_DEPTH {
        return Err(git_error_set!(
            ErrorClass::CONFIG,
            "maximum config include depth reached"
        ));
    }
    // An empty file (or one starting with a NUL): nothing to do.
    if buf.first().copied().unwrap_or(0) == 0 {
        return Ok(());
    }
    let path = file.path().to_vec();
    let mut reader = Reader {
        repo,
        file,
        list,
        level,
        depth,
        meta: None,
    };
    parse(Some(&path), buf, &mut reader)
}

/// `config_file_read`: read `file` (setting its stamp and checksum) and
/// its includes into `list`.
fn read(
    list: &mut ConfigList,
    repo: Option<&git_repository>,
    file: &mut ConfigFile,
    level: c_int,
    depth: u32,
) -> Result<()> {
    let cpath = as_cstr(&file.cpath);
    let st = match crate::util::posix::stat(cpath) {
        Ok(st) => st,
        Err(e) => return Err(fs_path::set_error(e.0, file.path(), "stat")),
    };
    let mut contents = GitStr::new();
    futils::readbuffer(&mut contents, cpath)?;
    file.stamp = Filestamp::from_stat(&st);
    let digest = hash::hash_buf(contents.as_bytes(), HashAlgorithm::Sha256)?;
    file.checksum.copy_from_slice(&digest.as_bytes()[..32]);
    read_buffer(list, repo, file, level, depth, contents.as_bytes())
}

/// `config_file_is_modified`: whether `file` or one of its includes
/// changed (refreshing the stamps on the way).
fn is_modified(file: &mut ConfigFile) -> Result<bool> {
    let cpath = as_cstr(&file.cpath);
    if futils::filestamp_check(Some(&mut file.stamp), cpath) != 0 {
        let mut buf = GitStr::new();
        futils::readbuffer(&mut buf, cpath)?;
        let digest = hash::hash_buf(buf.as_bytes(), HashAlgorithm::Sha256)?;
        if digest.as_bytes()[..32] != file.checksum {
            return Ok(true);
        }
    }
    for include in &mut file.includes {
        if is_modified(include)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// `quotes_for_value`.
fn quotes_for_value(value: &[u8]) -> &'static [u8] {
    match value.first() {
        None | Some(b' ') => return b"\"",
        _ => {}
    }
    if value.iter().any(|&c| c == b';' || c == b'#') {
        return b"\"";
    }
    if value.last() == Some(&b' ') {
        return b"\"";
    }
    b""
}

/// `write_line_to`: `line`, ended with a newline.
fn write_line_to(buf: &mut Vec<u8>, line: &[u8]) -> Result<()> {
    buf.try_reserve(line.len() + 1).map_err(|_| errors::oom())?;
    buf.extend_from_slice(line);
    if line.last().is_some_and(|&c| c != b'\n') {
        buf.push(b'\n');
    }
    Ok(())
}

/// `struct write_data` and the write callbacks.
struct Writer<'a> {
    buf: Vec<u8>,
    buffered_comment: Vec<u8>,
    in_section: bool,
    preg_replaced: bool,
    orig_section: &'a [u8],
    section: &'a [u8],
    orig_name: &'a [u8],
    name: &'a [u8],
    preg: Option<&'a Regexp>,
    value: Option<&'a [u8]>,
}

impl Writer<'_> {
    /// `write_value`.
    fn write_value(&mut self) -> Result<()> {
        let value = self.value.unwrap_or(b"");
        let q = quotes_for_value(value);
        let b = &mut self.buf;
        b.try_reserve(self.orig_name.len() + value.len() + 8)
            .map_err(|_| errors::oom())?;
        b.push(b'\t');
        b.extend_from_slice(self.orig_name);
        b.extend_from_slice(b" = ");
        b.extend_from_slice(q);
        b.extend_from_slice(value);
        b.extend_from_slice(q);
        b.push(b'\n');
        // A single name/value is written once, even if its section repeats.
        if self.preg.is_none() {
            self.value = None;
        }
        Ok(())
    }

    /// Flush the comments buffered before a section or variable.
    fn flush_comments(&mut self) -> Result<()> {
        self.buf
            .try_reserve(self.buffered_comment.len())
            .map_err(|_| errors::oom())?;
        self.buf.extend_from_slice(&self.buffered_comment);
        self.buffered_comment.clear();
        Ok(())
    }
}

/// `write_section`: `[section "subsection"]`.
fn write_section(buf: &mut Vec<u8>, key: &[u8]) -> Result<()> {
    let mut s = Vec::new();
    s.push(b'[');
    match memchr::memchr(b'.', key) {
        None => s.extend_from_slice(key),
        Some(dot) => {
            s.extend_from_slice(&key[..dot]);
            let escaped = escape_value(&key[dot + 1..])?;
            s.extend_from_slice(b" \"");
            s.extend_from_slice(&escaped);
            s.push(b'"');
        }
    }
    s.extend_from_slice(b"]\n");
    buf.try_reserve(s.len()).map_err(|_| errors::oom())?;
    buf.extend_from_slice(&s);
    Ok(())
}

impl ParseHandler for Writer<'_> {
    fn on_section(&mut self, section: &[u8], line: &[u8]) -> Result<()> {
        // Leaving the section without having written a single value: it
        // goes at the end of the section.
        let mut result = Ok(());
        if self.in_section && self.preg.is_none() && self.value.is_some() {
            result = self.write_value();
        }
        self.in_section = section == self.section;
        result?;
        self.flush_comments()?;
        write_line_to(&mut self.buf, line)
    }

    fn on_variable(
        &mut self,
        _section: Option<&[u8]>,
        name: &[u8],
        value: Option<&[u8]>,
        line: &[u8],
    ) -> Result<()> {
        self.flush_comments()?;
        let mut has_matched = self.in_section && strcasecmp(self.name, name) == 0;
        if has_matched && let Some(preg) = self.preg {
            // C would crash on a bare name (NULL value); it matches as "".
            has_matched = preg.matches(value.unwrap_or(b"")).is_ok();
        }
        if !has_matched {
            return write_line_to(&mut self.buf, line);
        }
        self.preg_replaced = true;
        // No value: the variable is deleted.
        if self.value.is_none() {
            return Ok(());
        }
        self.write_value()
    }

    fn on_comment(&mut self, line: &[u8]) -> Result<()> {
        write_line_to(&mut self.buffered_comment, line)
    }

    fn on_eof(&mut self, section: Option<&[u8]>) -> Result<()> {
        self.flush_comments()?;
        // The section was never seen: write it, and the value.
        if (self.preg.is_none() || !self.preg_replaced) && self.value.is_some() {
            if section != Some(self.section) {
                write_section(&mut self.buf, self.orig_section)?;
            }
            self.write_value()?;
        }
        Ok(())
    }
}

/// The section and name of a key, around its last dot.
fn split_key(key: &[u8]) -> (&[u8], &[u8]) {
    match memchr::memrchr(b'.', key) {
        Some(d) => (&key[..d], &key[d + 1..]),
        None => (&key[..0], key),
    }
}

/// The C status of a `Filebuf` result (its message is already set).
fn fb<T>(r: core::result::Result<T, errors::Error>) -> Result<T> {
    r.map_err(|e| e.raw_code())
}

impl FileBackend {
    fn cpath(&self) -> Vec<u8> {
        self.lock_file().cpath.clone()
    }

    fn path(&self) -> Vec<u8> {
        self.lock_file().path().to_vec()
    }

    fn lock_file(&self) -> std::sync::MutexGuard<'_, ConfigFile> {
        self.file.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_values(&self) -> std::sync::MutexGuard<'_, Option<Arc<ConfigList>>> {
        self.values.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_locked(&self) -> std::sync::MutexGuard<'_, Option<Locked>> {
        self.locked.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` with the repository `open` was given, if it is still live
    /// (`None` when there was none, or it has been freed: the conditions
    /// that need it then do not match).
    fn with_repo<R>(&self, f: impl FnOnce(Option<&git_repository>) -> R) -> R {
        let p = self.repo.load(Ordering::Acquire).cast_const();
        let serial = self.repo_serial.load(Ordering::Acquire);
        crate::repository::live::with_live(
            p,
            crate::repository::live::Token::from_parts(p, serial),
            f,
        )
    }

    fn level(&self) -> c_int {
        self.level.load(Ordering::Acquire)
    }

    /// `config_file_take_list` (an empty list before `open`).
    fn take_list(&self) -> Arc<ConfigList> {
        self.lock_values()
            .clone()
            .unwrap_or_else(|| ConfigList::new().freeze())
    }

    /// `config_file_set_entries`.
    fn set_entries(&self, cfg: BackendPtr, list: ConfigList) -> Result<()> {
        if cfg.is_readonly() {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "this backend is read-only"
            ));
        }
        let old = self.lock_values().replace(list.freeze());
        drop(old);
        Ok(())
    }

    /// `config_file_open`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository for the call. It is kept without
    /// a reference, as C does, and reached later only while it is still
    /// registered live ([`with_repo`](Self::with_repo)).
    unsafe fn open(&self, level: c_int, repo: *const git_repository) -> Result<()> {
        self.level.store(level, Ordering::Release);
        let tok = crate::repository::live::token(repo);
        self.repo_serial.store(tok.serial(), Ordering::Release);
        self.repo.store(repo.cast_mut(), Ordering::Release);
        *self.lock_values() = Some(ConfigList::new().freeze());
        let cpath = self.cpath();
        let cpath = as_cstr(&cpath);
        if !fs_path::exists(cpath) {
            return Ok(());
        }
        // git ignores configuration files it cannot read.
        if crate::util::posix::access(cpath, libc::R_OK).is_err() {
            return Err(ErrorCode::NOT_FOUND);
        }
        let mut list = ConfigList::new();
        let mut file = self.lock_file().without_includes();
        let r = self.with_repo(|repo| read(&mut list, repo, &mut file, level, 0));
        *self.lock_file() = file;
        match r {
            Ok(()) => {
                *self.lock_values() = Some(list.freeze());
                Ok(())
            }
            Err(e) => {
                *self.lock_values() = None;
                Err(e)
            }
        }
    }

    /// `config_file_refresh`.
    fn refresh(&self, cfg: BackendPtr) -> Result<()> {
        if cfg.is_readonly() {
            return Ok(());
        }
        let file = {
            let mut f = self.lock_file();
            match is_modified(&mut f) {
                // The common case allocates nothing.
                Ok(false) => return Ok(()),
                Err(e) if e == ErrorCode::NOT_FOUND => return Ok(()),
                Err(e) => return Err(e),
                Ok(true) => f.without_includes(),
            }
        };
        let mut file = file;
        let mut list = ConfigList::new();
        let level = self.level();
        let r = self.with_repo(|repo| read(&mut list, repo, &mut file, level, 0));
        *self.lock_file() = file;
        let r = r.and_then(|()| self.set_entries(cfg, list));
        match r {
            Err(e) if e == ErrorCode::NOT_FOUND => Ok(()),
            r => r,
        }
    }

    /// `config_file_refresh_from_buffer`.
    fn refresh_from_buffer(&self, cfg: BackendPtr, buf: &[u8]) -> Result<()> {
        let mut file = self.lock_file().without_includes();
        let mut list = ConfigList::new();
        let level = self.level();
        let r = self.with_repo(|repo| read_buffer(&mut list, repo, &mut file, level, 0, buf));
        *self.lock_file() = file;
        r?;
        self.set_entries(cfg, list)
    }

    /// `config_file_get`.
    fn get(&self, cfg: BackendPtr, key: &[u8]) -> Result<*mut git_config_backend_entry> {
        if !cfg.is_readonly() {
            self.refresh(cfg)?;
        }
        let values = self.lock_values();
        let Some(list) = values.as_ref() else {
            return Err(ErrorCode::NOT_FOUND);
        };
        match list.get(key) {
            None => Err(ErrorCode::NOT_FOUND),
            Some(e) => Ok(ConfigList::lend(list, e)),
        }
    }

    /// `config_file_iterator`.
    fn iterator(&self, cfg: BackendPtr) -> Result<*mut git_config_iterator> {
        self.refresh(cfg)?;
        Ok(crate::config_list::iterator_new(self.take_list()))
    }

    /// `config_file_set`.
    fn set(&self, cfg: BackendPtr, name: &[u8], value: Option<&[u8]>) -> Result<()> {
        let key = normalize_name(name)?;
        let list = self.take_list();
        match list.get_unique(&key) {
            Err(e) if e == ErrorCode::NOT_FOUND => {}
            Err(e) => return Err(e),
            // Nothing to write if the value is the same.
            Ok(existing) if existing.value() == value => return Ok(()),
            Ok(_) => {}
        }
        let esc = value.map(escape_value).transpose()?;
        self.write(cfg, name, &key, None, esc.as_deref())
    }

    /// `config_file_set_multivar`.
    fn set_multivar(
        &self,
        cfg: BackendPtr,
        name: &[u8],
        regexp: Option<&CStr>,
        value: Option<&[u8]>,
    ) -> Result<()> {
        git_assert_arg!(regexp.is_some(), "regexp");
        let key = normalize_name(name)?;
        let preg = Regexp::compile_opt(regexp, 0)?;
        self.write(cfg, name, &key, Some(&preg), value)
    }

    /// `config_file_delete`.
    fn delete(&self, cfg: BackendPtr, name: &[u8]) -> Result<()> {
        let key = normalize_name(name)?;
        let list = self.take_list();
        let entry = match list.get_unique(&key) {
            Ok(e) => e,
            Err(e) => {
                if e == ErrorCode::NOT_FOUND {
                    let _ = git_error_set!(
                        ErrorClass::CONFIG,
                        "could not find key '{}' to delete",
                        Bytes(name)
                    );
                }
                return Err(e);
            }
        };
        let entry_name = entry.name().to_vec();
        self.write(cfg, name, &entry_name, None, None)
    }

    /// `config_file_delete_multivar`.
    fn delete_multivar(&self, cfg: BackendPtr, name: &[u8], regexp: Option<&CStr>) -> Result<()> {
        let key = normalize_name(name)?;
        let list = self.take_list();
        if list.get(&key).is_none() {
            let _ = git_error_set!(
                ErrorClass::CONFIG,
                "could not find key '{}' to delete",
                Bytes(name)
            );
            return Err(ErrorCode::NOT_FOUND);
        }
        let preg = Regexp::compile_opt(regexp, 0)?;
        self.write(cfg, name, &key, Some(&preg), None)
    }

    /// `config_file_lock`.
    fn lock(&self) -> Result<()> {
        let path = self.path();
        let buf = fb(Filebuf::open(&path, 0, GIT_CONFIG_FILE_MODE))?;
        let cpath = cstring(&path)?;
        let mut content = GitStr::new();
        match futils::readbuffer(&mut content, as_cstr(&cpath)) {
            Ok(()) => {}
            Err(e) if e == ErrorCode::NOT_FOUND => {}
            // Dropping the filebuf removes the lock file.
            Err(e) => return Err(e),
        }
        *self.lock_locked() = Some(Locked {
            buf,
            content: content.as_bytes().to_vec(),
        });
        Ok(())
    }

    /// `config_file_unlock`.
    fn unlock(&self, success: bool) -> Result<()> {
        let Some(Locked { mut buf, content }) = self.lock_locked().take() else {
            // Not locked: C commits its zeroed filebuf, which fails.
            return if success {
                GitFilebuf::new().commit()
            } else {
                Ok(())
            };
        };
        if !success {
            // Dropping the filebuf removes the lock file.
            return Ok(());
        }
        let _ = buf.write(&content);
        fb(buf.commit())
    }

    /// `config_file_write`: rewrite the file with `key` (`orig_key` as the
    /// user wrote it) set to `value` (deleted if `None`), for the values
    /// matching `preg` if given.
    fn write(
        &self,
        cfg: BackendPtr,
        orig_key: &[u8],
        key: &[u8],
        preg: Option<&Regexp>,
        value: Option<&[u8]>,
    ) -> Result<()> {
        let path = self.path();
        let (section, name) = split_key(key);
        let (orig_section, orig_name) = split_key(orig_key);
        let writer = |contents: &[u8]| -> Result<Vec<u8>> {
            let mut w = Writer {
                buf: Vec::new(),
                buffered_comment: Vec::new(),
                in_section: false,
                preg_replaced: false,
                orig_section,
                section,
                orig_name,
                name,
                preg,
                value,
            };
            parse(Some(&path), contents, &mut w)?;
            Ok(w.buf)
        };

        // Locked: edit the locked copy.
        {
            let mut locked = self.lock_locked();
            if let Some(l) = locked.as_mut() {
                l.content = writer(until_nul(&l.content))?;
                return Ok(());
            }
        }

        // Dropped on the way out, it removes the lock file.
        let mut file = fb(Filebuf::open(
            &path,
            GIT_FILEBUF_HASH_SHA256,
            GIT_CONFIG_FILE_MODE,
        ))?;
        let cpath = cstring(&path)?;
        let mut contents = GitStr::new();
        match futils::readbuffer(&mut contents, as_cstr(&cpath)) {
            Ok(()) => {}
            Err(e) if e == ErrorCode::NOT_FOUND => {}
            Err(e) => return Err(e),
        }
        let buf = writer(contents.as_bytes())?;
        drop(contents);
        let _ = file.write(&buf);
        fb(file.commit())?;
        self.refresh_from_buffer(cfg, &buf)
    }
}

/// A slot's result as C's status.
fn status(r: Result<()>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => e.raw(),
    }
}

/// A slot's NULL-able C string argument.
///
/// # Safety
///
/// `p` is NULL or NUL-terminated, and unwritten for `'a` (the slot's
/// call).
unsafe fn opt<'a>(p: *const c_char) -> Option<&'a CStr> {
    // SAFETY: `p` is NULL or NUL-terminated and unwritten for `'a` (this
    // function's contract), which is `CStrArg::from_ptr`'s.
    unsafe { crate::util::cabi::CStrArg::from_ptr(p) }.get()
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot its backend in the marked (`@cfg`)
    // position, the only `git_config_backend *` argument; every slot runs
    // through `impl_cvtable!`'s trampolines (state via `enter`, `free` via
    // `release`) and keeps sys/config.h's contract (out-parameters written
    // only on success, entries and iterators handed over); all twelve slots
    // are set (`snapshot` in the base, to this crate's
    // `git_config_backend_snapshot`); `version` is
    // `GIT_CONFIG_BACKEND_VERSION` and `cfg` NULL until `add_backend` sets
    // it, as C accepts.
    unsafe impl CVtable<FileBackend> for ConfigBackendCell {}
    base = git_config_backend {
        snapshot: Some(crate::config_snapshot::ffi::git_config_backend_snapshot),
        ..git_config_backend::default()
    };
    /// `config_file_open`.
    fn open(@cfg, level: c_int, repo: *const git_repository) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `repo` is NULL or a live repository for the call (the
        // `open` slot's contract, sys/config.h).
        |b: &FileBackend| status(unsafe { b.open(level, repo) }),
    }
    /// `config_file_get`.
    fn get(@cfg, key: *const c_char, out: *mut *mut git_config_backend_entry) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &FileBackend| {
            // SAFETY: `cfg` is this slot's live table.
            let t = unsafe { table(cfg) };
            // SAFETY: `key` is a NUL-terminated string, unwritten for the
            // call (the slot's contract).
            match b.get(t, unsafe { arg(key) }) {
                Ok(e) => {
                    // SAFETY: `out` is aligned and writable for the call (the
                    // slot's contract), and nothing references `*out`.
                    unsafe { out.write(e) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `config_file_set`.
    fn set(@cfg, name: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &FileBackend| {
            // SAFETY: `cfg` is this slot's live table.
            let t = unsafe { table(cfg) };
            // SAFETY: the strings are NULL or NUL-terminated, unwritten for
            // the call (the slot's contract).
            status(b.set(t, unsafe { arg(name) }, unsafe { opt(value) }.map(CStr::to_bytes)))
        },
    }
    /// `config_file_set_multivar`.
    fn set_multivar(@cfg, name: *const c_char, regexp: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &FileBackend| {
            // SAFETY: `cfg` is this slot's live table.
            let t = unsafe { table(cfg) };
            // SAFETY: as for `set`.
            let name = unsafe { arg(name) };
            // SAFETY: as for `set`.
            let regexp = unsafe { opt(regexp) };
            // SAFETY: as for `set`.
            let value = unsafe { opt(value) }.map(CStr::to_bytes);
            status(b.set_multivar(t, name, regexp, value))
        },
    }
    /// `config_file_delete`.
    fn del(@cfg, name: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: as for `set`: this slot's table and string.
        |b: &FileBackend| status(b.delete(unsafe { table(cfg) }, unsafe { arg(name) })),
    }
    /// `config_file_delete_multivar`.
    fn del_multivar(@cfg, name: *const c_char, regexp: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: as for `set`: this slot's table and strings.
        |b: &FileBackend| status(b.delete_multivar(unsafe { table(cfg) }, unsafe { arg(name) }, unsafe { opt(regexp) })),
    }
    /// `config_file_iterator`.
    fn iterator(out: *mut *mut git_config_iterator, @cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `cfg` is this slot's live table.
        |b: &FileBackend| match b.iterator(unsafe { table(cfg) }) {
            Ok(it) => {
                // SAFETY: `out` is aligned and writable for the call (the
                // slot's contract), and nothing references `*out`.
                unsafe { out.write(it) };
                0
            }
            Err(e) => e.raw(),
        },
    }
    /// `config_file_lock`.
    fn lock(@cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &FileBackend| status(b.lock()),
    }
    /// `config_file_unlock`.
    fn unlock(@cfg, success: c_int) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &FileBackend| status(b.unlock(success != 0)),
    }
    release free;
}

/// `git_config_backend_from_file`.
pub fn backend_from_file(path: &[u8]) -> Result<Backend> {
    let path = until_nul(path);
    let file = ConfigFile::new(path)?;
    let state = FileBackend {
        values: Mutex::new(None),
        repo: AtomicPtr::new(core::ptr::null_mut()),
        repo_serial: AtomicU64::new(0),
        level: AtomicI32::new(0),
        file: Mutex::new(file),
        locked: Mutex::new(None),
    };
    let t = CTable::<ConfigBackendCell, FileBackend>::new(state).into_c();
    // SAFETY: `t` is the table `into_c` surrendered just now, owned by
    // nothing else, and released through its `free` slot
    // (`CTable::release`), which `Backend`'s policy calls.
    Ok(unsafe { Backend::from_c(t.as_ptr().cast()) }.unwrap_or_else(|| unreachable!()))
}

/// The exports of `config_file.c`.
pub mod ffi {
    use super::*;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::barrier;

    /// `git_config_backend_from_file`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `path` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_from_file(
        mut out: Out<'_, *mut git_config_backend>,
        path: CStrArg<'_>,
    ) -> c_int {
        barrier(|| match backend_from_file(path.to_bytes()) {
            Ok(b) => {
                out.write(b.into_c().cast());
                0
            }
            Err(e) => e.raw(),
        })
    }
}

/// The symbolic target of the repository's `HEAD`, read from its refdb
/// directly (`None` if `HEAD` is not symbolic): config_file.c's
/// `conditional_match_onbranch` (the higher-level functions read the
/// repository's config, which is being read).
fn repository_head_symbolic_target(
    repo: &crate::repository::GitRepository,
) -> crate::util::errors::Result<Option<Vec<u8>>> {
    use crate::refs::ffi as r;
    use crate::util::errors::ErrorCode;
    // C takes a reference (`git_repository_refdb`) and drops it at the
    // end; the repository keeps the refdb live meanwhile either way.
    let refdb = repo.refdb_weak()?;
    let mut head = core::ptr::null_mut();
    // SAFETY: `head` is a local out-parameter; `refdb` is the repository's
    // refdb, kept live by `repo` for the call; the name is a static
    // NUL-terminated string.
    ErrorCode::check(unsafe {
        crate::refdb::ffi::git_refdb_lookup(&raw mut head, refdb.as_ptr(), c"HEAD".as_ptr())
    })?;
    // SAFETY: `head` is the live reference the lookup just returned (it
    // succeeded), owned here.
    let result = if unsafe { r::git_reference_type(head) }
        != crate::repository::handles::GIT_REFERENCE_SYMBOLIC
    {
        None
    } else {
        // SAFETY: `head` is the live reference owned here, as above.
        let p = unsafe { r::git_reference_symbolic_target(head) };
        Some(if p.is_null() {
            Vec::new()
        } else {
            // SAFETY: a non-null target is a NUL-terminated string owned by
            // `head`, which is freed only after this copy, and that nothing
            // writes.
            unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes().to_vec()
        })
    };
    // SAFETY: `head` is the reference the lookup gave us, released once;
    // nothing borrowed from it is used afterwards.
    unsafe { r::git_reference_free(head) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_one(
        input: &[u8],
        orig_key: &[u8],
        key: &[u8],
        preg: Option<&Regexp>,
        value: Option<&[u8]>,
    ) -> Vec<u8> {
        let (section, name) = split_key(key);
        let (orig_section, orig_name) = split_key(orig_key);
        let mut w = Writer {
            buf: Vec::new(),
            buffered_comment: Vec::new(),
            in_section: false,
            preg_replaced: false,
            orig_section,
            section,
            orig_name,
            name,
            preg,
            value,
        };
        parse(Some(b"f"), input, &mut w).unwrap();
        w.buf
    }

    #[test]
    #[cfg_attr(miri, ignore)] // the repository's free reaches C
    fn the_repository_is_not_reached_after_its_free() {
        let _g = crate::util::test_support::init();
        let repo = crate::repository::GitRepository::alloc();
        assert!(!repo.is_null());
        // As `open` keeps it, without a reference.
        let b = FileBackend {
            values: Mutex::new(None),
            repo: AtomicPtr::new(repo),
            repo_serial: AtomicU64::new(crate::repository::live::token(repo).serial()),
            level: AtomicI32::new(0),
            file: Mutex::new(ConfigFile::new(b"/nonexistent/config").unwrap()),
            locked: Mutex::new(None),
        };
        assert!(b.with_repo(|r| r.is_some()));
        // SAFETY: the repository allocated above, which nothing else uses
        // (the backend does not hold a reference: C's model).
        unsafe { crate::repository::GitRepository::free(repo) };
        // A refresh now reads the conditions without a repository instead
        // of reading freed memory.
        assert!(b.with_repo(|r| r.is_none()));
    }

    #[test]
    fn writes_keep_the_file() {
        let _g = crate::util::test_support::init();
        let input = b"# top\n[core]\n\tbare = false ; c\n\n[other]\n\tx = 1\n";
        assert_eq!(
            write_one(input, b"core.bare", b"core.bare", None, Some(b"true")),
            b"# top\n[core]\n\tbare = true\n\n[other]\n\tx = 1\n"
        );
        assert_eq!(
            write_one(input, b"core.New", b"core.new", None, Some(b"a b ")),
            b"# top\n[core]\n\tbare = false ; c\n\tNew = \"a b \"\n\n[other]\n\tx = 1\n"
        );
        assert_eq!(
            write_one(input, b"Sec.Sub.k", b"sec.Sub.k", None, Some(b"v")),
            b"# top\n[core]\n\tbare = false ; c\n\n[other]\n\tx = 1\n[Sec \"Sub\"]\n\tk = v\n"
        );
        assert_eq!(
            write_one(input, b"core.bare", b"core.bare", None, None),
            b"# top\n[core]\n\n[other]\n\tx = 1\n"
        );
    }

    #[test]
    fn escapes_and_quotes() {
        assert_eq!(
            escape_value(b"a\"b\\c\nd\te").unwrap(),
            b"a\\\"b\\\\c\\nd\\te"
        );
        assert_eq!(quotes_for_value(b""), b"\"");
        assert_eq!(quotes_for_value(b" a"), b"\"");
        assert_eq!(quotes_for_value(b"a;b"), b"\"");
        assert_eq!(quotes_for_value(b"ab"), b"");
    }
}
