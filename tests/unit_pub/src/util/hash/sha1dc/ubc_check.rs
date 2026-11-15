//! `src/util/hash/sha1dc/ubc_check.c`: [`DvInfo`], the record of one disturbance vector,
//! checked against its contract: what [`DvInfo::new`] keeps and refuses, its getters, copies,
//! equality and `Debug`, and a table of records built as a `static`, read as `sha1.c` reads
//! `sha1_dvs`; and the port's symbols: [`SHA1_DVS`], checked against what `sha1.c` reads of it
//! and against the structure of SHA-1's message expansion, the `DV_*_BIT` constants and
//! [`DVMASKSIZE`], and [`ubc_check`], checked against the masks `ubc_check.c`'s C returns.
//!
//! The module has no C test (`modules.json`'s `tests` is empty). The C test that reaches
//! `ubc_check` and `sha1_dvs`, `test_sha1__detect_collision_attack` (`tests/util/sha1.c`),
//! hashes `shattered-1.pdf` through `sha1.c`: its two near-collision blocks are this suite's
//! too. The masks the tests expect were computed by `ubc_check.c`'s C. None reaches C, so
//! Miri runs them all.

use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::{
    DV_I_43_0_BIT, DV_I_44_0_BIT, DV_I_45_0_BIT, DV_I_46_0_BIT, DV_I_46_2_BIT, DV_I_47_0_BIT,
    DV_I_47_2_BIT, DV_I_48_0_BIT, DV_I_48_2_BIT, DV_I_49_0_BIT, DV_I_49_2_BIT, DV_I_50_0_BIT,
    DV_I_50_2_BIT, DV_I_51_0_BIT, DV_I_51_2_BIT, DV_I_52_0_BIT, DV_II_45_0_BIT, DV_II_46_0_BIT,
    DV_II_46_2_BIT, DV_II_47_0_BIT, DV_II_48_0_BIT, DV_II_49_0_BIT, DV_II_49_2_BIT, DV_II_50_0_BIT,
    DV_II_50_2_BIT, DV_II_51_0_BIT, DV_II_51_2_BIT, DV_II_52_0_BIT, DV_II_53_0_BIT, DV_II_54_0_BIT,
    DV_II_55_0_BIT, DV_II_56_0_BIT, DVMASKSIZE, DvInfo, DvInfoRef, SHA1_DVS, ubc_check,
};

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
    // Each record differs from `r` in one field alone, so that equality leaving any field out
    // fails here; fields no getter reads still count.
    let other_type = DvInfo::new(2, 43, 0, 58, 0, 0, DM_I_43_0).unwrap();
    let other_k = DvInfo::new(1, 44, 0, 58, 0, 0, DM_I_43_0).unwrap();
    let other_b = DvInfo::new(1, 43, 2, 58, 0, 0, DM_I_43_0).unwrap();
    let other_step = DvInfo::new(1, 43, 0, 65, 0, 0, DM_I_43_0).unwrap();
    let other_bit = DvInfo::new(1, 43, 0, 58, 0, 1, DM_I_43_0).unwrap();
    let mut dm = DM_I_43_0;
    dm[79] ^= 1;
    let other_dm = DvInfo::new(1, 43, 0, 58, 0, 0, dm).unwrap();
    for other in [
        other_type,
        other_k,
        other_b,
        other_step,
        other_bit,
        other_dm,
        terminator(),
    ] {
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

/// Each `DV_*_bit` constant, with the type, `k` and `b` of the vector it names.
const BITS: [(u32, i32, i32, i32); 32] = [
    (DV_I_43_0_BIT, 1, 43, 0),
    (DV_I_44_0_BIT, 1, 44, 0),
    (DV_I_45_0_BIT, 1, 45, 0),
    (DV_I_46_0_BIT, 1, 46, 0),
    (DV_I_46_2_BIT, 1, 46, 2),
    (DV_I_47_0_BIT, 1, 47, 0),
    (DV_I_47_2_BIT, 1, 47, 2),
    (DV_I_48_0_BIT, 1, 48, 0),
    (DV_I_48_2_BIT, 1, 48, 2),
    (DV_I_49_0_BIT, 1, 49, 0),
    (DV_I_49_2_BIT, 1, 49, 2),
    (DV_I_50_0_BIT, 1, 50, 0),
    (DV_I_50_2_BIT, 1, 50, 2),
    (DV_I_51_0_BIT, 1, 51, 0),
    (DV_I_51_2_BIT, 1, 51, 2),
    (DV_I_52_0_BIT, 1, 52, 0),
    (DV_II_45_0_BIT, 2, 45, 0),
    (DV_II_46_0_BIT, 2, 46, 0),
    (DV_II_46_2_BIT, 2, 46, 2),
    (DV_II_47_0_BIT, 2, 47, 0),
    (DV_II_48_0_BIT, 2, 48, 0),
    (DV_II_49_0_BIT, 2, 49, 0),
    (DV_II_49_2_BIT, 2, 49, 2),
    (DV_II_50_0_BIT, 2, 50, 0),
    (DV_II_50_2_BIT, 2, 50, 2),
    (DV_II_51_0_BIT, 2, 51, 0),
    (DV_II_51_2_BIT, 2, 51, 2),
    (DV_II_52_0_BIT, 2, 52, 0),
    (DV_II_53_0_BIT, 2, 53, 0),
    (DV_II_54_0_BIT, 2, 54, 0),
    (DV_II_55_0_BIT, 2, 55, 0),
    (DV_II_56_0_BIT, 2, 56, 0),
];

/// The `k` and `b` of a record, which only its `Debug` shows.
fn parameters(r: &DvInfo) -> (i32, i32) {
    let debug = format!("{r:?}");
    let field = |name: &str| -> i32 {
        let start = debug.find(name).expect("a field of the record") + name.len();
        let end = start + debug[start..].find(',').expect("a field ends");
        debug[start..end].parse().expect("an integer")
    };
    (field(", k: "), field(", b: "))
}

/// SHA-1's message expansion: word `i` of an expanded block from the 16 before it. It is linear
/// over XOR, so the difference of two expanded blocks follows it too.
fn expanded(t: &[u32], i: usize) -> u32 {
    (t[i - 3] ^ t[i - 8] ^ t[i - 14] ^ t[i - 16]).rotate_left(1)
}

/// The expanded message block of a 16-word block, as `sha1_compression_states` computes it.
fn expand(block: &[u32; 16]) -> [u32; 80] {
    let mut w = [0; 80];
    w[..16].copy_from_slice(block);
    for i in 16..80 {
        w[i] = expanded(&w, i);
    }
    w
}

/// An 80-word block of `SplitMix64`'s output from `seed`, two words from each of its 40 values,
/// low half first.
fn seeded(seed: u64) -> [u32; 80] {
    let mut state = seed;
    let mut w = [0; 80];
    for pair in w.as_chunks_mut::<2>().0 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        pair[0] = u32::try_from(z & 0xFFFF_FFFF).unwrap();
        pair[1] = u32::try_from(z >> 32).unwrap();
    }
    w
}

/// For each vector, by its bit, the first seed whose block [`seeded`] gives selects it, and the
/// mask `ubc_check.c`'s C returns for that block.
const WITNESSES: [(u64, u32); 32] = [
    (1016, 0x0000_0001),
    (2073, 0x0000_0002),
    (8279, 0x0000_0004),
    (283, 0x0000_0008),
    (289, 0x0000_0010),
    (13141, 0x0000_0020),
    (11, 0x0000_0040),
    (562, 0x0000_0080),
    (125, 0x0000_0100),
    (23, 0x0000_0200),
    (185, 0x0000_0400),
    (4382, 0x0000_0800),
    (216, 0x0000_1000),
    (19083, 0x0000_2000),
    (356, 0x0000_4000),
    (169, 0x0000_8000),
    (201, 0x0001_0000),
    (912, 0x0002_0000),
    (37, 0x0004_0000),
    (18885, 0x0008_0000),
    (17678, 0x0010_0000),
    (15423, 0x0020_0000),
    (348, 0x0040_0000),
    (8928, 0x0080_0000),
    (729, 0x0100_0000),
    (5478, 0x0200_0000),
    (159, 0x0400_0100),
    (45875, 0x0800_0000),
    (32064, 0x1000_0000),
    (596, 0x2000_0000),
    (32134, 0x4000_0010),
    (5137, 0x8000_0000),
];

/// `shattered-1.pdf`'s two near-collision blocks, at bytes 192 to 320, as big-endian words:
/// the blocks of the shattered attack, whose differences follow the disturbance vector
/// `II(52,0)`.
const SHATTERED_BLOCKS: [u32; 32] = [
    0x7346_DC91,
    0x66B6_7E11,
    0x8F02_9AB6,
    0x21B2_560F,
    0xF9CA_67CC,
    0xA8C7_F85B,
    0xA84C_7903,
    0x0C2B_3DE2,
    0x18F8_6DB3,
    0xA909_01D5,
    0xDF45_C14F,
    0x26FE_DFB3,
    0xDC38_E96A,
    0xC22F_E7BD,
    0x728F_0E45,
    0xBCE0_46D2,
    0x3C57_0FEB,
    0x1413_98BB,
    0x552E_F5A0,
    0xA82B_E331,
    0xFEA4_8037,
    0xB8B5_D71F,
    0x0E33_2EDF,
    0x93AC_3500,
    0xEB4D_DC0D,
    0xECC1_A864,
    0x790C_782C,
    0x7621_5660,
    0xDD30_9791,
    0xD06B_D0AF,
    0x3F98_CDA4,
    0xBC46_29B1,
];

#[test]
fn the_mask_is_one_word() {
    assert_eq!(DVMASKSIZE, 1);
    let mask: [u32; DVMASKSIZE] = ubc_check(&[0; 80]);
    assert_eq!(mask.len(), 1);
}

/// `sha1_process` reads the table up to its record of type `0`, selects each record by its
/// bit, and recompresses from its step, one of the two states `sha1.c` saves
/// (`DOSTORESTATE58` and `DOSTORESTATE65`).
#[test]
fn the_table_lists_each_vector_at_its_bit_then_ends() {
    assert_eq!(SHA1_DVS.len(), 33);
    let ends = SHA1_DVS
        .iter()
        .position(|r| r.as_ref().dv_type() == 0)
        .expect("a record ends the table");
    assert_eq!(ends, 32);
    for (i, r) in SHA1_DVS[..32].iter().enumerate() {
        let h = r.as_ref();
        assert_eq!(h.mask_bit(), u32::try_from(i).unwrap(), "record {i}");
        assert_eq!(h.dv_type(), if i < 16 { 1 } else { 2 }, "record {i}");
        // `BITS` is in bit order: its `k` is record `i`'s, which
        // `each_bit_constant_names_its_record` checks.
        let (_, _, k, _) = BITS[i];
        let step = if k < 50 { 58 } else { 65 };
        assert_eq!(h.recompression_step(), step, "record {i}");
    }
    assert_eq!(SHA1_DVS[32], terminator());
}

#[test]
fn each_bit_constant_names_its_record() {
    let mut all = 0;
    for (i, (bit, dv_type, k, b)) in BITS.into_iter().enumerate() {
        assert_eq!(bit, 1 << i, "BITS is in bit order");
        let r = &SHA1_DVS[usize::try_from(bit.trailing_zeros()).unwrap()];
        assert_eq!(r.as_ref().dv_type(), dv_type, "{bit:#X}");
        assert_eq!(parameters(r), (k, b), "{bit:#X}");
        assert_eq!(1 << r.as_ref().mask_bit(), bit);
        all |= bit;
    }
    assert_eq!(all, u32::MAX);
}

/// A vector's message difference is the difference of two expanded message blocks, so it
/// follows the expansion: a corrupted word of the table breaks it.
#[test]
fn each_message_difference_follows_the_message_expansion() {
    for (i, r) in SHA1_DVS[..32].iter().enumerate() {
        let dm = r.as_ref().message_difference();
        assert_ne!(dm, &[0; 80], "record {i}");
        for t in 16..80 {
            assert_eq!(dm[t], expanded(dm, t), "record {i}, word {t}");
        }
    }
    assert_eq!(SHA1_DVS[0].as_ref().message_difference(), &DM_I_43_0);
}

/// `X(k+1,b)` is `X(k,b)` one step later: each word of its difference is the word before of
/// its predecessor's.
#[test]
fn vectors_of_one_type_and_b_are_shifts_of_each_other() {
    let mut pairs = 0;
    // Each record's type, `k` and `b` are `BITS`', in bit order, as
    // `each_bit_constant_names_its_record` checks.
    for (i, &(_, dv_type, k, b)) in BITS.iter().enumerate() {
        let Some(next) = BITS
            .iter()
            .position(|&(_, t, nk, nb)| (t, nk, nb) == (dv_type, k + 1, b))
        else {
            continue;
        };
        let (dm, later) = (
            SHA1_DVS[i].as_ref().message_difference(),
            SHA1_DVS[next].as_ref().message_difference(),
        );
        assert_eq!(&later[1..], &dm[..79], "{dv_type} {k} {b}");
        pairs += 1;
    }
    // Type I: 9 of b = 0 and 5 of b = 2; type II: 11 of b = 0 and 2 of b = 2.
    assert_eq!(pairs, 27);
}

/// A block whose words are all equal meets every condition that compares two of its bits for
/// equality and fails each that wants them to differ: no vector has all its conditions met.
#[test]
fn ubc_check_selects_no_vector_of_a_uniform_block() {
    assert_eq!(ubc_check(&[0; 80]), [0]);
    assert_eq!(ubc_check(&[u32::MAX; 80]), [0]);
}

/// Every vector is selected by some block, each with the mask C returns for it.
#[test]
fn ubc_check_selects_each_vector_as_c_does() {
    for (i, (seed, mask)) in WITNESSES.into_iter().enumerate() {
        let selected = ubc_check(&seeded(seed));
        assert_eq!(selected, [mask], "seed {seed}");
        assert_ne!(selected[0] & (1 << i), 0, "seed {seed}");
    }
}

/// The conditions read words 35 to 64 alone: the others change no mask.
#[test]
fn ubc_check_reads_words_35_to_64_alone() {
    for (seed, mask) in WITNESSES {
        let mut w = seeded(seed);
        let other = seeded(seed ^ 0xFFFF);
        w[..35].copy_from_slice(&other[..35]);
        w[65..].copy_from_slice(&other[65..]);
        assert_eq!(ubc_check(&w), [mask], "seed {seed}");
    }
}

/// 512 seeded blocks, each mask folded in turn, `fold = fold.rotate_left(5) ^ mask`: the value,
/// the count of non-zero masks and their union C gives. 512, so that Miri runs it in seconds;
/// the equivalence tests compare two million blocks with C.
#[test]
fn ubc_check_over_seeded_blocks_folds_as_c_does() {
    let (mut fold, mut selecting, mut selected) = (0_u32, 0, 0);
    for seed in 0..512 {
        let [mask] = ubc_check(&seeded(seed));
        selecting += usize::from(mask != 0);
        selected |= mask;
        fold = fold.rotate_left(5) ^ mask;
    }
    assert_eq!((fold, selecting, selected), (0xE8B3_66C2, 33, 0x0445_D758));
}

/// The shattered attack's blocks, which `test_sha1__detect_collision_attack` feeds `sha1.c`: each
/// selects `II(52,0)` alone, the vector whose recompression tells the collision.
#[test]
fn ubc_check_selects_ii_52_0_for_the_shattered_blocks() {
    for block in SHATTERED_BLOCKS.as_chunks::<16>().0 {
        assert_eq!(ubc_check(&expand(block)), [DV_II_52_0_BIT]);
    }
    let r = SHA1_DVS[27].as_ref();
    assert_eq!((r.dv_type(), r.recompression_step()), (2, 65));
}
