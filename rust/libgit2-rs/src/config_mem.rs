//! Port of `src/libgit2/config_mem.c`: read-only backends over a
//! configuration text (`git_config_backend_from_string`) or over
//! `key=value` strings (`git_config_backend_from_values`, the CLI's `-c`).
//!
//! The backend is a `CTable` (kind: opaque state behind the mirrored
//! `git_config_backend` table) whose slots C calls.

use core::ffi::{CStr, c_char, c_int, c_uint};
use std::sync::{Arc, Mutex};

use ffibox::vtable::CTable;

use crate::config::{
    Backend, ConfigBackendCell, GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION, git_config_backend,
    git_config_backend_entry, git_config_backend_memory_options, git_config_iterator, slot_busy,
    slot_panicked,
};
use crate::config_list::{ConfigList, EntryMeta};
use crate::config_parse::{self, ParseHandler};
use crate::git_error_set;
use crate::repository::GitRepository as git_repository;
use crate::util::cabi::{CStrArg, Out};
use crate::util::ctype_compat::tolower;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};

/// What a memory backend reads at `open`.
#[derive(Debug)]
enum Source {
    /// Configuration text.
    Text(Vec<u8>),
    /// `key=value` strings.
    Values(Vec<Vec<u8>>),
}

/// `config_memory_backend`.
#[derive(Debug)]
pub struct MemBackend {
    backend_type: Vec<u8>,
    origin_path: Option<Vec<u8>>,
    source: Source,
    list: Mutex<Arc<ConfigList>>,
}

/// `config_error_readonly`.
pub(crate) fn error_readonly() -> c_int {
    git_error_set!(ErrorClass::CONFIG, "this backend is read-only").raw()
}

/// `read_variable_cb`'s state.
struct ReadMem<'l> {
    list: &'l mut ConfigList,
    meta: EntryMeta,
}

/// `section.name`, the name lower-cased (`git__tolower`).
pub(crate) fn entry_name(section: Option<&[u8]>, name: &[u8]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let extra = section.map_or(0, |s| s.len() + 1);
    buf.try_reserve_exact(extra + name.len())
        .map_err(|_| errors::oom())?;
    if let Some(s) = section {
        buf.extend_from_slice(s);
        buf.push(b'.');
    }
    buf.extend(name.iter().map(|&c| tolower(c)));
    Ok(buf)
}

impl ParseHandler for ReadMem<'_> {
    fn on_variable(
        &mut self,
        section: Option<&[u8]>,
        name: &[u8],
        value: Option<&[u8]>,
        _line: &[u8],
    ) -> Result<()> {
        let n = entry_name(section, name)?;
        self.list.push(&n, value, self.meta)
    }
}

impl MemBackend {
    /// `parse_config`.
    fn parse_config(&self, list: &mut ConfigList, text: &[u8], level: c_int) -> Result<()> {
        let backend_type = list.intern(&self.backend_type)?;
        let origin_path = match &self.origin_path {
            Some(p) => list.intern(p)?,
            None => core::ptr::null(),
        };
        let mut h = ReadMem {
            list,
            meta: EntryMeta {
                backend_type,
                origin_path,
                include_depth: 0,
                level,
            },
        };
        config_parse::parse(Some(b"in-memory"), text, &mut h)
    }

    /// `parse_values`.
    fn parse_values(&self, list: &mut ConfigList, values: &[Vec<u8>], level: c_int) -> Result<()> {
        let backend_type = list.intern(&self.backend_type)?;
        let origin_path = match &self.origin_path {
            Some(p) => list.intern(p)?,
            None => core::ptr::null(),
        };
        let meta = EntryMeta {
            backend_type,
            origin_path,
            include_depth: 0,
            level,
        };
        for v in values {
            // Without `=`, C's `strndup(value, eql - value)` (with `eql`
            // NULL) copies the whole string: a name without a value.
            let (name, value) = match memchr::memchr(b'=', v) {
                Some(eq) => (&v[..eq], Some(&v[eq + 1..])),
                None => (&v[..], None),
            };
            if name.is_empty() {
                return Err(git_error_set!(ErrorClass::CONFIG, "empty config key"));
            }
            list.push(name, value, meta)?;
        }
        Ok(())
    }

    /// `config_memory_open`.
    fn open(&self, level: c_int) -> c_int {
        let mut list = ConfigList::new();
        let r = match &self.source {
            Source::Text(t) if !t.is_empty() => self.parse_config(&mut list, t, level),
            Source::Values(v) if !v.is_empty() => self.parse_values(&mut list, v, level),
            _ => Ok(()),
        };
        // As C, which appends as it parses, the entries read before a
        // failure stay.
        *self.list.lock().unwrap_or_else(|e| e.into_inner()) = list.freeze();
        if r.is_err() { -1 } else { 0 }
    }

    fn list(&self) -> Arc<ConfigList> {
        Arc::clone(&self.list.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// `config_memory_get`.
    fn get(&self, key: &[u8], mut out: Out<'_, *mut git_config_backend_entry>) -> c_int {
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        match list.get(key) {
            None => ErrorCode::NOT_FOUND.raw(),
            Some(e) => {
                out.write(ConfigList::lend(&list, e));
                0
            }
        }
    }

    /// `config_memory_iterator`.
    fn iterator(&self, mut out: Out<'_, *mut git_config_iterator>) -> c_int {
        out.write(crate::config_list::iterator_new(self.list()));
        0
    }
}

/// A slot's C string argument.
///
/// # Safety
///
/// `p` is NULL or NUL-terminated, and unwritten for `'a` (the slot's
/// call).
pub(crate) unsafe fn arg<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: this function's contract is `CStrArg::from_ptr`'s; NULL is
    // `""`.
    unsafe { CStrArg::from_ptr(p) }.to_bytes()
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
    // `readonly` is 1: the writes fail as C's do.
    unsafe impl CVtable<MemBackend> for ConfigBackendCell {}
    base = git_config_backend {
        readonly: 1,
        snapshot: Some(crate::config_snapshot::ffi::git_config_backend_snapshot),
        ..git_config_backend::default()
    };
    /// `config_memory_open`.
    fn open(@cfg, level: c_int, repo: *const git_repository) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |b: &MemBackend| {
            let _ = repo;
            b.open(level)
        },
    }
    /// `config_memory_get`.
    fn get(@cfg, key: *const c_char, out: *mut *mut git_config_backend_entry) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `key` is a NUL-terminated string and `out` aligned and
        // writable for the call (the slot's contract, sys/config.h);
        // nothing references `*out`.
        |b: &MemBackend| b.get(unsafe { arg(key) }, unsafe { Out::from_ptr(out) }),
    }
    fn set(@cfg, name: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| {
            let _ = (name, value);
            error_readonly()
        },
    }
    fn set_multivar(@cfg, name: *const c_char, regexp: *const c_char, value: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| {
            let _ = (name, regexp, value);
            error_readonly()
        },
    }
    fn del(@cfg, name: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| {
            let _ = name;
            error_readonly()
        },
    }
    fn del_multivar(@cfg, name: *const c_char, regexp: *const c_char) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| {
            let _ = (name, regexp);
            error_readonly()
        },
    }
    /// `config_memory_iterator`.
    fn iterator(out: *mut *mut git_config_iterator, @cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        // SAFETY: `out` is aligned and writable for the call (the slot's
        // contract), and nothing references `*out`.
        |b: &MemBackend| b.iterator(unsafe { Out::from_ptr(out) }),
    }
    fn lock(@cfg) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| error_readonly(),
    }
    fn unlock(@cfg, success: c_int) -> c_int {
        on panic: slot_panicked(),
        on busy: slot_busy(),
        |_b: &MemBackend| {
            let _ = success;
            error_readonly()
        },
    }
    release free;
}

/// `config_backend_new`.
fn backend_new(source: Source, backend_type: Option<&[u8]>, origin_path: Option<&[u8]>) -> Backend {
    let state = MemBackend {
        backend_type: backend_type.unwrap_or(b"in-memory").to_vec(),
        origin_path: origin_path.map(<[u8]>::to_vec),
        source,
        list: Mutex::new(ConfigList::new().freeze()),
    };
    let p = CTable::<ConfigBackendCell, MemBackend>::new(state).into_c();
    // SAFETY: `p` is the table `into_c` surrendered just now, owned by
    // nothing else, and released through its `free` slot
    // (`CTable::release`), which `Backend`'s policy calls.
    unsafe { Backend::from_c(p.as_ptr().cast()) }.unwrap_or_else(|| unreachable!())
}

/// `git_config_backend_from_string`.
pub fn backend_from_string(
    text: &[u8],
    backend_type: Option<&[u8]>,
    origin_path: Option<&[u8]>,
) -> Result<Backend> {
    let mut t = Vec::new();
    t.try_reserve_exact(text.len()).map_err(|_| errors::oom())?;
    t.extend_from_slice(text);
    Ok(backend_new(Source::Text(t), backend_type, origin_path))
}

/// `git_config_backend_from_values`.
pub fn backend_from_values(
    values: &[&[u8]],
    backend_type: Option<&[u8]>,
    origin_path: Option<&[u8]>,
) -> Result<Backend> {
    let mut v = Vec::new();
    v.try_reserve_exact(values.len())
        .map_err(|_| errors::oom())?;
    for s in values {
        v.push(s.to_vec());
    }
    Ok(backend_new(Source::Values(v), backend_type, origin_path))
}

/// The exports of `config_mem.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;
    use crate::{git_assert_arg, git_error_set};

    /// The options' strings.
    ///
    /// # Safety
    ///
    /// The strings of `opts` are NULL or NUL-terminated, and unwritten for
    /// `'a`.
    unsafe fn options<'a>(
        opts: Option<&git_config_backend_memory_options>,
    ) -> (Option<&'a [u8]>, Option<&'a [u8]>) {
        let Some(o) = opts else {
            return (None, None);
        };
        let s = |p: *const c_char| {
            // SAFETY: each string is NULL or NUL-terminated, unwritten for
            // `'a` (this function's contract).
            unsafe { CStrArg::from_ptr(p) }.get().map(CStr::to_bytes)
        };
        (s(o.backend_type), s(o.origin_path))
    }

    /// `git_config_backend_memory_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_memory_options_init(
        mut opts: Out<'_, git_config_backend_memory_options>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            if version == 0 || version > GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION {
                return git_error_set!(
                    ErrorClass::INVALID,
                    "invalid version {} on git_config_backend_memory_options",
                    version as c_int
                )
                .raw();
            }
            opts.write(git_config_backend_memory_options {
                version: GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION,
                backend_type: core::ptr::null(),
                origin_path: core::ptr::null(),
            });
            0
        })
    }

    /// `git_config_backend_from_string`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `cfg` is readable for `len` bytes; `opts` is NULL
    /// or valid options whose strings are NULL or NUL-terminated, none
    /// written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_from_string(
        mut out: Out<'_, *mut git_config_backend>,
        cfg: *const c_char,
        len: usize,
        opts: Option<&git_config_backend_memory_options>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `cfg` is readable, and unwritten, for `len` bytes
            // during the call (this export's contract).
            let text = unsafe { crate::util::export::bytes(cfg.cast(), len) };
            // SAFETY: the strings of `opts` are NULL or NUL-terminated,
            // unwritten during the call (this export's contract).
            let (t, o) = unsafe { options(opts) };
            match backend_from_string(text, t, o) {
                Ok(b) => {
                    out.write(b.into_c().cast());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_config_backend_from_values`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `values` is readable for `len` NUL-terminated
    /// strings; `opts` as for `git_config_backend_from_string`; none is
    /// written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config_backend_from_values(
        mut out: Out<'_, *mut git_config_backend>,
        values: *const *const c_char,
        len: usize,
        opts: Option<&git_config_backend_memory_options>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `values` is NULL or an aligned C array of `len`
            // pointers, readable and unwritten during the call (this
            // export's contract).
            let ptrs = match unsafe { crate::util::cabi::slice_arg(values, len, "values") } {
                Ok(p) => p,
                Err(e) => return e.raw(),
            };
            // SAFETY: each pointer is a NUL-terminated string, unwritten
            // during the call (this export's contract).
            let v: Vec<&[u8]> = ptrs.iter().map(|&p| unsafe { arg(p) }).collect();
            // SAFETY: as in `git_config_backend_from_string`.
            let (t, o) = unsafe { options(opts) };
            match backend_from_values(&v, t, o) {
                Ok(b) => {
                    out.write(b.into_c().cast());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }
}
