//! Port of `src/libgit2/delta.c`: git's binary delta format (the "diff-delta"
//! code taken from git.git): applying a delta to its base, reading its
//! header, and creating a delta from a Rabin-fingerprint index of the base.
//!
//! # Types
//!
//! `git_delta_index` is opaque to C, but C frees it: `pack-objects.c` calls
//! `git_delta_index_free` in one place and `git__free` in another. So, as in
//! C, an index is one `git__allocator` block (its header, the bucket heads,
//! the entries; see [`IndexHeader`]), smaller than the C's (indices instead
//! of pointers), and [`git_delta_index_size`](ffi::git_delta_index_size)
//! reports the size of the C's allocation (`memsize`), so the packer's
//! memory accounting is unchanged. In Rust it is [`DeltaIndex<'a>`], the
//! block's owner, which borrows its source buffer: C's contract ("the given
//! buffer must not be freed nor altered before `git_delta_index_free`") is
//! the `'a`, and the exports state it.
//!
//! # Equivalence
//!
//! Deltas are byte for byte the C's: same Rabin windows, same hash sizing,
//! the same culling of over-full buckets (including its quirk: a bucket of
//! 64..127 entries keeps only its first one), the same choice among equal
//! matches, the same one-byte back-matching and copy encoding, the same
//! output growth and `max_size` cut-off. Error codes, classes and messages
//! are the C's. Every read and write is bounds-checked.

use core::alloc::Allocator;
use core::ffi::{c_int, c_void};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::NonNull;

use crate::git_error_set;
use crate::util::alloc;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::integer::{alloc_add, alloc_multiply};

/// `HASH_LIMIT`: the maximum entries kept per hash bucket.
const HASH_LIMIT: u32 = 64;
/// `RABIN_SHIFT`.
const RABIN_SHIFT: u32 = 23;
/// `RABIN_WINDOW`.
const RABIN_WINDOW: usize = 16;
/// `MAX_OP_SIZE`: the largest opcode sequence (header, window, copy).
const MAX_OP_SIZE: usize = 5 + 5 + 1 + RABIN_WINDOW + 7;
/// `GIT_DELTA_SIZE_MIN`: the smallest possible delta.
pub const GIT_DELTA_SIZE_MIN: usize = 4;

/// `sizeof(struct git_delta_index)` in the C (its fixed part, before the
/// flexible hash array): `unsigned long`, two pointers' worth and an
/// `unsigned int`, padded to 8.
const C_INDEX_HEADER_SIZE: usize = 32;
/// `sizeof(struct index_entry)` in the C: a pointer, an `unsigned int`, a
/// pointer.
const C_INDEX_ENTRY_SIZE: usize = 24;
/// `sizeof(struct index_entry *)`.
const C_POINTER_SIZE: usize = 8;
// The C sizes above are LP64's (the configuration this port builds).
const _: () =
    assert!(size_of::<*const u8>() == C_POINTER_SIZE && size_of::<core::ffi::c_ulong>() == 8);

/// The Rabin polynomial table `T` of the C.
static T: [u32; 256] = [
    0x00000000, 0xab59b4d1, 0x56b369a2, 0xfdeadd73, 0x063f6795, 0xad66d344, 0x508c0e37, 0xfbd5bae6,
    0x0c7ecf2a, 0xa7277bfb, 0x5acda688, 0xf1941259, 0x0a41a8bf, 0xa1181c6e, 0x5cf2c11d, 0xf7ab75cc,
    0x18fd9e54, 0xb3a42a85, 0x4e4ef7f6, 0xe5174327, 0x1ec2f9c1, 0xb59b4d10, 0x48719063, 0xe32824b2,
    0x1483517e, 0xbfdae5af, 0x423038dc, 0xe9698c0d, 0x12bc36eb, 0xb9e5823a, 0x440f5f49, 0xef56eb98,
    0x31fb3ca8, 0x9aa28879, 0x6748550a, 0xcc11e1db, 0x37c45b3d, 0x9c9defec, 0x6177329f, 0xca2e864e,
    0x3d85f382, 0x96dc4753, 0x6b369a20, 0xc06f2ef1, 0x3bba9417, 0x90e320c6, 0x6d09fdb5, 0xc6504964,
    0x2906a2fc, 0x825f162d, 0x7fb5cb5e, 0xd4ec7f8f, 0x2f39c569, 0x846071b8, 0x798aaccb, 0xd2d3181a,
    0x25786dd6, 0x8e21d907, 0x73cb0474, 0xd892b0a5, 0x23470a43, 0x881ebe92, 0x75f463e1, 0xdeadd730,
    0x63f67950, 0xc8afcd81, 0x354510f2, 0x9e1ca423, 0x65c91ec5, 0xce90aa14, 0x337a7767, 0x9823c3b6,
    0x6f88b67a, 0xc4d102ab, 0x393bdfd8, 0x92626b09, 0x69b7d1ef, 0xc2ee653e, 0x3f04b84d, 0x945d0c9c,
    0x7b0be704, 0xd05253d5, 0x2db88ea6, 0x86e13a77, 0x7d348091, 0xd66d3440, 0x2b87e933, 0x80de5de2,
    0x7775282e, 0xdc2c9cff, 0x21c6418c, 0x8a9ff55d, 0x714a4fbb, 0xda13fb6a, 0x27f92619, 0x8ca092c8,
    0x520d45f8, 0xf954f129, 0x04be2c5a, 0xafe7988b, 0x5432226d, 0xff6b96bc, 0x02814bcf, 0xa9d8ff1e,
    0x5e738ad2, 0xf52a3e03, 0x08c0e370, 0xa39957a1, 0x584ced47, 0xf3155996, 0x0eff84e5, 0xa5a63034,
    0x4af0dbac, 0xe1a96f7d, 0x1c43b20e, 0xb71a06df, 0x4ccfbc39, 0xe79608e8, 0x1a7cd59b, 0xb125614a,
    0x468e1486, 0xedd7a057, 0x103d7d24, 0xbb64c9f5, 0x40b17313, 0xebe8c7c2, 0x16021ab1, 0xbd5bae60,
    0x6cb54671, 0xc7ecf2a0, 0x3a062fd3, 0x915f9b02, 0x6a8a21e4, 0xc1d39535, 0x3c394846, 0x9760fc97,
    0x60cb895b, 0xcb923d8a, 0x3678e0f9, 0x9d215428, 0x66f4eece, 0xcdad5a1f, 0x3047876c, 0x9b1e33bd,
    0x7448d825, 0xdf116cf4, 0x22fbb187, 0x89a20556, 0x7277bfb0, 0xd92e0b61, 0x24c4d612, 0x8f9d62c3,
    0x7836170f, 0xd36fa3de, 0x2e857ead, 0x85dcca7c, 0x7e09709a, 0xd550c44b, 0x28ba1938, 0x83e3ade9,
    0x5d4e7ad9, 0xf617ce08, 0x0bfd137b, 0xa0a4a7aa, 0x5b711d4c, 0xf028a99d, 0x0dc274ee, 0xa69bc03f,
    0x5130b5f3, 0xfa690122, 0x0783dc51, 0xacda6880, 0x570fd266, 0xfc5666b7, 0x01bcbbc4, 0xaae50f15,
    0x45b3e48d, 0xeeea505c, 0x13008d2f, 0xb85939fe, 0x438c8318, 0xe8d537c9, 0x153feaba, 0xbe665e6b,
    0x49cd2ba7, 0xe2949f76, 0x1f7e4205, 0xb427f6d4, 0x4ff24c32, 0xe4abf8e3, 0x19412590, 0xb2189141,
    0x0f433f21, 0xa41a8bf0, 0x59f05683, 0xf2a9e252, 0x097c58b4, 0xa225ec65, 0x5fcf3116, 0xf49685c7,
    0x033df00b, 0xa86444da, 0x558e99a9, 0xfed72d78, 0x0502979e, 0xae5b234f, 0x53b1fe3c, 0xf8e84aed,
    0x17bea175, 0xbce715a4, 0x410dc8d7, 0xea547c06, 0x1181c6e0, 0xbad87231, 0x4732af42, 0xec6b1b93,
    0x1bc06e5f, 0xb099da8e, 0x4d7307fd, 0xe62ab32c, 0x1dff09ca, 0xb6a6bd1b, 0x4b4c6068, 0xe015d4b9,
    0x3eb80389, 0x95e1b758, 0x680b6a2b, 0xc352defa, 0x3887641c, 0x93ded0cd, 0x6e340dbe, 0xc56db96f,
    0x32c6cca3, 0x999f7872, 0x6475a501, 0xcf2c11d0, 0x34f9ab36, 0x9fa01fe7, 0x624ac294, 0xc9137645,
    0x26459ddd, 0x8d1c290c, 0x70f6f47f, 0xdbaf40ae, 0x207afa48, 0x8b234e99, 0x76c993ea, 0xdd90273b,
    0x2a3b52f7, 0x8162e626, 0x7c883b55, 0xd7d18f84, 0x2c043562, 0x875d81b3, 0x7ab75cc0, 0xd1eee811,
];

/// The table `U` of the C (removes a byte leaving the window).
static U: [u32; 256] = [
    0x00000000, 0x7eb5200d, 0x5633f4cb, 0x2886d4c6, 0x073e5d47, 0x798b7d4a, 0x510da98c, 0x2fb88981,
    0x0e7cba8e, 0x70c99a83, 0x584f4e45, 0x26fa6e48, 0x0942e7c9, 0x77f7c7c4, 0x5f711302, 0x21c4330f,
    0x1cf9751c, 0x624c5511, 0x4aca81d7, 0x347fa1da, 0x1bc7285b, 0x65720856, 0x4df4dc90, 0x3341fc9d,
    0x1285cf92, 0x6c30ef9f, 0x44b63b59, 0x3a031b54, 0x15bb92d5, 0x6b0eb2d8, 0x4388661e, 0x3d3d4613,
    0x39f2ea38, 0x4747ca35, 0x6fc11ef3, 0x11743efe, 0x3eccb77f, 0x40799772, 0x68ff43b4, 0x164a63b9,
    0x378e50b6, 0x493b70bb, 0x61bda47d, 0x1f088470, 0x30b00df1, 0x4e052dfc, 0x6683f93a, 0x1836d937,
    0x250b9f24, 0x5bbebf29, 0x73386bef, 0x0d8d4be2, 0x2235c263, 0x5c80e26e, 0x740636a8, 0x0ab316a5,
    0x2b7725aa, 0x55c205a7, 0x7d44d161, 0x03f1f16c, 0x2c4978ed, 0x52fc58e0, 0x7a7a8c26, 0x04cfac2b,
    0x73e5d470, 0x0d50f47d, 0x25d620bb, 0x5b6300b6, 0x74db8937, 0x0a6ea93a, 0x22e87dfc, 0x5c5d5df1,
    0x7d996efe, 0x032c4ef3, 0x2baa9a35, 0x551fba38, 0x7aa733b9, 0x041213b4, 0x2c94c772, 0x5221e77f,
    0x6f1ca16c, 0x11a98161, 0x392f55a7, 0x479a75aa, 0x6822fc2b, 0x1697dc26, 0x3e1108e0, 0x40a428ed,
    0x61601be2, 0x1fd53bef, 0x3753ef29, 0x49e6cf24, 0x665e46a5, 0x18eb66a8, 0x306db26e, 0x4ed89263,
    0x4a173e48, 0x34a21e45, 0x1c24ca83, 0x6291ea8e, 0x4d29630f, 0x339c4302, 0x1b1a97c4, 0x65afb7c9,
    0x446b84c6, 0x3adea4cb, 0x1258700d, 0x6ced5000, 0x4355d981, 0x3de0f98c, 0x15662d4a, 0x6bd30d47,
    0x56ee4b54, 0x285b6b59, 0x00ddbf9f, 0x7e689f92, 0x51d01613, 0x2f65361e, 0x07e3e2d8, 0x7956c2d5,
    0x5892f1da, 0x2627d1d7, 0x0ea10511, 0x7014251c, 0x5facac9d, 0x21198c90, 0x099f5856, 0x772a785b,
    0x4c921c31, 0x32273c3c, 0x1aa1e8fa, 0x6414c8f7, 0x4bac4176, 0x3519617b, 0x1d9fb5bd, 0x632a95b0,
    0x42eea6bf, 0x3c5b86b2, 0x14dd5274, 0x6a687279, 0x45d0fbf8, 0x3b65dbf5, 0x13e30f33, 0x6d562f3e,
    0x506b692d, 0x2ede4920, 0x06589de6, 0x78edbdeb, 0x5755346a, 0x29e01467, 0x0166c0a1, 0x7fd3e0ac,
    0x5e17d3a3, 0x20a2f3ae, 0x08242768, 0x76910765, 0x59298ee4, 0x279caee9, 0x0f1a7a2f, 0x71af5a22,
    0x7560f609, 0x0bd5d604, 0x235302c2, 0x5de622cf, 0x725eab4e, 0x0ceb8b43, 0x246d5f85, 0x5ad87f88,
    0x7b1c4c87, 0x05a96c8a, 0x2d2fb84c, 0x539a9841, 0x7c2211c0, 0x029731cd, 0x2a11e50b, 0x54a4c506,
    0x69998315, 0x172ca318, 0x3faa77de, 0x411f57d3, 0x6ea7de52, 0x1012fe5f, 0x38942a99, 0x46210a94,
    0x67e5399b, 0x19501996, 0x31d6cd50, 0x4f63ed5d, 0x60db64dc, 0x1e6e44d1, 0x36e89017, 0x485db01a,
    0x3f77c841, 0x41c2e84c, 0x69443c8a, 0x17f11c87, 0x38499506, 0x46fcb50b, 0x6e7a61cd, 0x10cf41c0,
    0x310b72cf, 0x4fbe52c2, 0x67388604, 0x198da609, 0x36352f88, 0x48800f85, 0x6006db43, 0x1eb3fb4e,
    0x238ebd5d, 0x5d3b9d50, 0x75bd4996, 0x0b08699b, 0x24b0e01a, 0x5a05c017, 0x728314d1, 0x0c3634dc,
    0x2df207d3, 0x534727de, 0x7bc1f318, 0x0574d315, 0x2acc5a94, 0x54797a99, 0x7cffae5f, 0x024a8e52,
    0x06852279, 0x78300274, 0x50b6d6b2, 0x2e03f6bf, 0x01bb7f3e, 0x7f0e5f33, 0x57888bf5, 0x293dabf8,
    0x08f998f7, 0x764cb8fa, 0x5eca6c3c, 0x207f4c31, 0x0fc7c5b0, 0x7172e5bd, 0x59f4317b, 0x27411176,
    0x1a7c5765, 0x64c97768, 0x4c4fa3ae, 0x32fa83a3, 0x1d420a22, 0x63f72a2f, 0x4b71fee9, 0x35c4dee4,
    0x1400edeb, 0x6ab5cde6, 0x42331920, 0x3c86392d, 0x133eb0ac, 0x6d8b90a1, 0x450d4467, 0x3bb8646a,
];

/// [`T`] padded to 512 entries, the range of `val >> RABIN_SHIFT` for a
/// `u32`, so that `t_of` needs neither a bounds check nor a mask on the
/// fingerprint's dependency chain. `val` stays below 2^31 (each `T[i]` has
/// bit 31 equal to bit 0 of `i`, which cancels the bit shifted into
/// position 31), so the padding is never read.
static T512: [u32; 512] = {
    let mut t = [0u32; 512];
    let mut i = 0;
    while i < 256 {
        t[i] = T[i];
        i += 1;
    }
    t
};

/// `T[val >> RABIN_SHIFT]`.
#[inline(always)]
fn t_of(val: u32) -> u32 {
    T512[(val >> RABIN_SHIFT) as usize]
}

/// One step of the Rabin fingerprint: shift in `byte`.
#[inline(always)]
fn rabin_push(val: u32, byte: u8) -> u32 {
    ((val << 8) | u32::from(byte)) ^ t_of(val)
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

/// `hdr_sz`: one varint size of the delta header, from `delta[*pos..]`.
#[inline(always)]
fn hdr_sz(delta: &[u8], pos: &mut usize) -> Result<usize> {
    // The one-byte case, without the loop.
    if let Some(&c) = delta.get(*pos)
        && c & 0x80 == 0
    {
        *pos += 1;
        return Ok(c as usize);
    }
    hdr_sz_slow(delta, pos)
}

/// [`hdr_sz`]'s loop.
#[inline(always)]
fn hdr_sz_slow(delta: &[u8], pos: &mut usize) -> Result<usize> {
    let mut p = *pos;
    let mut r: usize = 0;
    let mut shift: u32 = 0;
    loop {
        let Some(&c) = delta.get(p) else {
            return Err(invalid("truncated delta"));
        };
        if shift >= usize::BITS {
            return Err(invalid("delta header overflow"));
        }
        p += 1;
        // Bits shifted out at shift 63 are lost, as in C.
        r |= ((c & 0x7f) as usize) << shift;
        shift += 7;
        if c & 0x80 == 0 {
            break;
        }
    }
    *pos = p;
    Ok(r)
}

/// `git_delta_read_header`: the base and result sizes a delta declares.
pub fn read_header(delta: &[u8]) -> Result<(usize, usize)> {
    let mut pos = 0;
    let base = hdr_sz(delta, &mut pos)?;
    let result = hdr_sz(delta, &mut pos)?;
    Ok((base, result))
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Set `msg` (class `GIT_ERROR_INVALID`) and return -1, off the hot path.
#[cold]
#[inline(never)]
fn invalid(msg: &'static str) -> ErrorCode {
    errors::set_str(ErrorClass::INVALID, msg)
}

#[cold]
#[inline(never)]
fn apply_failed() -> ErrorCode {
    invalid("failed to apply delta")
}

/// `git_delta_apply`'s checks of the header: the result size, and where
/// the instructions start. `max_object_size` is
/// `git_indexer__max_object_size`.
#[inline(always)]
pub(crate) fn apply_header(
    base_len: usize,
    delta: &[u8],
    max_object_size: usize,
) -> Result<(usize, usize)> {
    let mut pos = 0;
    // Check that the base size matches the data we were given.
    match hdr_sz(delta, &mut pos) {
        Ok(base_sz) if base_sz == base_len => {}
        _ => {
            return Err(invalid(
                "failed to apply delta: base size does not match given data",
            ));
        }
    }
    let Ok(res_sz) = hdr_sz(delta, &mut pos) else {
        return Err(invalid(
            "failed to apply delta: base size does not match given data",
        ));
    };
    if res_sz > max_object_size {
        return Err(invalid("failed to apply delta: overly large object"));
    }
    Ok((res_sz, pos))
}

/// `git_delta_apply`'s loop: run the instructions of `delta[pos..]` on
/// `base` into `out`, which is the result's size; on success every byte of
/// `out` was written (an `Ok` is only returned when the writes, which are
/// contiguous from 0, reached `out.len()`).
///
/// Callers rely on that postcondition to treat `out` as initialized after
/// an `Ok`: they call `set_len`/`assume_init` right after this call, and
/// nowhere else.
#[inline(always)]
pub(crate) fn apply_body(
    base: &[u8],
    delta: &[u8],
    mut pos: usize,
    out: &mut [MaybeUninit<u8>],
) -> Result<()> {
    let end = delta.len();
    let mut o: usize = 0;
    while pos < end {
        let cmd = delta[pos];
        pos += 1;
        if cmd & 0x80 != 0 {
            // A copy from the base.
            let mut off: usize = 0;
            let mut len: usize = 0;
            macro_rules! add {
                ($o:ident, $shift:expr) => {
                    match delta.get(pos) {
                        Some(&b) => {
                            $o |= (b as usize) << $shift;
                            pos += 1;
                        }
                        None => return Err(apply_failed()),
                    }
                };
            }
            if cmd & 0x01 != 0 {
                add!(off, 0);
            }
            if cmd & 0x02 != 0 {
                add!(off, 8);
            }
            if cmd & 0x04 != 0 {
                add!(off, 16);
            }
            if cmd & 0x08 != 0 {
                add!(off, 24);
            }
            if cmd & 0x10 != 0 {
                add!(len, 0);
            }
            if cmd & 0x20 != 0 {
                add!(len, 8);
            }
            if cmd & 0x40 != 0 {
                add!(len, 16);
            }
            if len == 0 {
                len = 0x10000;
            }
            // `off + len` cannot overflow: both are below 2^32. `o + len`
            // cannot either: `o <= out.len() <= isize::MAX`.
            let (Some(src), Some(dst)) = (base.get(off..off + len), out.get_mut(o..o + len)) else {
                return Err(apply_failed());
            };
            dst.write_copy_of_slice(src);
            o += len;
        } else if cmd != 0 {
            // A literal insert from the delta itself.
            let n = cmd as usize;
            let (Some(src), Some(dst)) = (delta.get(pos..pos + n), out.get_mut(o..o + n)) else {
                return Err(apply_failed());
            };
            dst.write_copy_of_slice(src);
            pos += n;
            o += n;
        } else {
            // cmd == 0 is reserved for future encodings.
            return Err(apply_failed());
        }
    }
    if o != out.len() {
        return Err(apply_failed());
    }
    Ok(())
}

/// Apply `delta` to `base` with the result-size limit `max_object_size`.
pub fn apply_with_limit(base: &[u8], delta: &[u8], max_object_size: usize) -> Result<Vec<u8>> {
    let (res_sz, pos) = apply_header(base.len(), delta, max_object_size)?;
    let mut out: Vec<u8> = Vec::new();
    out.try_reserve_exact(alloc_add(res_sz, 1)?)
        .map_err(|_| errors::oom())?;
    apply_body(base, delta, pos, &mut out.spare_capacity_mut()[..res_sz])?;
    // SAFETY: `apply_body` succeeded, so it wrote all `res_sz` bytes of the
    // spare capacity, which the reservation made at least that long.
    unsafe { out.set_len(res_sz) };
    Ok(out)
}

/// Apply `delta` to `base` (`git_delta_apply`): the result, or the C's
/// error (`failed to apply delta…`, class `GIT_ERROR_INVALID`). Results
/// larger than `GIT_OPT_SET_MAX_OBJECT_SIZE` are refused, as in C.
pub fn apply(base: &[u8], delta: &[u8]) -> Result<Vec<u8>> {
    apply_with_limit(base, delta, crate::sys::indexer::max_object_size())
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// No entry (C's NULL `next`).
const NONE: u32 = u32::MAX;

/// `struct index_entry`: a Rabin window ending at `ptr` in the source.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Offset of the byte after the window in the source (C's `ptr`).
    ptr: usize,
    /// The window's fingerprint.
    val: u32,
    /// The next entry of the bucket, or [`NONE`].
    next: u32,
}

/// The head of an index's allocation (`git_delta_index` as C sees it: a
/// pointer it frees with `git__free`, as `pack-objects.c` does, or with
/// `git_delta_index_free`). The allocation is one `git__allocator` block:
/// this header, the `hash_mask + 1` bucket heads (`u32`), then the `entries`
/// entries.
#[repr(C)]
#[derive(Debug)]
pub struct IndexHeader {
    /// The size of the C's index (what `git_delta_index_size` reports).
    memsize: usize,
    /// The source buffer.
    src_buf: *const u8,
    /// Its length.
    src_size: usize,
    /// The bucket count minus one (a power of two minus one).
    hash_mask: u32,
    /// The entries kept.
    entries: u32,
}

/// Offset of the bucket heads in an index's block.
const HASH_OFFSET: usize = core::mem::size_of::<IndexHeader>();

/// `git_delta_index`: the Rabin fingerprints of a source buffer, from which
/// deltas against it are made. Borrows the source.
///
/// Its invariant (made by [`new`](Self::new), the only constructor but the
/// export's `from_raw`): `block` is a live `git__allocator` block this index
/// owns, laid out as [`IndexHeader`] says, fully initialized, never written
/// again; `src_buf`/`src_size` are the source, borrowed for `'a`; every
/// bucket head and `next` is [`NONE`] or below `entries`; every entry's
/// `ptr` is below `src_size`.
pub struct DeltaIndex<'a> {
    block: NonNull<IndexHeader>,
    _src: PhantomData<&'a [u8]>,
}

// SAFETY: the block is owned and immutable once built; the source is a
// shared borrow of bytes (`&[u8]` is `Send` and `Sync`).
unsafe impl Send for DeltaIndex<'_> {}
// SAFETY: as above: shared access only reads.
unsafe impl Sync for DeltaIndex<'_> {}

impl core::fmt::Debug for DeltaIndex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeltaIndex")
            .field("memsize", &self.memsize())
            .field("source_len", &self.source().len())
            .field("buckets", &self.hash().len())
            .field("entries", &self.entries().len())
            .finish()
    }
}

impl Drop for DeltaIndex<'_> {
    fn drop(&mut self) {
        // SAFETY: the block is this index's own `git__allocator` block (the
        // invariant), freed once, here.
        unsafe { alloc::free(self.block.as_ptr().cast()) };
    }
}

fn try_vec<T: Clone>(n: usize, v: T) -> Result<Vec<T>> {
    let mut x = Vec::new();
    x.try_reserve_exact(n).map_err(|_| errors::oom())?;
    x.resize(n, v);
    Ok(x)
}

impl<'a> DeltaIndex<'a> {
    /// `git_delta_index_init`: index `src`. `None` for an empty source (C's
    /// NULL index).
    pub fn new(src: &'a [u8]) -> Result<Option<DeltaIndex<'a>>> {
        let bufsize = src.len();
        if bufsize == 0 {
            return Ok(None);
        }

        // Determine index hash size. Indexing skips the first byte to allow
        // for optimizing the Rabin polynomial initialization in `create`.
        let mut entries: u32 = ((bufsize - 1) as u32) / RABIN_WINDOW as u32;
        if bufsize as u64 >= 0xffff_ffff {
            // The format cannot encode offsets above 32 bits.
            entries = 0xffff_fffe / RABIN_WINDOW as u32;
        }
        let mut hsize = entries / 4;
        let mut i = 4;
        while i < 31 && (1u32 << i) < hsize {
            i += 1;
        }
        hsize = 1 << i;
        let hsize = hsize as usize;
        let cap = entries as usize;

        // `lookup_index_alloc`'s size, as the C allocated it.
        let entries_len = alloc_multiply(cap, C_INDEX_ENTRY_SIZE)?;
        let hash_len = alloc_multiply(hsize, C_POINTER_SIZE)?;
        let memsize = alloc_add(alloc_add(C_INDEX_HEADER_SIZE, entries_len)?, hash_len)?;

        // This port's block, no larger than the C's (4-byte heads, 16-byte
        // entries). `hsize >= 16`, so the entries start 8-aligned.
        let entries_offset = HASH_OFFSET + hsize * 4;
        let block_len = alloc_add(entries_offset, alloc_multiply(cap, size_of::<Entry>())?)?;
        if block_len > isize::MAX as usize {
            return Err(errors::oom());
        }
        let Some(block) = NonNull::new(alloc::malloc(block_len).cast::<u8>()) else {
            return Err(ErrorCode::GENERIC);
        };
        // A fresh block of `block_len` bytes, 8-aligned (what
        // `git__allocator` gives), which nothing else references: the header
        // at 0, the heads (`u32`) at `HASH_OFFSET`, the entries (8-aligned)
        // at `entries_offset`, three disjoint ranges within it.
        let hash_ptr = block
            .cast::<IndexHeader>()
            .as_ptr()
            .wrapping_add(1)
            .cast::<u32>();
        #[expect(clippy::cast_ptr_alignment, reason = "8-aligned: see above")]
        let slots_ptr = hash_ptr.wrapping_add(hsize).cast::<MaybeUninit<Entry>>();
        // SAFETY: the heads' range of the block (above); writing initializes
        // them (to NONE).
        unsafe { core::ptr::write_bytes(hash_ptr, 0xff, hsize) };
        // SAFETY: the heads, initialized just above, referenced only here.
        let hash = unsafe { core::slice::from_raw_parts_mut(hash_ptr, hsize) };
        // SAFETY: the entries' range of the block, referenced only here;
        // `MaybeUninit` needs no initialization.
        let slots = unsafe { core::slice::from_raw_parts_mut(slots_ptr, cap) };
        let index = DeltaIndex {
            block: block.cast(),
            _src: PhantomData,
        };
        // Until the header is written the drop only frees the block.
        // SAFETY: the header's place in the fresh block (see above).
        unsafe {
            block.cast::<IndexHeader>().write(IndexHeader {
                memsize,
                src_buf: src.as_ptr(),
                src_size: src.len(),
                hash_mask: (hsize - 1) as u32,
                entries: 0,
            })
        };

        let mut hash_count = try_vec(hsize, 0u32)?;
        // Entries made: `slots[..n - 1]` are written; the last one is
        // `last`, which the next window may still move (C's `entry[-1]`),
        // written to `slots[n - 1]` when the next entry is made.
        let mut n: usize = 0;
        let mut last = Entry {
            ptr: 0,
            val: 0,
            next: NONE,
        };

        // Populate the index, from the last window back: window `k` is
        // `src[16k + 1..=16k + 16]`, and its entry points after it.
        let mut prev_val = !0u32;
        let windows = &src[1..1 + cap * RABIN_WINDOW];
        for (k, w) in windows
            .as_chunks::<RABIN_WINDOW>()
            .0
            .iter()
            .enumerate()
            .rev()
        {
            let ptr = (k + 1) * RABIN_WINDOW;
            let mut val = 0u32;
            for &b in w {
                val = rabin_push(val, b);
            }
            if val == prev_val {
                // Keep the lowest of consecutive identical blocks (an
                // entry exists: `prev_val` is a window's value, and every
                // window value is below 2^31, never the initial `!0`).
                last.ptr = ptr;
            } else {
                prev_val = val;
                if n > 0 {
                    slots[n - 1].write(last);
                }
                // `hash.len() == hsize`, a power of two: the bucket
                // `val & (hsize - 1)`, with a mask the compiler sees is in
                // bounds.
                let b = val as usize & (hash.len() - 1);
                last = Entry {
                    ptr,
                    val,
                    next: hash[b],
                };
                hash[b] = n as u32;
                n += 1;
                if let Some(c) = hash_count.get_mut(b) {
                    *c += 1;
                }
            }
        }
        if n > 0 {
            slots[n - 1].write(last);
        }
        // SAFETY: one entry at most per window, so `n <= cap`; the loop
        // wrote `slots[..n]` (each slot when the entry after it was made,
        // the last one just above). `slots` is not used again.
        let ents =
            unsafe { core::slice::from_raw_parts_mut(slots.as_mut_ptr().cast::<Entry>(), n) };

        // Cull over-full buckets uniformly (C's loop, quirks included: the
        // skip count is 0 for 64..127 entries, so `--skip` runs to the end
        // of the list and only the first entry is kept).
        for (b, &count) in hash_count.iter().enumerate() {
            if count < HASH_LIMIT {
                continue;
            }
            #[cfg(test)]
            CULLED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let mut entry = hash[b];
            loop {
                let keep = entry;
                let mut skip: i32 = (count / HASH_LIMIT / 2) as i32;
                loop {
                    entry = ents[entry as usize].next;
                    skip = skip.wrapping_sub(1);
                    if skip == 0 || entry == NONE {
                        break;
                    }
                }
                ents[keep as usize].next = entry;
                if entry == NONE {
                    break;
                }
            }
        }

        // SAFETY: the header, written above; `hash`/`ents` are not used
        // again, so the block is immutable from here on.
        unsafe { (*index.block.as_ptr()).entries = n as u32 };
        Ok(Some(index))
    }

    fn header(&self) -> &IndexHeader {
        // SAFETY: the invariant: a live, initialized header never written
        // again.
        unsafe { self.block.as_ref() }
    }

    /// The bucket heads' address (after the header).
    fn hash_ptr(&self) -> *const u32 {
        self.block.as_ptr().wrapping_add(1).cast::<u32>()
    }

    /// The bucket heads.
    fn hash(&self) -> &[u32] {
        let n = self.header().hash_mask as usize + 1;
        // SAFETY: the invariant: `hash_mask + 1` initialized heads after the
        // header of the live, immutable block.
        unsafe { core::slice::from_raw_parts(self.hash_ptr(), n) }
    }

    /// The entries.
    fn entries(&self) -> &[Entry] {
        let h = self.header();
        #[expect(clippy::cast_ptr_alignment, reason = "8-aligned: the invariant")]
        let p = self
            .hash_ptr()
            .wrapping_add(h.hash_mask as usize + 1)
            .cast::<Entry>();
        // SAFETY: the invariant: `entries` initialized entries after the
        // heads (8-aligned) of the live, immutable block.
        unsafe { core::slice::from_raw_parts(p, h.entries as usize) }
    }

    /// `git_delta_index_size`: the bytes the C's index occupied.
    pub fn memsize(&self) -> usize {
        self.header().memsize
    }

    /// The indexed source.
    pub fn source(&self) -> &'a [u8] {
        let h = self.header();
        // SAFETY: the invariant: the source, borrowed for `'a`.
        unsafe { core::slice::from_raw_parts(h.src_buf, h.src_size) }
    }

    /// `git_delta_create_from_index`: a delta from the source to `target`,
    /// `None` for an empty target. With a non-zero `max_size`, a delta
    /// larger than it fails with [`ErrorCode::BUFS`].
    pub fn create(&self, target: &[u8], max_size: usize) -> Result<Option<Vec<u8>>> {
        self.create_in(target, max_size, std::alloc::Global)
    }

    /// [`create`](Self::create) into a buffer of `alloc`.
    pub(crate) fn create_in<A: Allocator>(
        &self,
        trg: &[u8],
        max_size: usize,
        alloc: A,
    ) -> Result<Option<Vec<u8, A>>> {
        if trg.is_empty() {
            return Ok(None);
        }
        let src = self.source();
        let (hash, entries) = (self.hash(), self.entries());
        if src.len() > u32::MAX as usize
            || trg.len() > u32::MAX as usize
            || max_size > (u32::MAX as usize - MAX_OP_SIZE - 1)
        {
            return Err(git_error_set!(
                ErrorClass::INVALID,
                "buffer sizes too large for delta processing"
            ));
        }

        let mut bufsize: usize = 8192;
        if max_size != 0 && bufsize >= max_size {
            bufsize = max_size + MAX_OP_SIZE + 1;
        }
        let mut buf: Vec<u8, A> = Vec::new_in(alloc);
        buf.try_reserve_exact(bufsize).map_err(|_| errors::oom())?;
        buf.resize(bufsize, 0);
        let mut bufpos: usize = 0;

        // The reference and target sizes.
        for size in [src.len() as u32, trg.len() as u32] {
            let mut i = size;
            while i >= 0x80 {
                buf[bufpos] = (i | 0x80) as u8;
                bufpos += 1;
                i >>= 7;
            }
            buf[bufpos] = i as u8;
            bufpos += 1;
        }

        let top = trg.len();
        let mut d: usize = 0;
        bufpos += 1;
        let mut val: u32 = 0;
        let mut i = 0;
        while i < RABIN_WINDOW && d < top {
            buf[bufpos] = trg[d];
            bufpos += 1;
            val = rabin_push(val, trg[d]);
            i += 1;
            d += 1;
        }
        let mut inscnt: isize = i as isize;

        let mut moff: u32 = 0;
        let mut msize: u32 = 0;
        while d < top {
            if msize < 4096 {
                val ^= U[trg[d - RABIN_WINDOW] as usize];
                val = rabin_push(val, trg[d]);
                // `hash.len() == hash_mask + 1`, a power of two.
                let mut e = hash[val as usize & (hash.len() - 1)];
                while e != NONE {
                    let entry = entries[e as usize];
                    e = entry.next;
                    if entry.val != val {
                        continue;
                    }
                    let ref_size = (src.len() - entry.ptr).min(top - d) as u32;
                    if ref_size <= msize {
                        break;
                    }
                    let n = ref_size as usize;
                    let m = common_prefix(&src[entry.ptr..entry.ptr + n], &trg[d..d + n]) as u32;
                    if msize < m {
                        // The best match so far.
                        msize = m;
                        moff = entry.ptr as u32;
                        if msize >= 4096 {
                            break;
                        }
                    }
                }
            }

            if msize < 4 {
                if inscnt == 0 {
                    bufpos += 1;
                }
                buf[bufpos] = trg[d];
                bufpos += 1;
                d += 1;
                inscnt += 1;
                if inscnt == 0x7f {
                    buf[bufpos - inscnt as usize - 1] = inscnt as u8;
                    inscnt = 0;
                }
                msize = 0;
            } else {
                if inscnt != 0 {
                    while moff != 0 && src[moff as usize - 1] == trg[d - 1] {
                        // Match one byte back.
                        msize += 1;
                        moff -= 1;
                        d -= 1;
                        bufpos -= 1;
                        inscnt -= 1;
                        if inscnt != 0 {
                            continue;
                        }
                        bufpos -= 1; // remove the count slot
                        inscnt -= 1; // make it -1
                        break;
                    }
                    // With `inscnt == -1` this writes the slot the copy
                    // opcode overwrites next, as in C.
                    buf[(bufpos as isize - inscnt - 1) as usize] = inscnt as u8;
                    inscnt = 0;
                }

                // A copy op is limited to 64KB (pack v2).
                let left = msize.saturating_sub(0x10000);
                msize -= left;

                let op = bufpos;
                bufpos += 1;
                let mut c: u8 = 0x80;
                for (k, bit) in [(0u32, 0x01u8), (8, 0x02), (16, 0x04), (24, 0x08)] {
                    if moff & (0xff << k) != 0 {
                        buf[bufpos] = (moff >> k) as u8;
                        bufpos += 1;
                        c |= bit;
                    }
                }
                for (k, bit) in [(0u32, 0x10u8), (8, 0x20)] {
                    if msize & (0xff << k) != 0 {
                        buf[bufpos] = (msize >> k) as u8;
                        bufpos += 1;
                        c |= bit;
                    }
                }
                buf[op] = c;

                d += msize as usize;
                moff = moff.wrapping_add(msize);
                msize = left;

                if msize < 4096 {
                    val = 0;
                    for &b in &trg[d - RABIN_WINDOW..d] {
                        val = rabin_push(val, b);
                    }
                }
            }

            if bufpos >= bufsize - MAX_OP_SIZE {
                // `bufsize * 3 / 2` in `unsigned int`, as C.
                let mut new_size = ((bufsize as u32).wrapping_mul(3) / 2) as usize;
                if max_size != 0 && new_size >= max_size {
                    new_size = max_size + MAX_OP_SIZE + 1;
                }
                if max_size != 0 && bufpos > max_size {
                    break;
                }
                if new_size <= bufsize {
                    // C's `unsigned int` wrapped and it would realloc to a
                    // smaller buffer (rust/DEFECTS.md): fail instead.
                    return Err(errors::oom());
                }
                buf.try_reserve_exact(new_size - bufsize)
                    .map_err(|_| errors::oom())?;
                buf.resize(new_size, 0);
                bufsize = new_size;
            }
        }

        if inscnt != 0 {
            buf[bufpos - inscnt as usize - 1] = inscnt as u8;
        }

        if max_size != 0 && bufpos > max_size {
            let _ = git_error_set!(
                ErrorClass::NOMEMORY,
                "delta would be larger than maximum size"
            );
            return Err(ErrorCode::BUFS);
        }

        buf.truncate(bufpos);
        Ok(Some(buf))
    }
}

/// The length of the common prefix of two equally long slices (C's
/// byte-by-byte `while (ref_size-- && *src++ == *ref) ref++;`).
#[inline]
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + 8 <= n {
        let x = u64::from_le_bytes(a[i..i + 8].try_into().unwrap_or([0; 8]));
        let y = u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]));
        let diff = x ^ y;
        if diff != 0 {
            return i + (diff.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < n && a[i] == b[i] {
        i += 1;
    }
    i
}

/// `git_delta`: a delta from `src` to `trg` (`None` when either is empty,
/// as the C inline returns a NULL buffer); with a non-zero `max_size`, a
/// delta larger than it fails with [`ErrorCode::BUFS`].
pub fn diff(src: &[u8], trg: &[u8], max_size: usize) -> Result<Option<Vec<u8>>> {
    match DeltaIndex::new(src)? {
        Some(index) => index.create(trg, max_size),
        None => Ok(None),
    }
}

/// `git_delta_read_header_fromstream`'s parse, over the bytes read.
fn read_header_from(read: &mut dyn FnMut(&mut [u8]) -> isize) -> Result<(usize, usize)> {
    const BUFFER_LEN: usize = 16;
    let mut buffer = [0u8; BUFFER_LEN];
    let mut len = 0;
    while len < BUFFER_LEN {
        let r = read(&mut buffer[len..]);
        if r == 0 {
            break;
        }
        if r == ErrorCode::BUFS.raw() as isize {
            continue;
        }
        if r < 0 {
            // C added the negative status to `len` (rust/DEFECTS.md); the
            // stream has set the error.
            return Err(ErrorCode::GENERIC);
        }
        len += (r as usize).min(BUFFER_LEN - len);
    }
    read_header(&buffer[..len])
}

/// The C ABI of `delta.c`.
pub mod ffi {
    use super::*;
    use crate::git_assert_arg;
    use crate::util::alloc::{GIT_ALLOC, GitAlloc};
    use crate::util::cabi::Out;
    use crate::util::export::{barrier, bytes};
    use core::ffi::c_char;
    use core::mem::ManuallyDrop;

    /// `git_delta_index` (opaque to C): the block of a [`DeltaIndex`] over
    /// the buffer C passed, which C keeps alive and unaltered until the
    /// index is freed (with `git_delta_index_free`, or `git__free`, which
    /// `pack-objects.c` uses).
    #[allow(non_camel_case_types)]
    pub type git_delta_index = IndexHeader;

    /// The index whose block C holds, borrowed (not freed when dropped).
    ///
    /// # Safety
    ///
    /// `p` is a live index from [`git_delta_index_init`], not freed while
    /// the result is used, whose source is still allocated and unwritten.
    unsafe fn borrow_index(p: *const git_delta_index) -> ManuallyDrop<DeltaIndex<'static>> {
        ManuallyDrop::new(DeltaIndex {
            // SAFETY: the caller's contract: a non-null live block.
            block: unsafe { NonNull::new_unchecked(p.cast_mut()) },
            _src: core::marker::PhantomData,
        })
    }

    /// Hand a `git__allocator` buffer to C.
    fn into_c(v: Vec<u8, GitAlloc>) -> (*mut c_void, usize) {
        let mut v = ManuallyDrop::new(v);
        debug_assert!(v.capacity() > 0);
        (v.as_mut_ptr().cast(), v.len())
    }

    /// `git_delta_index_init`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `buf` is NULL or readable for `bufsize` bytes,
    /// which are neither freed nor written until the index is freed with
    /// [`git_delta_index_free`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_index_init(
        mut out: Out<'_, *mut git_delta_index>,
        buf: *const c_void,
        bufsize: usize,
    ) -> c_int {
        barrier(|| {
            out.write(core::ptr::null_mut());
            if buf.is_null() || bufsize == 0 {
                return 0;
            }
            // SAFETY: the caller's contract: `bufsize` readable bytes that
            // stay unwritten and allocated while the index lives (the
            // `'static` is that promise; the index is freed by C).
            let src: &'static [u8] = unsafe { bytes(buf.cast::<c_char>(), bufsize) };
            match DeltaIndex::new(src) {
                Ok(Some(index)) => {
                    out.write(ManuallyDrop::new(index).block.as_ptr());
                    0
                }
                Ok(None) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_delta_index_free`.
    ///
    /// # Safety
    ///
    /// `index` is NULL or an index from [`git_delta_index_init`] not freed
    /// yet (with this or `git__free`), and not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_index_free(index: *mut git_delta_index) {
        barrier(|| {
            if !index.is_null() {
                // SAFETY: the caller's contract: a live index from `init`.
                unsafe { alloc::free(index.cast()) };
            }
        })
    }

    /// `git_delta_index_size`. `SIZE_MAX` (C's `-1`) for NULL.
    ///
    /// # Safety
    ///
    /// `index` is NULL or a live index from [`git_delta_index_init`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_index_size(index: *mut git_delta_index) -> usize {
        barrier(|| {
            git_assert_arg!(!index.is_null(), "index", usize::MAX);
            // SAFETY: the caller's contract; indexes are only read.
            unsafe { borrow_index(index) }.memsize()
        })
    }

    /// `git_delta_create_from_index`.
    ///
    /// # Safety
    ///
    /// `out` and `out_size` are writable; `trg_buf` is NULL or readable
    /// for `trg_size` bytes; `index` is a live index from
    /// [`git_delta_index_init`] (it may be NULL only when the target is
    /// empty; C dereferenced it).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_create_from_index(
        out: *mut *mut c_void,
        out_size: *mut usize,
        index: *const git_delta_index,
        trg_buf: *const c_void,
        trg_size: usize,
        max_size: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes writable out-parameters.
            unsafe {
                out.write(core::ptr::null_mut());
            }
            // SAFETY: as above.
            unsafe {
                out_size.write(0);
            }
            if trg_buf.is_null() || trg_size == 0 {
                return 0;
            }
            git_assert_arg!(!index.is_null(), "index", -1);
            // SAFETY: the caller's contract: a live index, and `trg_size`
            // readable target bytes.
            let index = unsafe { borrow_index(index) };
            // SAFETY: as above.
            let trg = unsafe { bytes(trg_buf.cast::<c_char>(), trg_size) };
            match index.create_in(trg, max_size, GIT_ALLOC) {
                Ok(Some(v)) => {
                    let (p, n) = into_c(v);
                    // SAFETY: as above.
                    unsafe {
                        out.write(p);
                    }
                    // SAFETY: as above.
                    unsafe {
                        out_size.write(n);
                    }
                    0
                }
                Ok(None) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_delta_read_header`.
    ///
    /// # Safety
    ///
    /// `base_out` and `result_out` are writable; `delta` is NULL or
    /// readable for `delta_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_read_header(
        mut base_out: Out<'_, usize>,
        mut result_out: Out<'_, usize>,
        delta: *const u8,
        delta_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let delta = unsafe { bytes(delta.cast::<c_char>(), delta_len) };
            let mut pos = 0;
            // C writes the base size before reading the result's.
            let r = hdr_sz(delta, &mut pos).and_then(|b| {
                base_out.write(b);
                hdr_sz(delta, &mut pos)
            });
            match r {
                Ok(res) => {
                    result_out.write(res);
                    0
                }
                Err(_) => -1,
            }
        })
    }

    /// `git_delta_read_header_fromstream`.
    ///
    /// # Safety
    ///
    /// `base_sz` and `res_sz` are writable; `stream` is a live
    /// `git_packfile_stream`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_read_header_fromstream(
        base_sz: *mut usize,
        res_sz: *mut usize,
        stream: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let mut read = |buf: &mut [u8]| -> isize {
                // SAFETY: `stream` is live (the caller); `buf` is writable
                // for its length, and the stream writes at most that.
                unsafe {
                    crate::pack::GitPackfileStream::read(stream.cast(), buf.as_mut_ptr(), buf.len())
                }
            };
            match read_header_from(&mut read) {
                Ok((b, r)) => {
                    // SAFETY: the caller passes writable out-parameters.
                    unsafe {
                        base_sz.write(b);
                    }
                    // SAFETY: as above.
                    unsafe {
                        res_sz.write(r);
                    }
                    0
                }
                Err(_) => -1,
            }
        })
    }

    /// `git_delta_apply` into a `git__malloc` buffer of the result's size
    /// plus a NUL, as C.
    fn apply_c(base: &[u8], delta: &[u8], max: usize) -> Result<(*mut c_void, usize)> {
        let (res_sz, pos) = apply_header(base.len(), delta, max)?;
        let alloc_sz = alloc_add(res_sz, 1)?;
        if alloc_sz > isize::MAX as usize {
            return Err(errors::oom());
        }
        let p = alloc::malloc(alloc_sz);
        if p.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: `p` is a fresh allocation of `alloc_sz <= isize::MAX`
        // bytes, which nothing else references; `MaybeUninit` needs no
        // initialization.
        let buf = unsafe { core::slice::from_raw_parts_mut(p.cast::<MaybeUninit<u8>>(), alloc_sz) };
        let (body, nul) = buf.split_at_mut(res_sz);
        match apply_body(base, delta, pos, body) {
            Ok(()) => {
                nul[0].write(0);
                Ok((p, res_sz))
            }
            Err(e) => {
                // SAFETY: `p` is ours (from `malloc` above), not handed out;
                // `buf` is not used again.
                unsafe { alloc::free(p) };
                Err(e)
            }
        }
    }

    /// `git_delta_apply`.
    ///
    /// # Safety
    ///
    /// `out` and `out_len` are writable; `base` is NULL or readable for
    /// `base_len` bytes, `delta` for `delta_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_delta_apply(
        out: *mut *mut c_void,
        out_len: *mut usize,
        base: *const u8,
        base_len: usize,
        delta: *const u8,
        delta_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes writable out-parameters.
            unsafe {
                out.write(core::ptr::null_mut());
            }
            // SAFETY: as above.
            unsafe {
                out_len.write(0);
            }
            // SAFETY: the caller's contract.
            let base = unsafe { bytes(base.cast::<c_char>(), base_len) };
            // SAFETY: as above.
            let delta = unsafe { bytes(delta.cast::<c_char>(), delta_len) };
            let max = crate::sys::indexer::max_object_size();
            match apply_c(base, delta, max) {
                Ok((p, n)) => {
                    // SAFETY: as above.
                    unsafe {
                        out.write(p);
                    }
                    // SAFETY: as above.
                    unsafe {
                        out_len.write(n);
                    }
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }
}

/// Buckets culled so far (tests check the culling runs).
#[cfg(test)]
static CULLED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
mod cull_tests {
    use super::*;
    use core::sync::atomic::Ordering;

    /// A source of `per` windows of each of 17 different contents: a
    /// period of 17 bytes, so the 16-byte windows do not collapse into
    /// one entry per run, and each bucket gets `per` entries.
    fn source(per: usize) -> Vec<u8> {
        (0..17 * 16 * per + 7)
            .map(|i| b'a' + (i % 17) as u8)
            .collect()
    }

    #[test]
    fn over_full_buckets_are_culled() {
        let _g = crate::util::test_support::init();
        // 64..127 entries (skip 0: C's quirk keeps one entry), 128+
        // (skip 1, 2, …).
        for per in [
            64usize,
            100,
            127,
            128,
            200,
            if cfg!(miri) { 260 } else { 1000 },
        ] {
            let src = source(per);
            let before = CULLED.load(Ordering::Relaxed);
            let idx = DeltaIndex::new(&src).unwrap().unwrap();
            assert!(
                CULLED.load(Ordering::Relaxed) > before,
                "no culling for {per}"
            );
            let mut trg = src[5..].to_vec();
            trg.extend_from_slice(b"some new tail bytes");
            trg.extend_from_slice(&src[..300]);
            let d = idx.create(&trg, 0).unwrap().unwrap();
            assert_eq!(apply(&src, &d).unwrap(), trg);
        }
    }
}
