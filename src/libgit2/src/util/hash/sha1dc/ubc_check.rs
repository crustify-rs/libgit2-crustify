//! `src/util/hash/sha1dc/ubc_check.c`: the unavoidable-bitcondition check of SHA-1 collision
//! detection, ported to Rust.
//!
//! # Disturbance vectors
//!
//! SHA-1 collision detection (`sha1.c`) checks each message block against a table of
//! disturbance vectors, `sha1_dvs`: one [`DvInfo`] record per vector, of type I or II
//! ([`dv_type`](DvInfoRef::dv_type)), ended by a record whose type is `0`. For a record whose
//! bit `ubc_check` sets in its mask ([`mask_bit`](DvInfoRef::mask_bit)), `sha1.c` XORs the
//! expanded message block with the vector's message difference
//! ([`message_difference`](DvInfoRef::message_difference)) and recompresses it from the
//! record's step ([`recompression_step`](DvInfoRef::recompression_step)), with the state it
//! saved at that step.
//!
//! The record keeps the C layout, `dv_info_t`, while `sha1.c`'s C reads the table: its
//! `sha1_process` indexes the saved states by the step and shifts a mask by the bit, both
//! unchecked, so [`DvInfo::new`] builds only records whose step and bit are in range.

pub use self::ffi_import::{DvInfo, DvInfoMut, DvInfoRef};

#[expect(
    unsafe_code,
    reason = "reads dv_info_t's fields through its handles' pointers and lends its message \
              difference as an array, and asserts the record's `Send` and `Sync`"
)]
mod ffi_import {
    use core::fmt;
    use core::marker::PhantomData;

    use ffibox::{CRefSeam, define_ctype};

    use crate::sys::libgit2 as sys;

    /// The steps of SHA-1's compression, and of an expanded message block's words.
    const STEPS: usize = 80;

    define_ctype!(
        /// One disturbance vector of SHA-1 collision detection, as the table `sha1_dvs` lists
        /// them: its type and parameters, the expanded message block's XOR difference it
        /// defines, the step `sha1.c` recompresses from, and the bit of `ubc_check`'s mask
        /// that selects it.
        ///
        /// Plain data: six integers and 80 words, no pointer and nothing to release. C builds
        /// none and writes none: it reads the records of the table `ubc_check.c` defines, by
        /// value. A value Rust holds is built with [`new`](Self::new), which keeps the step
        /// below 80 and the mask's bit below 32, as `sha1.c` indexes and shifts by them; it
        /// is `Copy`, like the C struct. A [`DvInfoRef`] over a record C holds, which
        /// [`CRefSeam::from_ptr`] borrows, is one whose fields keep the same ranges.
        ///
        /// Wraps `st::dv_info_t`
        #[doc(alias = "dv_info_t")]
        #[derive(Clone, Copy)]
        DvInfo,
        DvInfoRef,
        DvInfoMut,
        sys::dv_info_t
    );

    // SAFETY: a `DvInfo` is six `c_int`s and 80 `u32`s: no pointer, no interior mutability,
    // no thread affinity, nothing to release, so moving one to another thread moves bytes.
    unsafe impl Send for DvInfo {}

    // SAFETY: `DvInfoMut` is an exclusive borrow of such a value, as `&mut DvInfo`.
    unsafe impl Send for DvInfoMut<'_> {}

    // SAFETY: shared access only reads the fields (no handle writes one: `DvInfoMut` has no
    // setter, and C writes no record of the table it reads), so concurrent readers race
    // nothing.
    unsafe impl Sync for DvInfo {}

    // SAFETY: as above; `DvInfoRef` is a shared borrow, as `&DvInfo`.
    unsafe impl Send for DvInfoRef<'_> {}

    // SAFETY: as above.
    unsafe impl Sync for DvInfoRef<'_> {}

    // SAFETY: `&DvInfoMut` only reaches the getters, through `as_ref`.
    unsafe impl Sync for DvInfoMut<'_> {}

    impl DvInfo {
        /// Builds the record of a disturbance vector: its type (`1` for I, `2` for II, `0`
        /// for the record that ends the table) and parameters `k` and `b`, the step to
        /// recompress from, the word and bit of `ubc_check`'s mask that select it, and the
        /// expanded message block's XOR difference it defines.
        ///
        /// Returns `None` where `sha1.c` could not read the record: a step outside `0..80`,
        /// which indexes its saved states, a bit outside `0..32`, by which it shifts the mask,
        /// or a word other than `0`, the one word of `ubc_check`'s mask (`DVMASKSIZE` is 1).
        /// The other fields take any value. `const`, so that a table of records is a
        /// `static`.
        #[must_use]
        pub const fn new(
            dv_type: i32,
            k: i32,
            b: i32,
            recompression_step: i32,
            mask_word: i32,
            mask_bit: i32,
            message_difference: [u32; STEPS],
        ) -> Option<Self> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_possible_wrap,
                reason = "`STEPS` is 80, and `u32::BITS` 32: both fit in an `i32`"
            )]
            let in_range = 0 <= recompression_step
                && recompression_step < STEPS as i32
                && mask_word == 0
                && 0 <= mask_bit
                && mask_bit < u32::BITS as i32;
            if !in_range {
                return None;
            }
            Some(Self(
                sys::dv_info_t {
                    dvType: dv_type,
                    dvK: k,
                    dvB: b,
                    testt: recompression_step,
                    maski: mask_word,
                    maskb: mask_bit,
                    dm: message_difference,
                },
                PhantomData,
            ))
        }
    }

    impl<'a> DvInfoRef<'a> {
        /// The vector's type: `1` for a type I vector, `2` for a type II, and `0` for the
        /// record that ends the table, as `sha1.c` reads it.
        ///
        /// Wraps `fld::dv_info_t.dvType`
        #[doc(alias = "dv_info_t.dvType")]
        #[must_use]
        pub fn dv_type(&self) -> i32 {
            // SAFETY: `self.as_ptr()` addresses the live record this handle borrows; the
            // projection forms no reference to it.
            let dv_type_ptr = unsafe { &raw const (*self.as_ptr()).dvType };
            // SAFETY: an initialised `c_int`, unwritten while the handle borrows it.
            unsafe { dv_type_ptr.read() }
        }

        /// The step `sha1.c` recompresses from, in `0..80`: the index of the state it saved
        /// at that step.
        ///
        /// # Panics
        ///
        /// Panics, with `dv_info_t.testt out of range`, only if a record that
        /// [`CRefSeam::from_ptr`] borrowed breaks its contract.
        ///
        /// Wraps `fld::dv_info_t.testt`
        #[doc(alias = "dv_info_t.testt")]
        #[must_use]
        pub fn recompression_step(&self) -> usize {
            // SAFETY: `self.as_ptr()` addresses the live record this handle borrows; the
            // projection forms no reference to it.
            let testt_ptr = unsafe { &raw const (*self.as_ptr()).testt };
            // SAFETY: an initialised `c_int`, unwritten while the handle borrows it.
            let testt = unsafe { testt_ptr.read() };
            usize::try_from(testt)
                .ok()
                .filter(|&step| step < STEPS)
                .expect("dv_info_t.testt out of range")
        }

        /// The bit of `ubc_check`'s one-word mask that selects this vector, in `0..32`.
        ///
        /// # Panics
        ///
        /// Panics, with `dv_info_t.maskb out of range`, only if a record that
        /// [`CRefSeam::from_ptr`] borrowed breaks its contract.
        ///
        /// Wraps `fld::dv_info_t.maskb`
        #[doc(alias = "dv_info_t.maskb")]
        #[must_use]
        pub fn mask_bit(&self) -> u32 {
            // SAFETY: `self.as_ptr()` addresses the live record this handle borrows; the
            // projection forms no reference to it.
            let maskb_ptr = unsafe { &raw const (*self.as_ptr()).maskb };
            // SAFETY: an initialised `c_int`, unwritten while the handle borrows it.
            let maskb = unsafe { maskb_ptr.read() };
            u32::try_from(maskb)
                .ok()
                .filter(|&bit| bit < u32::BITS)
                .expect("dv_info_t.maskb out of range")
        }

        /// The XOR difference the vector defines on an expanded message block: one word per
        /// step, which `sha1.c` XORs into the block before it recompresses.
        ///
        /// Wraps `fld::dv_info_t.dm`
        #[doc(alias = "dv_info_t.dm")]
        #[must_use]
        pub fn message_difference(&self) -> &'a [u32; STEPS] {
            // SAFETY: `self.as_ptr()` addresses the live record this handle borrows; the
            // projection forms no reference to it.
            let dm_ptr = unsafe { &raw const (*self.as_ptr()).dm };
            // SAFETY: `dm_ptr` addresses the record's 80 initialised, aligned `u32`s, which
            // live as long as the record this handle borrows for `'a`; nothing writes them
            // while it does: `DvInfoMut` has no setter, a shared handle taken from one
            // borrows it, and C writes no record of the table it reads.
            unsafe { &*dm_ptr }
        }

        /// A copy of the record, which outlives the borrow.
        #[must_use]
        pub fn copied(&self) -> DvInfo {
            // SAFETY: `self.as_ptr()` addresses the live, initialised record this handle
            // borrows, unwritten while it does; it is plain data, so a bitwise copy is a
            // value of its own.
            DvInfo(unsafe { self.as_ptr().read() }, PhantomData)
        }
    }

    impl PartialEq for DvInfo {
        /// Every field equal, those [`DvInfoRef`]'s getters leave out included.
        fn eq(&self, other: &Self) -> bool {
            let (a, b) = (&self.0, &other.0);
            (a.dvType, a.dvK, a.dvB, a.testt, a.maski, a.maskb, a.dm)
                == (b.dvType, b.dvK, b.dvB, b.testt, b.maski, b.maskb, b.dm)
        }
    }

    impl Eq for DvInfo {}

    impl PartialEq for DvInfoRef<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.copied() == other.copied()
        }
    }

    impl Eq for DvInfoRef<'_> {}

    impl fmt::Debug for DvInfo {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let r = &self.0;
            f.debug_struct("DvInfo")
                .field("dv_type", &r.dvType)
                .field("k", &r.dvK)
                .field("b", &r.dvB)
                .field("recompression_step", &r.testt)
                .field("mask_word", &r.maski)
                .field("mask_bit", &r.maskb)
                .field("message_difference", &r.dm)
                .finish()
        }
    }

    impl fmt::Debug for DvInfoRef<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.copied().fmt(f)
        }
    }

    impl fmt::Debug for DvInfoMut<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }
}
