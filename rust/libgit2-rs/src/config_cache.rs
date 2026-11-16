//! Port of `src/libgit2/config_cache.c`: the repository's cache of
//! `core.*` values (`git_configmap_item`), read through the repository's
//! config and kept in `repo->configmap_cache`.
//!
//! The cache is an array of `intptr_t` in `git_repository`
//! (`GitRepository::configmap_cache`, atomics), which C reads and writes
//! atomically, as here.

use core::ffi::c_int;
use core::sync::atomic::Ordering;

use crate::config::{
    GIT_CONFIGMAP_FALSE, GIT_CONFIGMAP_INT32, GIT_CONFIGMAP_STRING, GIT_CONFIGMAP_TRUE, GitConfig,
    RawConfigMap, Want, lookup_map_value, parse_bool,
};
use crate::repository::{GIT_CONFIGMAP_CACHE_MAX, GitRepository};
use crate::util::errors::Result;

/// `GIT_CONFIGMAP_NOT_CACHED`.
pub const GIT_CONFIGMAP_NOT_CACHED: isize = -1;

const fn map(kind: c_int, s: Option<&'static [u8]>, v: c_int) -> RawConfigMap<'static> {
    RawConfigMap {
        kind,
        str_match: s,
        map_value: v,
    }
}

// The values of `repository.h`.
const GIT_EOL_UNSET: c_int = 0;
const GIT_EOL_CRLF: c_int = 1;
const GIT_EOL_LF: c_int = 2;
const GIT_EOL_NATIVE: c_int = GIT_EOL_LF;
const GIT_AUTO_CRLF_FALSE: c_int = 0;
const GIT_AUTO_CRLF_TRUE: c_int = 1;
const GIT_AUTO_CRLF_INPUT: c_int = 2;
const GIT_SAFE_CRLF_FALSE: c_int = 0;
const GIT_SAFE_CRLF_FAIL: c_int = 1;
const GIT_SAFE_CRLF_WARN: c_int = 2;
const GIT_LOGALLREFUPDATES_FALSE: c_int = 0;
const GIT_LOGALLREFUPDATES_TRUE: c_int = 1;
const GIT_LOGALLREFUPDATES_UNSET: c_int = 2;
const GIT_LOGALLREFUPDATES_ALWAYS: c_int = 3;
/// `GIT_OID_MAX_HEXSIZE` (SHA-256 is built in).
const GIT_ABBREV_FALSE: c_int = 64;
const GIT_ABBREV_DEFAULT: c_int = 7;

static EOL: [RawConfigMap<'static>; 4] = [
    map(GIT_CONFIGMAP_FALSE, None, GIT_EOL_UNSET),
    map(GIT_CONFIGMAP_STRING, Some(b"lf"), GIT_EOL_LF),
    map(GIT_CONFIGMAP_STRING, Some(b"crlf"), GIT_EOL_CRLF),
    map(GIT_CONFIGMAP_STRING, Some(b"native"), GIT_EOL_NATIVE),
];
static AUTOCRLF: [RawConfigMap<'static>; 3] = [
    map(GIT_CONFIGMAP_FALSE, None, GIT_AUTO_CRLF_FALSE),
    map(GIT_CONFIGMAP_TRUE, None, GIT_AUTO_CRLF_TRUE),
    map(GIT_CONFIGMAP_STRING, Some(b"input"), GIT_AUTO_CRLF_INPUT),
];
static SAFECRLF: [RawConfigMap<'static>; 3] = [
    map(GIT_CONFIGMAP_FALSE, None, GIT_SAFE_CRLF_FALSE),
    map(GIT_CONFIGMAP_TRUE, None, GIT_SAFE_CRLF_FAIL),
    map(GIT_CONFIGMAP_STRING, Some(b"warn"), GIT_SAFE_CRLF_WARN),
];
static LOGALLREFUPDATES: [RawConfigMap<'static>; 3] = [
    map(GIT_CONFIGMAP_FALSE, None, GIT_LOGALLREFUPDATES_FALSE),
    map(GIT_CONFIGMAP_TRUE, None, GIT_LOGALLREFUPDATES_TRUE),
    map(
        GIT_CONFIGMAP_STRING,
        Some(b"always"),
        GIT_LOGALLREFUPDATES_ALWAYS,
    ),
];
static ABBREV: [RawConfigMap<'static>; 3] = [
    map(GIT_CONFIGMAP_INT32, None, 0),
    map(GIT_CONFIGMAP_FALSE, None, GIT_ABBREV_FALSE),
    map(GIT_CONFIGMAP_STRING, Some(b"auto"), GIT_ABBREV_DEFAULT),
];

/// `struct map_data`.
struct MapData {
    name: &'static core::ffi::CStr,
    maps: &'static [RawConfigMap<'static>],
    default_value: c_int,
}

/// `_configmaps`, indexed by `git_configmap_item`.
static CONFIGMAPS: [MapData; GIT_CONFIGMAP_CACHE_MAX] = [
    MapData {
        name: c"core.autocrlf",
        maps: &AUTOCRLF,
        default_value: GIT_AUTO_CRLF_FALSE,
    },
    MapData {
        name: c"core.eol",
        maps: &EOL,
        default_value: GIT_EOL_NATIVE,
    },
    MapData {
        name: c"core.symlinks",
        maps: &[],
        default_value: GIT_CONFIGMAP_TRUE,
    },
    MapData {
        name: c"core.ignorecase",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.filemode",
        maps: &[],
        default_value: GIT_CONFIGMAP_TRUE,
    },
    MapData {
        name: c"core.ignorestat",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.trustctime",
        maps: &[],
        default_value: GIT_CONFIGMAP_TRUE,
    },
    MapData {
        name: c"core.abbrev",
        maps: &ABBREV,
        default_value: GIT_ABBREV_DEFAULT,
    },
    MapData {
        name: c"core.precomposeunicode",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.safecrlf",
        maps: &SAFECRLF,
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.logallrefupdates",
        maps: &LOGALLREFUPDATES,
        default_value: GIT_LOGALLREFUPDATES_UNSET,
    },
    MapData {
        name: c"core.protecthfs",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.protectntfs",
        maps: &[],
        default_value: GIT_CONFIGMAP_TRUE,
    },
    MapData {
        name: c"core.fsyncobjectfiles",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
    MapData {
        name: c"core.longpaths",
        maps: &[],
        default_value: GIT_CONFIGMAP_FALSE,
    },
];

/// `git_config__configmap_lookup`: the value of `item` in `config` (its
/// default when unset).
pub fn configmap_lookup(config: &GitConfig, item: usize) -> Result<c_int> {
    let data = &CONFIGMAPS[item];
    let entry = config.get_entry(data.name, false, Want::NoMissing)?;
    let Some(entry) = entry else {
        return Ok(data.default_value);
    };
    let value = entry.value();
    if data.maps.is_empty() {
        parse_bool(value).map(c_int::from)
    } else {
        lookup_map_value(data.maps, value)
    }
}

/// `git_repository__configmap_lookup`: the cached value of `item`, looked
/// up (and cached) the first time.
pub fn repository_configmap_lookup(repo: &GitRepository, item: usize) -> Result<c_int> {
    let cache = repo.configmap_cache();
    let value = cache[item].load(Ordering::SeqCst);
    if value != GIT_CONFIGMAP_NOT_CACHED {
        return Ok(value as c_int);
    }
    let config = repo.config_weak()?;
    let out = configmap_lookup(config, item)?;
    let _ = cache[item].compare_exchange(value, out as isize, Ordering::SeqCst, Ordering::SeqCst);
    Ok(out)
}

/// `git_repository__configmap_lookup_cache_clear`.
pub fn configmap_lookup_cache_clear(repo: &GitRepository) {
    for slot in repo.configmap_cache() {
        slot.store(GIT_CONFIGMAP_NOT_CACHED, Ordering::SeqCst);
    }
}

/// The exports of `config_cache.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// `git_config__configmap_lookup`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `config` is live; `item` is a
    /// `git_configmap_item`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_config__configmap_lookup(
        out: *mut c_int,
        config: *mut GitConfig,
        item: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            let r = configmap_lookup(unsafe { &*config }, item as usize);
            match r {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_repository__configmap_lookup`. As in C, `*out` is the cached
    /// value (-1 when not cached) even when the lookup then fails.
    ///
    /// # Safety
    ///
    /// `out` is writable; `repo` is live; `item` is a `git_configmap_item`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_repository__configmap_lookup(
        out: *mut c_int,
        repo: *mut GitRepository,
        item: c_int,
    ) -> c_int {
        barrier(|| {
            let item = item as usize;
            // SAFETY: per the contract: a live repository (`Sync`).
            let repo = unsafe { &*repo };
            let cached = repo.configmap_cache()[item].load(Ordering::SeqCst);
            // SAFETY: `out` is writable.
            unsafe { out.write(cached as c_int) };
            match repository_configmap_lookup(repo, item) {
                // SAFETY: `out` is writable.
                Ok(v) => unsafe {
                    out.write(v);
                    0
                },
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_repository__configmap_lookup_cache_clear`.
    ///
    /// # Safety
    ///
    /// `repo` is live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_repository__configmap_lookup_cache_clear(
        repo: *mut GitRepository,
    ) {
        // SAFETY: per the contract: a live repository (`Sync`).
        barrier(|| configmap_lookup_cache_clear(unsafe { &*repo }))
    }
}

crate::c_layout_test! {
    headers: ["repository.h"],
    types: [],
    asserts: [
        "offsetof(git_repository, configmap_cache) == 272",
        "sizeof(((git_repository *)0)->configmap_cache) == 15 * sizeof(intptr_t)",
        "GIT_CONFIGMAP_CACHE_MAX == 15",
        "GIT_ABBREV_FALSE == 64",
        "GIT_EOL_NATIVE == 2",
        "GIT_LOGALLREFUPDATES_UNSET == 2",
        "GIT_CONFIGMAP_NOT_CACHED == -1",
    ],
}
