//! Port of `src/libgit2/index_map.c`: the index's map from (path, stage)
//! to entry, case-sensitive or not (`git_index_entrymap`).
//!
//! Only `index.c` used the map, so it is private to the port: it lives in
//! the Rust part of `git_index` (the header no longer has it), and
//! `index_map.c`'s functions, which nothing else referenced, are not
//! exported. It keeps the C's khash ([`GitHashSet`]), hash (X31 over the
//! lower-cased path, plus the stage) and equality (the stage and `strcmp`
//! or `strcasecmp`), so lookups find what the C's found, including when
//! the map's own case flag and the index's disagree (the C sets them
//! separately).
//!
//! Keys are pointers to entries ([`EntryHead`]); a lookup key is an
//! [`EntryHead`] on the stack. Invariant: every key in the map points to a
//! live entry, which the map does not own.

use core::ptr::NonNull;

use super::{EntryHead, entry_path_raw, entry_stage_raw};
use crate::util::errors::{ErrorClass, Result};
use crate::util::hashmap::GitHashSet;
use crate::git_error_set;

/// A key: an entry (or a lookup key) that outlives its use in the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Key(pub(crate) NonNull<EntryHead>);

/// `git_index_entrymap_hash`: `__ac_X31_hash_string` of the lower-cased
/// path, plus the stage.
#[inline]
fn hash(k: &Key) -> u32 {
    let (path, stage) = parts(k);
    // `git__tolower` is `tolower((unsigned char)c)` on Unix (the C locale).
    let low = |c: u8| u32::from(c.to_ascii_lowercase());
    let mut h: u32 = 0;
    if let Some((&first, rest)) = path.split_first() {
        h = low(first);
        for &c in rest {
            h = (h << 5).wrapping_sub(h).wrapping_add(low(c));
        }
    }
    h.wrapping_add(stage)
}

/// A key's path and stage.
#[inline(always)]
fn parts<'a>(k: &Key) -> (&'a [u8], u32) {
    // SAFETY: the map's invariant: a key points to a live entry or to the
    // caller's lookup key, unwritten during the call.
    let path = unsafe { entry_path_raw(k.0.as_ptr()) };
    // SAFETY: as above.
    (path, unsafe { entry_stage_raw(k.0.as_ptr()) })
}

#[inline]
fn equal_default(a: &Key, b: &Key) -> bool {
    let ((pa, sa), (pb, sb)) = (parts(a), parts(b));
    sa == sb && pa == pb
}

#[inline]
fn equal_icase(a: &Key, b: &Key) -> bool {
    let ((pa, sa), (pb, sb)) = (parts(a), parts(b));
    sa == sb && pa.eq_ignore_ascii_case(pb)
}

/// `git_index_entrymap`.
#[derive(Default)]
pub(crate) struct EntryMap {
    /// `ignore_case`: the equality in use.
    pub(crate) ignore_case: bool,
    set: GitHashSet<Key>,
}

impl core::fmt::Debug for EntryMap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EntryMap")
            .field("ignore_case", &self.ignore_case)
            .field("len", &self.set.len())
            .finish()
    }
}

impl EntryMap {
    fn eq(&self) -> fn(&Key, &Key) -> bool {
        if self.ignore_case { equal_icase } else { equal_default }
    }

    /// `git_index_entrymap_get`: the entry equal to `key`.
    ///
    /// # Safety
    ///
    /// `key` points to an entry head (with its path) readable for the call.
    pub(crate) unsafe fn get(&self, key: NonNull<EntryHead>) -> Option<NonNull<EntryHead>> {
        let k = Key(key);
        let idx = self.set.find_with(&k, hash, self.eq())?;
        self.set.at(idx).map(|(k, ())| k.0)
    }

    /// `git_index_entrymap_put`: insert `entry`, replacing an equal key.
    ///
    /// # Safety
    ///
    /// `entry` is a live entry that stays live until it is removed from the
    /// map (or the map is cleared or disposed).
    pub(crate) unsafe fn put(&mut self, entry: NonNull<EntryHead>) -> Result<()> {
        let eq = self.eq();
        self.set.put_with(Key(entry), (), hash, eq).map(|_| ())
    }

    /// `git_index_entrymap_remove` of the key equal to `entry`. When that
    /// key is another entry's, `entry`'s own key (if it is in the map,
    /// which can happen after the case flag changed) is removed too: the C
    /// left it, and freed the entry (see `rust/DEFECTS.md`).
    ///
    /// # Safety
    ///
    /// `entry` points to an entry head readable for the call.
    pub(crate) unsafe fn remove(&mut self, entry: NonNull<EntryHead>) {
        let k = Key(entry);
        let eq = self.eq();
        let removed = self.set.remove_with(&k, hash, eq);
        if let Some((other, ())) = removed
            && other != k
            && let Some(idx) = self.set.find_with(&k, hash, |a, b| a == b)
        {
            let _ = self.set.remove_at(idx);
        }
    }

    /// `git_index_entrymap_resize` (C: `count` must fit 32 bits).
    pub(crate) fn resize(&mut self, count: usize) -> Result<()> {
        let Ok(count) = u32::try_from(count) else {
            return Err(git_error_set!(ErrorClass::INDEX, "index map is out of bounds"));
        };
        self.set.resize_with(count, hash)
    }

    /// `git_index_entrymap_clear`.
    pub(crate) fn clear(&mut self) {
        self.set.clear();
    }

    /// `git_index_entrymap_swap`.
    pub(crate) fn swap(&mut self, other: &mut EntryMap) {
        core::mem::swap(self, other);
    }

    /// `git_index_entrymap_dispose`.
    pub(crate) fn dispose(&mut self) {
        self.set.dispose();
    }
}
