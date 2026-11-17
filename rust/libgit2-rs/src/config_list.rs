//! Port of `src/libgit2/config_list.c`: the list of entries a backend has
//! read, in file order, with a map from each name to its last entry.
//!
//! [`ConfigList`] is opaque to C (only the config modules used
//! `git_config_list`). It is built by one owner, then frozen into an
//! `Arc` ([`ConfigList::freeze`]) and never changed again; C's refcount is
//! the `Arc`'s. Each entry is a [`ListEntry`], whose first field is the
//! public `git_config_backend_entry` that libgit2 hands out: its `free`
//! drops one reference to the list, as `git_config_list_entry_free` did.
//!
//! Since a frozen list never changes, iterating it needs no copy: C's
//! `git_config_list_dup` before each iteration is not needed, and the
//! iterator ([`iterator_new`]) shares the list.

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint};
use core::hash::{BuildHasher, Hash, Hasher};
use core::ptr::NonNull;
use std::collections::HashMap;
use std::sync::Arc;

use ffibox::vtable::CTable;

use crate::config::{
    ConfigIterator, git_config_backend_entry, git_config_entry, git_config_iterator,
};
use crate::git_error_set;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};

/// A NUL-terminated string the list owns: `len` bytes and a NUL, allocated
/// by the list (Rust's allocator) and freed with it.
#[derive(Clone, Copy)]
struct Text {
    ptr: NonNull<u8>,
    /// The length with the NUL.
    alloc: usize,
}

impl Text {
    /// A copy of `parts`, concatenated, each followed by a NUL.
    fn new(parts: &[&[u8]]) -> Result<Text> {
        let total = parts.iter().map(|p| p.len() + 1).sum::<usize>();
        let mut v: Vec<u8> = Vec::new();
        v.try_reserve_exact(total).map_err(|_| errors::oom())?;
        for p in parts {
            v.extend_from_slice(p);
            v.push(0);
        }
        let b: Box<[u8]> = v.into_boxed_slice();
        let alloc = b.len();
        let ptr = NonNull::new(Box::into_raw(b).cast::<u8>()).ok_or_else(errors::oom)?;
        Ok(Text { ptr, alloc })
    }

    /// The bytes at `off..off + len`.
    ///
    /// # Safety
    ///
    /// The text is live (not yet [`free`](Self::free)d) for `'a`, and the
    /// range is within it.
    unsafe fn bytes<'a>(self, off: usize, len: usize) -> &'a [u8] {
        debug_assert!(off + len <= self.alloc);
        // SAFETY: `off <= off + len <= alloc` (this function's contract), so
        // `p` stays within the live allocation of `alloc` bytes.
        let p = unsafe { self.ptr.as_ptr().add(off) };
        // SAFETY: `len` bytes at `p` are within the allocation, initialized
        // by `new` and written by nothing after it, and the allocation is
        // not freed for `'a` (this function's contract); `u8` has
        // alignment 1 and `len <= alloc <= isize::MAX` (a `Box`'s size).
        unsafe { core::slice::from_raw_parts(p, len) }
    }

    /// The C string at `off`.
    fn c_ptr(self, off: usize) -> *const c_char {
        self.ptr.as_ptr().wrapping_add(off).cast_const().cast()
    }

    /// Free it.
    ///
    /// # Safety
    ///
    /// It came from [`new`](Self::new), and is not used again.
    unsafe fn free(self) {
        let slice = core::ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.alloc);
        // SAFETY: `ptr` and `alloc` are the `Box<[u8]>` that `new` leaked
        // (this function's contract), rebuilt and dropped once; no copy of
        // the text is used again.
        drop(unsafe { Box::from_raw(slice) });
    }
}

/// A map key: a name in one of the list's [`Text`]s.
#[derive(Clone, Copy)]
struct Key {
    ptr: NonNull<u8>,
    len: usize,
}

impl Key {
    fn bytes(&self) -> &[u8] {
        // SAFETY: keys are made only by `ConfigList::push`, over the name
        // (`len` initialized bytes) at the start of an entry's `Text`; a key
        // lives only in that list's map, which is cleared before the texts
        // are freed (`ConfigList::drop`), and nothing writes a text.
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}
impl Eq for Key {}
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes().hash(state)
    }
}
impl core::borrow::Borrow<[u8]> for Key {
    fn borrow(&self) -> &[u8] {
        self.bytes()
    }
}

/// A fast string hash for the name map (FxHash's mixing, a word at a
/// time): the iteration order of the map is never observable.
#[derive(Clone, Copy, Default)]
struct FnvBuild;
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // A word at a time (FxHash's step), then the tail byte-wise.
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut h = self.0;
        let (words, tail) = bytes.as_chunks::<8>();
        for w in words {
            h = (h.rotate_left(5) ^ u64::from_le_bytes(*w)).wrapping_mul(K);
        }
        for &b in tail {
            h = (h.rotate_left(5) ^ u64::from(b)).wrapping_mul(K);
        }
        self.0 = h;
    }
}
impl BuildHasher for FnvBuild {
    type Hasher = Fnv;
    fn build_hasher(&self) -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

/// The map's value for a name: its last entry, and whether there are more.
#[derive(Clone, Copy)]
struct Head {
    last: usize,
    multivar: bool,
}

/// Where an entry was read: what C copies into each `git_config_entry`
/// besides its name and value.
///
/// The fields are crate-private: only this crate's readers build one, from
/// strings the list interned; [`ConfigList::push`] checks that.
#[derive(Clone, Copy, Debug)]
pub struct EntryMeta {
    /// `backend_type`, a string of the list ([`ConfigList::intern`]) or
    /// NULL.
    pub(crate) backend_type: *const c_char,
    /// `origin_path`, likewise.
    pub(crate) origin_path: *const c_char,
    /// `include_depth`.
    pub(crate) include_depth: c_uint,
    /// `level`.
    pub(crate) level: c_int,
}

impl EntryMeta {
    /// `include_depth`.
    pub fn include_depth(&self) -> c_uint {
        self.include_depth
    }

    /// `level`.
    pub fn level(&self) -> c_int {
        self.level
    }
}

/// `git_config_list_entry`: the public entry, then the list it belongs to.
///
/// `base` is what C sees (through the pointers libgit2 hands out), in an
/// `UnsafeCell`: C has a `git_config_entry *` and Rust never reads it, but
/// the Rust-side copies of what it holds (`text`, the lengths, `meta`).
#[repr(C)]
pub struct ListEntry {
    base: UnsafeCell<git_config_backend_entry>,
    /// The list, set by [`ConfigList::freeze`]: what `free` releases.
    list: *const ConfigList,
    /// `name\0` or `name\0value\0`.
    text: Text,
    name_len: usize,
    has_value: bool,
    meta: EntryMeta,
}

impl ListEntry {
    /// The normalized name.
    pub fn name(&self) -> &[u8] {
        // SAFETY: an entry exists only inside the list that made it (its
        // fields are private, it is neither `Clone` nor built elsewhere),
        // whose texts live as long as the list, which `&self` borrows;
        // `name_len < alloc` (the name and its NUL are the text's start).
        unsafe { self.text.bytes(0, self.name_len) }
    }

    /// The value (`None` for a bare name).
    pub fn value(&self) -> Option<&[u8]> {
        if !self.has_value {
            return None;
        }
        let off = self.name_len + 1;
        // SAFETY: as in `name`: the value and its NUL follow the name's NUL.
        Some(unsafe { self.text.bytes(off, self.text.alloc - off - 1) })
    }

    /// The value as a C string (NULL for none), as C sees it.
    pub fn value_ptr(&self) -> *const c_char {
        if self.has_value {
            self.text.c_ptr(self.name_len + 1)
        } else {
            core::ptr::null()
        }
    }

    /// Where it was read.
    pub fn meta(&self) -> EntryMeta {
        self.meta
    }

    /// The public entry C is handed: a pointer to the whole entry (its
    /// `free` reads `list`, after `base`), writable only in `base`.
    pub fn as_backend_entry(&self) -> *mut git_config_backend_entry {
        core::ptr::from_ref(self).cast_mut().cast()
    }
}

/// `git_config_list`: entries in file order, a map of names, and the
/// interned strings (backend types and paths) that entries point to.
pub struct ConfigList {
    entries: Vec<ListEntry>,
    map: HashMap<Key, Head, FnvBuild>,
    strings: Vec<Text>,
}

// SAFETY: the fields that are not `Send` by themselves are the raw
// pointers: each `Text` and `Key` points into an allocation the list owns
// (freed only by its `drop`), each entry's `base` strings into those texts,
// and each entry's `list` at the list itself. Moving the list moves all of
// them together, and `Text` is freed by Rust's allocator, from any thread.
unsafe impl Send for ConfigList {}
// SAFETY: a list is written only through `&mut` (before `freeze`); a
// frozen, shared list is never written: Rust only reads it, and C only
// reads the entries it is lent (`base` is an `UnsafeCell` for C's sake,
// but nothing writes it) and calls their `free`, which touches only the
// `Arc`'s atomic count.
unsafe impl Sync for ConfigList {}

impl Drop for ConfigList {
    fn drop(&mut self) {
        self.map.clear();
        for e in self.entries.drain(..) {
            // SAFETY: each entry's text came from `Text::new` and is the
            // entry's own, freed once here; the map's keys into it were
            // cleared just above, and the list is being dropped, so no
            // entry is lent any more (a lent entry holds a reference).
            unsafe { e.text.free() };
        }
        for s in self.strings.drain(..) {
            // SAFETY: as above, for the interned strings, which only the
            // entries' `base` (dropped above) point into.
            unsafe { s.free() };
        }
    }
}

impl core::fmt::Debug for ConfigList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConfigList")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl Default for ConfigList {
    fn default() -> Self {
        Self::new()
    }
}

/// `git_config_list_entry_free`: release the list the entry belongs to.
///
/// # Safety
///
/// `e` is an entry of a frozen list that was lent ([`ConfigList::lend`]),
/// released once.
unsafe extern "C" fn list_entry_free(e: *mut git_config_backend_entry) {
    let entry = e.cast::<ListEntry>();
    // SAFETY: `e` is the `base` (first field, `repr(C)`) of a live
    // `ListEntry`, whose `list` was set by `freeze` and never changes.
    let list = unsafe { (*entry).list };
    // SAFETY: `lend` added the reference this releases; `list` is the
    // `Arc`'s data pointer (see `freeze`).
    unsafe { Arc::decrement_strong_count(list) };
}

impl ConfigList {
    /// `git_config_list_new`.
    pub fn new() -> ConfigList {
        ConfigList {
            entries: Vec::new(),
            map: HashMap::with_hasher(FnvBuild),
            strings: Vec::new(),
        }
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries, in order.
    pub fn entries(&self) -> &[ListEntry] {
        &self.entries
    }

    /// `git_config_list_add_string`: the list's copy of `s` (one per
    /// distinct string).
    pub fn intern(&mut self, s: &[u8]) -> Result<*const c_char> {
        for t in &self.strings {
            // SAFETY: an interned text of this list, live while `self` is
            // borrowed; `alloc - 1` (its length without the NUL) is within
            // it.
            if unsafe { t.bytes(0, t.alloc - 1) } == s {
                return Ok(t.c_ptr(0));
            }
        }
        self.strings.try_reserve(1).map_err(|_| errors::oom())?;
        let t = Text::new(&[s])?;
        self.strings.push(t);
        Ok(t.c_ptr(0))
    }

    /// `git_config_list_append`: add an entry (a multivar if its name is
    /// already there).
    ///
    /// # Panics
    ///
    /// If `meta`'s strings are not NULL or strings this list interned.
    pub fn push(&mut self, name: &[u8], value: Option<&[u8]>, meta: EntryMeta) -> Result<()> {
        let owned = |p: *const c_char| {
            p.is_null() || self.strings.iter().any(|t| core::ptr::eq(t.c_ptr(0), p))
        };
        assert!(
            owned(meta.backend_type) && owned(meta.origin_path),
            "EntryMeta strings must be interned in the list they are pushed to"
        );
        self.entries.try_reserve(1).map_err(|_| errors::oom())?;
        self.map.try_reserve(1).map_err(|_| errors::oom())?;
        let text = match value {
            Some(v) => Text::new(&[name, v])?,
            None => Text::new(&[name])?,
        };
        let base = git_config_backend_entry {
            entry: git_config_entry {
                name: text.c_ptr(0),
                value: if value.is_some() {
                    text.c_ptr(name.len() + 1)
                } else {
                    core::ptr::null()
                },
                backend_type: meta.backend_type,
                origin_path: meta.origin_path,
                include_depth: meta.include_depth,
                level: meta.level,
            },
            free: Some(list_entry_free),
        };
        let index = self.entries.len();
        self.entries.push(ListEntry {
            base: UnsafeCell::new(base),
            list: core::ptr::null(),
            text,
            name_len: name.len(),
            has_value: value.is_some(),
            meta,
        });
        match self.map.get_mut(name) {
            Some(head) => {
                head.multivar = true;
                head.last = index;
            }
            None => {
                let key = Key {
                    ptr: text.ptr,
                    len: name.len(),
                };
                self.map.insert(
                    key,
                    Head {
                        last: index,
                        multivar: false,
                    },
                );
            }
        }
        Ok(())
    }

    /// `git_config_list_dup_entry`: add a copy of `entry`, interning its
    /// backend type and origin.
    ///
    /// # Safety
    ///
    /// `entry` is a valid `git_config_entry` (its strings NUL-terminated or
    /// NULL), not written during the call.
    pub unsafe fn push_c(&mut self, entry: *const git_config_entry) -> Result<()> {
        // SAFETY: `entry` is a valid, aligned `git_config_entry` that
        // nothing writes during the call (this function's contract).
        let e = unsafe { &*entry };
        let cs = |p: *const c_char| -> Option<&[u8]> {
            // SAFETY: each string of the entry is NULL (excluded here) or
            // NUL-terminated, unwritten during the call (this function's
            // contract); the bytes are copied before it returns.
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes())
        };
        let name = cs(e.name).unwrap_or(b"");
        let value = cs(e.value);
        let backend_type = match cs(e.backend_type) {
            Some(t) => self.intern(t)?,
            None => core::ptr::null(),
        };
        let origin_path = match cs(e.origin_path) {
            Some(t) => self.intern(t)?,
            None => core::ptr::null(),
        };
        self.push(
            name,
            value,
            EntryMeta {
                backend_type,
                origin_path,
                include_depth: e.include_depth,
                level: e.level,
            },
        )
    }

    /// `git_config_list_get`: the last entry named `key`.
    pub fn get(&self, key: &[u8]) -> Option<&ListEntry> {
        self.map.get(key).map(|h| &self.entries[h.last])
    }

    /// `git_config_list_get_unique`: the entry named `key`, which must be
    /// neither a multivar nor included; `GIT_ENOTFOUND` (no message) if
    /// there is none.
    pub fn get_unique(&self, key: &[u8]) -> Result<&ListEntry> {
        let Some(head) = self.map.get(key) else {
            return Err(ErrorCode::NOT_FOUND);
        };
        if head.multivar {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "entry is not unique due to being a multivar"
            ));
        }
        let e = &self.entries[head.last];
        if e.meta.include_depth != 0 {
            return Err(git_error_set!(
                ErrorClass::CONFIG,
                "entry is not unique due to being included"
            ));
        }
        Ok(e)
    }

    /// Share the list: from here on it is never changed, and each entry's
    /// `free` releases a reference to it.
    pub fn freeze(self) -> Arc<ConfigList> {
        let mut me = self;
        Arc::new_cyclic(move |weak| {
            // The `Arc`'s data pointer (what `Arc::into_raw` returns).
            let p = weak.as_ptr();
            for e in &mut me.entries {
                e.list = p;
            }
            me
        })
    }

    /// Lend `entry` (of `list`) to C: a reference to the list, which the
    /// entry's `free` releases (`git_config_list_incref` and `*out =
    /// &entry->base`).
    ///
    /// # Panics
    ///
    /// If `entry` is not an entry of `list` (an entry of an unfrozen list,
    /// for one, has no list to lend).
    pub fn lend(list: &Arc<ConfigList>, entry: &ListEntry) -> *mut git_config_backend_entry {
        assert!(
            core::ptr::eq(entry.list, Arc::as_ptr(list)),
            "ConfigList::lend: the entry is not one of this list's"
        );
        // SAFETY: `entry.list` is `list`'s data pointer (`freeze`), and
        // `list` keeps the count above zero during the call.
        unsafe { Arc::increment_strong_count(entry.list) };
        entry.as_backend_entry()
    }
}

/// `config_list_iterator`: the entries of a list, in order.
#[derive(Debug)]
pub struct ListIter {
    list: Arc<ConfigList>,
    next: usize,
}

impl ListIter {
    /// An iterator over `list`.
    pub fn new(list: Arc<ConfigList>) -> ListIter {
        ListIter { list, next: 0 }
    }

    /// `config_iterator_next`: the next entry; `None` at the end.
    pub fn next_entry(&mut self) -> Option<&ListEntry> {
        let e = self.list.entries.get(self.next)?;
        self.next += 1;
        Some(e)
    }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes an iterator's `next` the iterator, second, and
    // frees it once through `free`; `next` and `free` are both set.
    unsafe impl CVtable<ListIter> for ConfigIterator {}
    base = git_config_iterator::default();
    /// `config_iterator_next`.
    fn next(entry: *mut *mut git_config_backend_entry, @iter) -> c_int {
        on panic: crate::config::slot_panicked(),
        on busy: crate::config::iter_busy(),
        |it: &mut ListIter| match it.next_entry() {
            Some(e) => {
                // SAFETY: `entry` is aligned and writable for the call
                // (`next`'s contract, sys/config.h), and nothing
                // references `*entry`.
                unsafe { entry.write(e.as_backend_entry()) };
                0
            }
            None => ErrorCode::ITER_OVER.raw(),
        },
    }
    release free;
}

/// `git_config_list_iterator_new`: a C iterator over `list` (sharing it).
pub fn iterator_new(list: Arc<ConfigList>) -> *mut git_config_iterator {
    CTable::<ConfigIterator, ListIter>::new(ListIter::new(list))
        .into_c()
        .as_ptr()
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)] // each block's SAFETY covers its sequence
mod tests {
    use super::*;

    fn meta() -> EntryMeta {
        EntryMeta {
            backend_type: core::ptr::null(),
            origin_path: core::ptr::null(),
            include_depth: 0,
            level: 5,
        }
    }

    #[test]
    fn push_get_and_lend() {
        let _g = crate::util::test_support::init();
        let mut l = ConfigList::new();
        let t = l.intern(b"file").unwrap();
        assert_eq!(l.intern(b"file").unwrap(), t);
        l.push(b"a.b", Some(b"1"), meta()).unwrap();
        l.push(b"a.c", None, meta()).unwrap();
        l.push(b"a.b", Some(b"2"), meta()).unwrap();
        assert_eq!(l.get(b"a.b").unwrap().value(), Some(&b"2"[..]));
        assert_eq!(l.get(b"a.c").unwrap().value(), None);
        assert!(l.get_unique(b"a.b").is_err());
        assert!(l.get_unique(b"a.c").is_ok());
        assert_eq!(l.get_unique(b"x").err(), Some(ErrorCode::NOT_FOUND));

        let l = l.freeze();
        let e = ConfigList::lend(&l, l.get(b"a.b").unwrap());
        assert_eq!(Arc::strong_count(&l), 2);
        // SAFETY: `e` is a lent entry, read and freed as C does.
        unsafe {
            let v = CStr::from_ptr((*e).entry.value);
            assert_eq!(v.to_bytes(), b"2");
            ((*e).free.unwrap())(e);
        }
        assert_eq!(Arc::strong_count(&l), 1);
    }

    #[test]
    #[should_panic(expected = "not one of this list's")]
    fn lend_unfrozen_entry_panics() {
        let _g = crate::util::test_support::init();
        let mut l = ConfigList::new();
        l.push(b"a.b", Some(b"1"), meta()).unwrap();
        let other = ConfigList::new().freeze();
        // An entry of an unfrozen list has no list to lend: a panic, not
        // an `Arc` made from NULL.
        let _ = ConfigList::lend(&other, &l.entries()[0]);
    }

    #[test]
    #[should_panic(expected = "interned in the list")]
    fn push_foreign_strings_panics() {
        let _g = crate::util::test_support::init();
        let mut a = ConfigList::new();
        let t = a.intern(b"file").unwrap();
        let mut b = ConfigList::new();
        let m = EntryMeta {
            backend_type: t,
            ..meta()
        };
        let _ = b.push(b"a.b", None, m);
    }

    #[test]
    fn c_iterator() {
        let _g = crate::util::test_support::init();
        let mut l = ConfigList::new();
        l.push(b"a.b", Some(b"1"), meta()).unwrap();
        l.push(b"a.c", Some(b"2"), meta()).unwrap();
        let l = l.freeze();
        let it = iterator_new(l.clone());
        let mut names = Vec::new();
        // SAFETY: as C drives an iterator: `next` until it fails, then `free`.
        unsafe {
            let mut e = core::ptr::null_mut();
            while ((*it).next.unwrap())(&mut e, it) == 0 {
                names.push(CStr::from_ptr((*e).entry.name).to_bytes().to_vec());
            }
            ((*it).free.unwrap())(it);
        }
        assert_eq!(names, [b"a.b".to_vec(), b"a.c".to_vec()]);
        assert_eq!(Arc::strong_count(&l), 1);
    }
}
