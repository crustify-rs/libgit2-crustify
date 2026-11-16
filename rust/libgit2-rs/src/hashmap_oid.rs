//! A Rust equivalent of `src/libgit2/hashmap_oid.h`: hash maps keyed by
//! object id, with the C's hash (the first four id bytes) and equality
//! (`git_oid_equal`), so that their iteration order is the C's.
//!
//! - [`OidMap<'a, V>`]: keys are `&'a Oid` (C's `const git_oid *`, same
//!   layout), typically pointing into the values;
//! - [`GitHashMap<Oid, V>`](GitHashMap): keys by value, for maps that only
//!   Rust sees;
//! - [`OidPtr`]: a raw `const git_oid *` key for maps that C also touches,
//!   whose keys live as long as C says.
//!
//! ```
//! use libgit2_rs::__test::hashmap_oid::OidMap;
//! use libgit2_rs::{Oid, OidType};
//! # let _g = libgit2_rs::__test::util::test_support::init();
//! let a = Oid::parse_prefix(b"abcd", OidType::Sha1)?;
//! let mut m: OidMap<'_, u32> = OidMap::new();
//! m.put(&a, 1)?;
//! assert_eq!(m.get(&&a), Some(&1));
//! # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
//! ```

use core::ffi::c_void;
use core::ptr::NonNull;

use crate::oid::Oid;
use crate::util::hashmap::{GitHashMap, GitHashSet, HashKey};

/// `git_hashmap_oid_hashcode` and `git_oid_equal`.
impl HashKey for Oid {
    #[inline]
    fn hash_code(&self) -> u32 {
        self.hashcode()
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        self == other
    }
}

/// `git_hashmap_oid_hashcode` and `git_oid_equal`.
impl HashKey for &Oid {
    #[inline]
    fn hash_code(&self) -> u32 {
        self.hashcode()
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

/// A `const git_oid *` key whose lifetime C (or the map's owner) vouches
/// for.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OidPtr(NonNull<Oid>);

// SAFETY: a pointer to an immutable oid, read from any thread.
unsafe impl Send for OidPtr {}
// SAFETY: as above.
unsafe impl Sync for OidPtr {}

impl OidPtr {
    /// The key for `p`.
    ///
    /// # Safety
    ///
    /// `p` points to an oid that stays valid and unwritten for as long as
    /// the key is used (in a map, until it is removed or the map is
    /// cleared or disposed).
    #[inline]
    pub const unsafe fn new(p: NonNull<Oid>) -> OidPtr {
        OidPtr(p)
    }

    /// The pointer.
    #[inline]
    pub const fn as_ptr(&self) -> *const Oid {
        self.0.as_ptr()
    }

    /// The oid.
    #[inline]
    pub fn get(&self) -> &Oid {
        // SAFETY: the constructor's contract.
        unsafe { self.0.as_ref() }
    }
}

impl HashKey for OidPtr {
    #[inline]
    fn hash_code(&self) -> u32 {
        self.get().hashcode()
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

/// `GIT_HASHMAP_OID_STRUCT(name, val_t)` with borrowed keys.
pub type OidMap<'a, V> = GitHashMap<&'a Oid, V>;

/// A set of borrowed oids (`GIT_HASHSET_SETUP(name, const git_oid *, …)`).
pub type OidSet<'a> = GitHashSet<&'a Oid>;

/// An oid map that C also sees, with `void *` values.
pub type OidPtrMap = GitHashMap<OidPtr, *mut c_void>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid::OidType;

    fn oid(i: u32) -> Oid {
        let mut id = [0u8; 32];
        id[..4].copy_from_slice(&i.wrapping_mul(2_654_435_761).to_be_bytes());
        id[4] = i as u8;
        Oid::from_raw_parts(OidType::Sha1 as u8, id)
    }

    #[test]
    fn borrowed_and_owned_keys_iterate_alike() {
        let _g = crate::util::test_support::init();
        let ids: Vec<Oid> = (0..500).map(oid).collect();
        let mut a: OidMap<'_, usize> = OidMap::new();
        let mut b: GitHashMap<Oid, usize> = GitHashMap::new();
        let mut c = OidPtrMap::new();
        for (i, id) in ids.iter().enumerate() {
            a.put(id, i).unwrap();
            b.put(*id, i).unwrap();
            // SAFETY: `ids` outlives `c`.
            c.put(
                unsafe { OidPtr::new(NonNull::from(id)) },
                core::ptr::null_mut(),
            )
            .unwrap();
        }
        for i in (0..500).step_by(3) {
            a.remove(&&ids[i]);
            b.remove(&ids[i]);
        }
        let ka: Vec<Oid> = a.keys().map(|k| **k).collect();
        let kb: Vec<Oid> = b.keys().copied().collect();
        assert_eq!(ka, kb);
        assert_eq!(a.get(&&ids[1]), Some(&1));
        assert_eq!(c.len(), 500);
        // SAFETY: `ids` outlives the key.
        let k = unsafe { OidPtr::new(NonNull::from(&ids[7])) };
        assert!(c.contains(&k));
    }
}
