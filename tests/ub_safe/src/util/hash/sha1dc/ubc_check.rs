//! `src/util/hash/sha1dc/ubc_check.c`: [`DvInfo`], the record of one disturbance vector,
//! through its safe API alone.
//!
//! A record holds plain data, which Rust values are built, copied and read through without C:
//! its handles reborrow, and the message difference a shared handle lends is read beside the
//! other handles that share it (Miri, `BSan`); records, their shared handles and an exclusive
//! one cross threads, and a `static` table and a shared value are read from several at once
//! (`TSan`, for its `Send` and `Sync`). The lent difference outliving its record, and a shared
//! handle beside the exclusive one, are in `compile_fail/`. The port's table, [`SHA1_DVS`], is
//! read from several threads at once, each lending its records' differences to [`ubc_check`]
//! as blocks, beside a thread that checks blocks of its own (`TSan`, Miri). None reaches C, so
//! Miri runs them all.

use std::sync::Barrier;
use std::thread;

use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::{
    DvInfo, DvInfoMut, DvInfoRef, SHA1_DVS, ubc_check,
};

/// A record of the given type, step and bit, whose message difference counts up from `tag`.
const fn record(dv_type: i32, step: i32, bit: i32, tag: u32) -> DvInfo {
    let mut dm = [0; 80];
    let (mut j, mut word) = (0, tag);
    while j < 80 {
        dm[j] = word;
        j += 1;
        word += 1;
    }
    match DvInfo::new(dv_type, 43, 0, step, 0, bit, dm) {
        Some(r) => r,
        None => panic!("out of range"),
    }
}

/// A table as `ubc_check.c`'s port builds `sha1_dvs`: a `static`, which needs `Sync`.
static TABLE: [DvInfo; 3] = [
    record(1, 58, 3, 0x300),
    record(2, 65, 31, 0x1F00),
    record(0, 0, 0, 0),
];

/// Every field the getters reach, folded into one value.
fn read_all(r: DvInfoRef<'_>) -> u64 {
    let dm = r.message_difference();
    let sum: u64 = dm.iter().map(|&w| u64::from(w)).sum();
    sum + u64::from(r.mask_bit())
        + u64::try_from(r.recompression_step()).unwrap()
        + u64::try_from(r.dv_type()).unwrap()
}

fn read_through(m: &DvInfoMut<'_>) -> u64 {
    read_all(m.as_ref())
}

#[test]
fn handles_reborrow_and_lend_the_difference_beside_each_other() {
    let mut r = record(1, 58, 7, 0x700);
    let expected = read_all(r.as_ref());
    let mut m = r.as_mut();
    assert_eq!(read_through(&m.as_mut()), expected);
    let shared = m.as_ref();
    let again = shared;
    let dm = shared.message_difference();
    assert_eq!(read_all(again), expected);
    assert_eq!(dm[79], 0x700 + 79);
    assert_eq!(read_through(&m), expected);
    let first = r.as_ref().message_difference();
    let second = r.as_ref().message_difference();
    assert_eq!(first, second);
    // A copy outlives the record it was taken from.
    let copy = {
        let inner = record(1, 58, 7, 0x700);
        inner.as_ref().copied()
    };
    assert_eq!(read_all(copy.as_ref()), expected);
}

#[test]
fn records_cross_threads_and_shared_ones_are_read_concurrently() {
    let local = record(2, 65, 12, 0xC00);
    let expected_local = read_all(local.as_ref());
    let expected_table: Vec<u64> = TABLE.iter().map(|r| read_all(r.as_ref())).collect();
    let shared = local.as_ref();
    let barrier = Barrier::new(4);
    thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                barrier.wait();
                for _ in 0..50 {
                    assert_eq!(read_all(shared), expected_local);
                    for (r, &e) in TABLE.iter().zip(&expected_table) {
                        assert_eq!(read_all(r.as_ref()), e);
                    }
                }
            });
        }
        s.spawn(|| {
            barrier.wait();
            for _ in 0..50 {
                assert_eq!(read_all(local.as_ref()), expected_local);
                assert_eq!(TABLE[1].as_ref().message_difference()[0], 0x1F00);
            }
        });
        barrier.wait();
        assert_eq!(read_all(TABLE[0].as_ref()), expected_table[0]);
    });
    let mut moved = local;
    let handle = thread::spawn(move || {
        let r = moved;
        read_all(r.as_ref())
    });
    assert_eq!(handle.join().unwrap(), expected_local);
    let exclusive = moved.as_mut();
    let from_thread = thread::scope(|s| s.spawn(move || read_through(&exclusive)).join().unwrap());
    assert_eq!(from_thread, expected_local);
}

/// The masks of the table's differences checked as blocks, one per record.
fn masks_of_the_table() -> Vec<u32> {
    SHA1_DVS
        .iter()
        .map(|r| ubc_check(r.as_ref().message_difference())[0])
        .collect()
}

#[test]
fn the_ported_table_is_read_and_lent_from_threads_at_once() {
    let expected = masks_of_the_table();
    let expected_reads: Vec<u64> = SHA1_DVS.iter().map(|r| read_all(r.as_ref())).collect();
    let barrier = Barrier::new(4);
    thread::scope(|s| {
        for _ in 0..3 {
            s.spawn(|| {
                barrier.wait();
                for _ in 0..4 {
                    assert_eq!(masks_of_the_table(), expected);
                    for (r, &e) in SHA1_DVS.iter().zip(&expected_reads) {
                        assert_eq!(read_all(r.as_ref()), e);
                    }
                }
            });
        }
        barrier.wait();
        let mut w = [0_u32; 80];
        for round in 0..64_u32 {
            w[35 + usize::try_from(round % 30).unwrap()] ^= round.wrapping_mul(0x9E37_79B9);
            let [mask] = ubc_check(&w);
            assert_eq!(ubc_check(&w), [mask]);
        }
    });
    // A difference borrowed from the table lives as long as the program.
    let lent: &'static [u32; 80] = SHA1_DVS[31].as_ref().message_difference();
    let from_thread = thread::spawn(move || ubc_check(lent)).join().unwrap();
    assert_eq!(from_thread[0], expected[31]);
}
