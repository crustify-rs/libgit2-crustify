//! Equivalence of `src/util/hash/sha1dc/ubc_check.c`'s `sha1_dvs` and `ubc_check` with their
//! ports, `SHA1_DVS` and `ubc_check`.
//!
//! The table is compared record for record, every field included. The check is compared on
//! the blocks `sha1.c` hands it, the expanded message blocks of a stream, and on blocks that
//! reach each of its conditions: two million seeded ones, of which about one in twenty
//! selects a vector and each vector is selected; every one-bit change of a block that
//! selects each vector, which fails its conditions one at a time; and every block of
//! `shattered-1.pdf`, whose two near-collision blocks select `II(52,0)`. Each block is one
//! array, which the C reads through its pointer and the Rust borrows, and the C writes its
//! mask over a slot that starts at `u32::MAX`, as `sha1_process`'s does.
//!
//! `BSan`, which does not finish the full tests within its hour, checks fewer: the edge
//! blocks and 1024 seeded ones, the one-bit changes of the first selecting block in the words
//! the conditions read, and the first eight blocks of `shattered-1.pdf`, its near-collision
//! pair among them. Every condition of the C and the Rust runs on each call, so each still
//! reaches every path the full test checks but the guarded clears of the vectors it does not
//! select.

use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::{
    DV_II_52_0_BIT, DVMASKSIZE, DvInfoRef, SHA1_DVS, ubc_check,
};
use libgit2_crustify::sys::libgit2 as sys;

/// `ubc_check.c`'s `ubc_check` of `w`: the mask it writes.
fn c_ubc_check(w: &[u32; 80]) -> [u32; DVMASKSIZE] {
    let mut mask = [u32::MAX; DVMASKSIZE];
    // SAFETY: `w` is 80 initialised words, which the C only reads during the call, and `mask`
    // the `DVMASKSIZE` words it writes.
    unsafe { sys::ubc_check(w.as_ptr(), mask.as_mut_ptr()) };
    mask
}

/// The Rust and the C of `w`, compared; the mask.
fn same(w: &[u32; 80], what: &dyn Fn() -> String) -> u32 {
    let rust = ubc_check(w);
    assert_eq!(rust, c_ubc_check(w), "{}", what());
    rust[0]
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

/// The expanded message block of a 64-byte block, as `sha1_compression_states` computes it
/// from the words `sha1_process` loads big-endian.
fn expand(block: &[u8; 64]) -> [u32; 80] {
    let mut w = [0; 80];
    for (word, bytes) in w.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*bytes);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    w
}

#[test]
#[cfg_attr(miri, ignore = "reads a C global")]
fn sha1_dvs_is_the_c_table() {
    let c_table = (&raw const sys::sha1_dvs).cast::<sys::dv_info_t>();
    for (i, rust) in SHA1_DVS.iter().enumerate() {
        // SAFETY: the C defines `sha1_dvs` with 33 records, which nothing writes, and `i` is
        // below 33: the pointer addresses an initialised, aligned record of the table, which
        // lives as long as the program. `wrapping_add` leaves the declared bound of `[_; 0]`.
        let c = unsafe {
            <DvInfoRef<'static> as ffibox::CRefSeam<'static>>::from_ptr(
                c_table.wrapping_add(i).cast_mut(),
            )
        }
        .expect("a record");
        assert_eq!(c.copied(), *rust, "record {i}");
        assert_eq!(c, rust.as_ref(), "record {i}");
    }
}

/// Edge blocks, then two million seeded ones: about one in twenty selects a vector, and every
/// vector is selected.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn ubc_check_matches_c_on_seeded_blocks() {
    let mut edges = vec![
        [0; 80],
        [u32::MAX; 80],
        [0xAAAA_AAAA; 80],
        [0x5555_5555; 80],
    ];
    for i in 0..80 {
        let mut w = [0; 80];
        w[i] = u32::MAX;
        edges.push(w);
        edges.push(w.map(|x| !x));
    }
    for w in &edges {
        same(w, &|| format!("{w:X?}"));
    }
    let seeds: u64 = if cfg!(bsan) { 1 << 10 } else { 1 << 21 };
    let (mut selecting, mut selected) = (0_u64, 0_u32);
    for seed in 0..seeds {
        let mask = same(&seeded(seed), &|| format!("seed {seed}"));
        selecting += u64::from(mask != 0);
        selected |= mask;
    }
    if !cfg!(bsan) {
        assert_eq!(selected, u32::MAX);
    }
    assert!(selecting > seeds / 32, "{selecting}");
}

/// For each vector, the first seeded block that selects it, and every one of its one-bit
/// changes: each condition the vector's bit depends on fails alone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn ubc_check_matches_c_on_each_one_bit_change_of_a_selecting_block() {
    let (wanted, words) = if cfg!(bsan) {
        (1, 35..=64)
    } else {
        (32, 0..=79)
    };
    let mut witnesses = [None; 32];
    let mut seed = 0;
    while witnesses.iter().flatten().count() < wanted {
        let w = seeded(seed);
        let mask = same(&w, &|| format!("seed {seed}"));
        for (bit, slot) in witnesses.iter_mut().enumerate() {
            if mask & (1 << bit) != 0 && slot.is_none() {
                *slot = Some((seed, w));
            }
        }
        seed += 1;
    }
    let mut changed = 0;
    for (seed, w) in witnesses.into_iter().flatten() {
        let mask = same(&w, &|| format!("seed {seed}"));
        for word in words.clone() {
            for bit in 0..32 {
                let mut flipped = w;
                flipped[word] ^= 1 << bit;
                let m = same(&flipped, &|| format!("seed {seed}, word {word}, bit {bit}"));
                changed += u32::from(m != mask);
                assert!(
                    m == mask || (35..=64).contains(&word),
                    "seed {seed}, word {word}"
                );
            }
        }
    }
    assert!(changed > 0);
}

/// Every block of `shattered-1.pdf`, as `sha1.c` checks it while it hashes the file in
/// `test_sha1__detect_collision_attack`: its near-collision blocks, the fourth and fifth,
/// select `II(52,0)`.
#[test]
#[cfg_attr(miri, ignore = "reads a file")]
fn ubc_check_matches_c_on_shattered_1_pdf() {
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/sha1/shattered-1.pdf"
    ))
    .unwrap();
    let blocks = pdf.as_chunks::<64>().0;
    let checked = if cfg!(bsan) { &blocks[..8] } else { blocks };
    let mut selecting = 0;
    for (i, block) in checked.iter().enumerate() {
        let mask = same(&expand(block), &|| format!("block {i}"));
        selecting += usize::from(mask != 0);
        if i == 3 || i == 4 {
            assert_eq!(mask, DV_II_52_0_BIT, "block {i}");
        }
    }
    if !cfg!(bsan) {
        assert_eq!((pdf.len() / 64, selecting), (6600, 337));
    }
}
