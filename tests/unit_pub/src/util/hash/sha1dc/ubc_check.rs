//! `src/util/hash/sha1dc/ubc_check.c`: [`DvInfo`], the record of one disturbance vector,
//! checked against its contract: what [`DvInfo::new`] keeps and refuses, its getters, copies,
//! equality and `Debug`, and a table of records built as a `static`, read as `sha1.c` reads
//! `sha1_dvs`.
//!
//! The module has no C test (`modules.json`'s `tests` is empty). Its data are the first and
//! last records of `ubc_check.c`'s `sha1_dvs`. None reaches C, so Miri runs them all.

use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::{DvInfo, DvInfoRef};

/// The message difference of `sha1_dvs`'s first record, `I(43,0)`.
const DM_I_43_0: [u32; 80] = [
    0x0800_0000,
    0x9800_000C,
    0xD800_0010,
    0x0800_0010,
    0xB800_0010,
    0x9800_0000,
    0x6000_0000,
    0x0000_0008,
    0xC000_0000,
    0x9000_0014,
    0x1000_0010,
    0xB800_0014,
    0x2800_0000,
    0x2000_0010,
    0x4800_0000,
    0x0800_0018,
    0x6000_0000,
    0x9000_0010,
    0xF000_0010,
    0x9000_0008,
    0xC000_0000,
    0x9000_0010,
    0xF000_0010,
    0xB000_0008,
    0x4000_0000,
    0x9000_0000,
    0xF000_0010,
    0x9000_0018,
    0x6000_0000,
    0x9000_0010,
    0x9000_0010,
    0x9000_0000,
    0x8000_0000,
    0x0000_0010,
    0xA000_0000,
    0x2000_0000,
    0xA000_0000,
    0x2000_0010,
    0x0000_0000,
    0x2000_0010,
    0x2000_0000,
    0x0000_0010,
    0x2000_0000,
    0x0000_0010,
    0xA000_0000,
    0x0000_0000,
    0x2000_0000,
    0x2000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0001,
    0x0000_0020,
    0x0000_0001,
    0x4000_0002,
    0x4000_0040,
    0x4000_0002,
    0x8000_0004,
    0x8000_0080,
    0x8000_0006,
    0x0000_0049,
    0x0000_0103,
    0x8000_0009,
    0x8000_0012,
    0x8000_0202,
    0x0000_0018,
    0x0000_0164,
    0x0000_0408,
    0x8000_00E6,
    0x8000_004C,
    0x0000_0803,
    0x8000_0161,
    0x8000_0599,
];

/// `sha1_dvs`'s first record: `{1,43,0,58,0,0, {...}}`.
fn i_43_0() -> DvInfo {
    DvInfo::new(1, 43, 0, 58, 0, 0, DM_I_43_0).expect("the C table's first record is in range")
}

/// `sha1_dvs`'s last record, which ends the table: every field zero.
fn terminator() -> DvInfo {
    DvInfo::new(0, 0, 0, 0, 0, 0, [0; 80]).expect("the C table's terminator is in range")
}

/// A record of the given step and bit, its other fields those of `I(43,0)`.
fn with(step: i32, word: i32, bit: i32) -> Option<DvInfo> {
    DvInfo::new(1, 43, 0, step, word, bit, DM_I_43_0)
}

const fn record(dv_type: i32, step: i32, bit: i32, first_word: u32) -> DvInfo {
    let mut dm = [0; 80];
    dm[0] = first_word;
    match DvInfo::new(dv_type, 43, 0, step, 0, bit, dm) {
        Some(r) => r,
        None => panic!("out of range"),
    }
}

/// A table built at compile time, as `ubc_check.c`'s port builds `sha1_dvs`: two vectors and
/// the terminator.
static TABLE: [DvInfo; 3] = [
    record(1, 58, 0, 0x0800_0000),
    record(2, 65, 31, 0xB400_0008),
    record(0, 0, 0, 0),
];

#[test]
fn new_keeps_every_field_of_the_c_tables_first_record() {
    let r = i_43_0();
    let h = r.as_ref();
    assert_eq!(h.dv_type(), 1);
    assert_eq!(h.recompression_step(), 58);
    assert_eq!(h.mask_bit(), 0);
    assert_eq!(h.message_difference(), &DM_I_43_0);
    let debug = format!("{r:?}");
    assert!(debug.starts_with(
        "DvInfo { dv_type: 1, k: 43, b: 0, recompression_step: 58, mask_word: 0, mask_bit: 0, \
         message_difference: [134217728, 2550136844,"
    ));
}

#[test]
fn new_builds_the_terminator() {
    let r = terminator();
    let h = r.as_ref();
    assert_eq!(h.dv_type(), 0);
    assert_eq!(h.recompression_step(), 0);
    assert_eq!(h.mask_bit(), 0);
    assert_eq!(h.message_difference(), &[0; 80]);
}

/// The step indexes `sha1.c`'s 80 saved states, and the bit shifts a 32-bit mask: each bound is
/// kept on both sides, and `DVMASKSIZE`'s one word is the only one.
#[test]
fn new_refuses_a_step_bit_or_word_sha1_c_cannot_read() {
    for step in [0, 1, 58, 65, 79] {
        let r = with(step, 0, 0).expect("in range");
        assert_eq!(
            r.as_ref().recompression_step(),
            usize::try_from(step).unwrap()
        );
    }
    for step in [-1, 80, 81, i32::MIN, i32::MAX] {
        assert_eq!(with(step, 0, 0), None, "step {step}");
    }
    for bit in [0, 1, 16, 31] {
        let r = with(58, 0, bit).expect("in range");
        assert_eq!(r.as_ref().mask_bit(), u32::try_from(bit).unwrap());
    }
    for bit in [-1, 32, 33, i32::MIN, i32::MAX] {
        assert_eq!(with(58, 0, bit), None, "bit {bit}");
    }
    for word in [-1, 1, 2, i32::MIN, i32::MAX] {
        assert_eq!(with(58, word, 0), None, "word {word}");
    }
}

/// The type and the parameters `k` and `b` take any value: `sha1.c` only compares the type
/// with `0`, and reads neither parameter.
#[test]
fn new_takes_any_type_and_parameters() {
    for v in [i32::MIN, -1, 0, 1, 2, 3, i32::MAX] {
        let r = DvInfo::new(v, v, v, 65, 0, 31, [u32::MAX; 80]).expect("in range");
        assert_eq!(r.as_ref().dv_type(), v);
        assert!(format!("{r:?}").contains(&format!("k: {v}, b: {v},")));
        assert_eq!(r.as_ref().message_difference(), &[u32::MAX; 80]);
    }
}

/// Read as `sha1_process` reads `sha1_dvs`: up to the record whose type is `0`, each selected
/// by its bit of the mask.
#[test]
fn a_static_table_is_read_up_to_its_terminator() {
    let mask: u32 = 0x8000_0000;
    let mut selected = Vec::new();
    let mut i = 0;
    while TABLE[i].as_ref().dv_type() != 0 {
        let r = TABLE[i].as_ref();
        if mask & (1 << r.mask_bit()) != 0 {
            selected.push((r.recompression_step(), r.message_difference()[0]));
        }
        i += 1;
    }
    assert_eq!(i, 2);
    assert_eq!(selected, [(65, 0xB400_0008)]);
}

#[test]
fn copies_and_equality_cover_every_field() {
    let r = i_43_0();
    let copy = r;
    assert_eq!(copy, r);
    assert_eq!(r.as_ref().copied(), r);
    assert_eq!(r.as_ref(), copy.as_ref());
    // Fields no getter reads still count.
    let other_k = DvInfo::new(1, 44, 0, 58, 0, 0, DM_I_43_0).unwrap();
    let other_b = DvInfo::new(1, 43, 2, 58, 0, 0, DM_I_43_0).unwrap();
    let mut dm = DM_I_43_0;
    dm[79] ^= 1;
    let other_dm = DvInfo::new(1, 43, 0, 58, 0, 0, dm).unwrap();
    for other in [other_k, other_b, other_dm, terminator()] {
        assert_ne!(other, r);
        assert_ne!(other.as_ref(), r.as_ref());
    }
}

#[test]
fn handles_read_the_same_record() {
    let mut r = i_43_0();
    let expected = format!("{r:?}");
    let shared: DvInfoRef<'_> = r.as_ref();
    let dm = shared.message_difference();
    assert_eq!(format!("{shared:?}"), expected);
    assert_eq!(dm[1], 0x9800_000C);
    let mut m = r.as_mut();
    assert_eq!(format!("{m:?}"), expected);
    let reborrowed = m.as_mut();
    assert_eq!(reborrowed.as_ref().recompression_step(), 58);
    assert_eq!(m.as_ref().copied(), i_43_0());
}
