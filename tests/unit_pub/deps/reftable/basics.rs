//! Unit tests for `deps/reftable/basics.c`: `HashId`'s conversions, sizes and their error,
//! the buffer `Buf` through its owner and handles, the owners of reftable's allocator,
//! `parse_names`, `binsearch` and the error types, the size and format constants, the
//! big-endian helpers, `alloc_size`, `swap`, `free_and_null`, the allocation macros' forms
//! and name lists, decided by Rust assertions.
//!
//! Every test that allocates or calls C is ignored under Miri, which cannot call libgit2.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::error::Error as _;
use std::ffi::CStr;
use std::fmt::{self, Write as _};
use std::mem::MaybeUninit;
use std::panic;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{
    Buf, BufMut, BufRef, BufVal, Error, HashId, LenOutOfRange, Names, NamesRef, NamesVec,
    ReftableArrayFree, ReftableBox, ReftableStr, ReftableVec, UnnamedHashId, binsearch,
    parse_names,
};

#[test]
fn hash_id_converts_from_and_to_its_c_value() {
    assert_eq!(HashId::from_raw(89), Some(HashId::Sha1));
    assert_eq!(HashId::from_raw(247), Some(HashId::Sha256));
    assert_eq!(HashId::Sha1.raw(), 89);
    assert_eq!(HashId::Sha256.raw(), 247);
    assert_eq!(ffi::reftable_hash::from(HashId::Sha256), 247);
    assert_eq!(HashId::try_from(89), Ok(HashId::Sha1));
}

#[test]
fn unset_and_unknown_hash_ids_are_refused_with_their_value() {
    // `0` is reftable's unset id, which its routines read as SHA-1: no variant names it.
    for raw in [0, 1, 20, 32, 88, 90, 246, 248, u32::MAX] {
        assert_eq!(HashId::from_raw(raw), None);
        let err = HashId::try_from(raw).unwrap_err();
        assert_eq!(err.raw(), raw);
        assert_eq!(
            err.to_string(),
            format!("reftable hash id {raw} is not named by reftable-basics.h")
        );
        assert!(std::error::Error::source(&err).is_none());
    }
}

#[test]
fn hash_id_and_its_error_compare_and_hash_by_value() {
    let ids: HashSet<HashId> = [HashId::Sha1, HashId::Sha256, HashId::Sha1]
        .into_iter()
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(HashId::Sha1, HashId::Sha256);
    assert_eq!(format!("{:?}", HashId::Sha256), "Sha256");
    let errs: HashSet<UnnamedHashId> = [0, 0, 1]
        .into_iter()
        .map(|raw| HashId::try_from(raw).unwrap_err())
        .collect();
    assert_eq!(errs.len(), 2);
}

#[test]
fn empty_buf_reports_no_bytes_through_every_handle() {
    let mut value = Buf::zeroed();
    let r = value.as_ref();
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
    assert_eq!(r.capacity(), 0);
    assert_eq!(r.as_bytes(), b"");
    assert_eq!(r.as_c_str(), Some(c""));
    let mut m = value.as_mut();
    assert_eq!(m.as_mut().as_ref().len(), 0);
    assert_eq!(m.as_ref().as_bytes(), b"");
}

#[test]
fn buf_debug_shows_length_capacity_and_escaped_bytes() {
    let mut value = Buf::zeroed();
    let expected = r#"Buf { len: 0, capacity: 0, bytes: "" }"#;
    assert_eq!(format!("{value:?}"), expected);
    assert_eq!(format!("{:?}", value.as_ref()), expected);
    assert_eq!(format!("{:?}", value.as_mut()), expected);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn hash_ids_report_their_id_sizes() {
    use libgit2::libgit2::deps::reftable::basics::{HASH_SIZE_SHA1, HASH_SIZE_SHA256};

    assert_eq!(HashId::Sha1.size(), 20);
    assert_eq!(HashId::Sha256.size(), 32);
    // In the unit of the constants and of slices, so that an id is cut without a cast.
    assert_eq!(HashId::Sha1.size(), HASH_SIZE_SHA1);
    assert_eq!(HashId::Sha256.size(), HASH_SIZE_SHA256);
    let id = [0xAB_u8; HASH_SIZE_SHA256];
    assert_eq!(id[..HashId::Sha1.size()].len(), HASH_SIZE_SHA1);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn new_buf_appends_bytes_and_strings_with_a_terminator() {
    let mut value = Buf::new();
    assert!(value.as_ref().is_empty());
    assert_eq!(
        value.as_ref().capacity(),
        0,
        "owns nothing until it is filled"
    );
    value.as_mut().add(b"refs/").unwrap();
    value.as_mut().add_str(c"heads/main").unwrap();
    let r = value.as_ref();
    assert_eq!(r.as_bytes(), b"refs/heads/main");
    assert_eq!(r.as_c_str(), Some(c"refs/heads/main"));
    assert!(r.capacity() > r.len());
    // Arbitrary bytes, NUL included.
    value.as_mut().add(&[0, 0xFF]).unwrap();
    assert_eq!(value.as_ref().as_bytes(), b"refs/heads/main\0\xFF");
    assert_eq!(value.as_ref().as_c_str(), None, "a NUL comes first");
    // An empty append still allocates the terminator's byte.
    let mut empty = Buf::new();
    empty.as_mut().add(b"").unwrap();
    assert_eq!(empty.as_ref().as_c_str(), Some(c""));
    assert!(empty.as_ref().capacity() >= 1);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn truncate_and_reset_keep_the_allocation() {
    let mut value = Buf::new();
    value.as_mut().add(b"0x0001-0x0002.ref").unwrap();
    let capacity = value.as_ref().capacity();
    assert_eq!(value.as_mut().truncate(18), Err(LenOutOfRange));
    assert_eq!(value.as_ref().as_bytes(), b"0x0001-0x0002.ref");
    value.as_mut().truncate(17).unwrap();
    value.as_mut().truncate(6).unwrap();
    assert_eq!(value.as_ref().as_c_str(), Some(c"0x0001"));
    assert_eq!(value.as_ref().capacity(), capacity);
    value.as_mut().reset();
    assert_eq!(value.as_ref().as_c_str(), Some(c""));
    assert_eq!(value.as_ref().capacity(), capacity);
    // An empty buffer owning nothing trims to 0 alone.
    let mut empty = Buf::new();
    empty.as_mut().reset();
    assert_eq!(empty.as_mut().truncate(0), Ok(()));
    assert_eq!(empty.as_mut().truncate(1), Err(LenOutOfRange));
    assert_eq!(empty.as_ref().capacity(), 0);
    assert_eq!(
        LenOutOfRange.to_string(),
        "length beyond the buffer's contents"
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn detach_hands_the_contents_out_and_empties_the_buffer() {
    let mut value = Buf::new();
    assert_eq!(value.as_mut().detach(), None, "nothing to hand out");
    value.as_mut().add_str(c"tables.list").unwrap();
    let s: ReftableStr = value.as_mut().detach().unwrap();
    assert_eq!(s.as_c_str(), c"tables.list");
    assert_eq!(value.as_ref().capacity(), 0);
    assert!(value.as_ref().is_empty());
    // Reusable afterwards, as C's comment promises.
    value.as_mut().add(b"a\0b").unwrap();
    let s = value.as_mut().detach().unwrap();
    assert_eq!(s.as_c_str(), c"a", "read up to the first NUL");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn buffers_order_bytewise_and_share_prefixes() {
    let bufs = |items: &[&[u8]]| {
        items
            .iter()
            .map(|bytes| {
                let mut value = Buf::new();
                if !bytes.is_empty() {
                    value.as_mut().add(bytes).unwrap();
                }
                value
            })
            .collect::<Vec<_>>()
    };
    let values = bufs(&[b"", b"a", b"ab", b"abc", b"b", b"\xFF", b"ab"]);
    let r: Vec<_> = values.iter().map(BufVal::as_ref).collect();
    assert_eq!(r[0].cmp(&r[1]), Ordering::Less, "empty first");
    assert_eq!(
        r[1].cmp(&r[2]),
        Ordering::Less,
        "a prefix before what extends it"
    );
    assert_eq!(r[3].cmp(&r[4]), Ordering::Less);
    assert_eq!(r[5].cmp(&r[4]), Ordering::Greater, "bytes compare unsigned");
    assert_eq!(r[2], r[6]);
    assert_ne!(r[2], r[3]);
    assert_eq!(r[2].partial_cmp(&r[3]), Some(Ordering::Less));
    let mut sorted = r.clone();
    sorted.sort();
    let order: Vec<&[u8]> = sorted.iter().map(BufRef::as_bytes).collect();
    assert_eq!(order, [&b""[..], b"a", b"ab", b"ab", b"abc", b"b", b"\xFF"]);
    assert_eq!(r[3].common_prefix_len(r[2]), 2);
    assert_eq!(r[3].common_prefix_len(r[4]), 0);
    assert_eq!(r[3].common_prefix_len(r[3]), 3);
    assert_eq!(r[0].common_prefix_len(r[3]), 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn buffers_hash_by_the_contents_they_compare() {
    // Equal contents in allocations of different sizes, and an unallocated buffer beside an
    // allocated empty one: equal buffers must hash alike.
    let mut short = Buf::new();
    short.as_mut().add(b"refs/heads/main").unwrap();
    let mut long = Buf::new();
    long.as_mut().add(b"refs/heads/main-and-more").unwrap();
    long.as_mut().truncate(15).unwrap();
    assert_ne!(short.as_ref().capacity(), long.as_ref().capacity());
    let unallocated = Buf::new();
    let mut emptied = Buf::new();
    emptied.as_mut().add(b"x").unwrap();
    emptied.as_mut().reset();
    let mut other = Buf::new();
    other.as_mut().add(b"refs/heads/maint").unwrap();
    let set: HashSet<BufRef<'_>> = [
        short.as_ref(),
        long.as_ref(),
        unallocated.as_ref(),
        emptied.as_ref(),
        other.as_ref(),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 3, "main, empty and maint");
    assert!(set.contains(&long.as_ref()));
    assert!(set.contains(&unallocated.as_ref()));
}

/// The traits a `Vec<u8>` has that the buffer's forms share, checked at compile time.
#[test]
fn buffers_cross_threads_as_a_vec_does() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Buf>();
    send_sync::<BufVal>();
    send_sync::<BufRef<'_>>();
    send_sync::<BufMut<'_>>();
    send_sync::<ReftableBox<Buf>>();
}

/// The traits a `Vec` of strings, and a slice of them, have that the name lists share,
/// checked at compile time.
#[test]
fn name_lists_cross_threads_as_a_vec_of_strings_does() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Names>();
    send_sync::<NamesVec<'_>>();
    send_sync::<NamesRef<'_>>();
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn grow_with_keeps_the_elements_and_fills_the_new_ones() {
    let mut run = ReftableVec::from_slice(&[7u32, 8]).unwrap();
    let mut next = 100;
    let mut fill = || {
        next += 1;
        next
    };
    ReftableArrayFree::grow_with(&mut run, 2, &mut fill).unwrap();
    assert_eq!(&*run, [7, 8], "long enough already: nothing filled");
    ReftableArrayFree::grow_with(&mut run, 3, &mut fill).unwrap();
    assert_eq!(
        &*run,
        [7, 8, 101, 102, 103],
        "2 * 2 + 1 elements, filled in order"
    );
    ReftableArrayFree::grow_with(&mut run, 9, &mut fill).unwrap();
    assert_eq!(run.len(), 2 * 5 + 1);
    assert_eq!(run[..5], [7, 8, 101, 102, 103]);
    assert_eq!(run[5..], [104, 105, 106, 107, 108, 109]);
    // A size that overflows fails, and the run is left as it was.
    assert_eq!(
        ReftableArrayFree::grow_with(&mut run, usize::MAX, &mut fill),
        Err(Error::OutOfMemory)
    );
    assert_eq!(run.len(), 11);
    assert_eq!(run[10], 109);
    // An empty run grows from nothing, as `REFTABLE_ALLOC_GROW` grows a NULL array.
    let mut names = ReftableVec::<ReftableStr>::from_fn(0, |_| unreachable!()).unwrap();
    ReftableArrayFree::grow_with(&mut names, 1, || ReftableStr::from_c_str(c"a.ref").unwrap())
        .unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].as_c_str(), c"a.ref");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reftable_owners_allocate_copy_and_resize() {
    let run = ReftableVec::from_slice(&[1u32, 2, 3]).unwrap();
    assert_eq!(&*run, [1, 2, 3]);
    assert_eq!(run.clone(), run);
    let zeroed = ReftableArrayFree::new_zeroed::<u64>(4).unwrap();
    assert_eq!(&*zeroed, [0; 4]);
    assert!(ReftableArrayFree::new_zeroed::<u8>(0).unwrap().is_empty());
    assert!(ReftableArrayFree::new_zeroed::<u64>(usize::MAX / 4).is_none());

    let s = ReftableStr::from_c_str(c"0x01-0x02-abcd.ref").unwrap();
    assert_eq!(s.clone(), s);
    assert_eq!(s.to_str(), Ok("0x01-0x02-abcd.ref"));

    let boxed = ReftableBox::<Buf>::new_zeroed().unwrap();
    assert!(boxed.as_ref().is_empty());
    assert!(ReftableBox::<Buf>::new_uninit().is_some());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn runs_grow_as_reftable_alloc_grow_does() {
    let mut run = ReftableVec::<u16>::new_uninit(0).unwrap();
    assert_eq!(ReftableArrayFree::grow(&mut run, 0), Ok(()));
    assert!(run.is_empty(), "long enough already");
    ReftableArrayFree::grow(&mut run, 1).unwrap();
    assert_eq!(run.len(), 1, "max(2 * 0 + 1, 1)");
    run[0] = MaybeUninit::new(11);
    ReftableArrayFree::grow(&mut run, 2).unwrap();
    assert_eq!(run.len(), 3, "2 * 1 + 1");
    ReftableArrayFree::grow(&mut run, 10).unwrap();
    assert_eq!(run.len(), 10, "the request wins");
    ReftableArrayFree::grow(&mut run, 4).unwrap();
    assert_eq!(run.len(), 10);
    // Shrinking keeps the allocation's first elements; resizing to 0 releases it.
    ReftableArrayFree::resize(&mut run, 2).unwrap();
    assert_eq!(run.len(), 2);
    ReftableArrayFree::resize(&mut run, 0).unwrap();
    assert!(run.is_empty());
    assert_eq!(
        ReftableArrayFree::resize(&mut run, usize::MAX),
        Err(Error::OutOfMemory)
    );
    assert_eq!(
        ReftableArrayFree::grow(&mut run, isize::MAX.cast_unsigned()),
        Err(Error::OutOfMemory)
    );
    assert!(run.is_empty(), "left as it was");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parse_names_reads_newline_ended_lines_and_drops_empty_ones() {
    let names = parse_names(b"0x01-0x02-a.ref\n\n0x03-0x04-b.ref\n").unwrap();
    assert_eq!(names.len(), 2);
    assert!(!names.is_empty());
    assert_eq!(names.get(0), Some(c"0x01-0x02-a.ref"));
    assert_eq!(names.get(1), Some(c"0x03-0x04-b.ref"));
    assert_eq!(names.get(2), None);
    assert_eq!(names.iter().len(), 2);
    assert_eq!(
        format!("{names:?}"),
        r#"["0x01-0x02-a.ref", "0x03-0x04-b.ref"]"#
    );
    for input in [&b""[..], b"\n", b"\n\n\n"] {
        let names = parse_names(input).unwrap();
        assert!(names.is_empty(), "{input:?}");
        assert_eq!(names.iter().next(), None);
    }
    // Bytes beyond ASCII are names too.
    let names = parse_names(b"\xFF\xFE\n").unwrap();
    assert_eq!(names.get(0).map(CStr::to_bytes), Some(&b"\xFF\xFE"[..]));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parse_names_refuses_unended_lines_and_nul_bytes() {
    for input in [&b"a"[..], b"a\nb", b"a\0\n", b"\0", b"a\n\0b\n"] {
        assert_eq!(parse_names(input).unwrap_err(), Error::Format, "{input:?}");
    }
}

#[test]
fn parse_names_refuses_inputs_c_cannot_count() {
    // Lazily zeroed pages: nothing reads past the length check.
    let input = vec![0u8; usize::try_from(i32::MAX).unwrap() + 1];
    assert_eq!(parse_names(&input).unwrap_err(), Error::Api);
}

/// A sink that refuses every write, as a full buffer or a closed stream does.
struct Refusing;

impl fmt::Write for Refusing {
    fn write_str(&mut self, _: &str) -> fmt::Result {
        Err(fmt::Error)
    }
}

#[test]
fn errors_pass_a_refusing_sink_s_failure_on() {
    let unnamed = HashId::try_from(0).unwrap_err();
    assert_eq!(write!(Refusing, "{unnamed}"), Err(fmt::Error));
    assert_eq!(write!(Refusing, "{}", Error::Other(-2)), Err(fmt::Error));
    assert_eq!(write!(Refusing, "{LenOutOfRange}"), Err(fmt::Error));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_finds_the_first_true_index_of_an_ascending_predicate() {
    for len in 0..40 {
        for first_true in 0..=len {
            let mut probed = Vec::new();
            let found = binsearch(len, |k| {
                probed.push(k);
                Ok::<_, ()>(k >= first_true)
            });
            assert_eq!(found, Ok(first_true), "len {len}, first true {first_true}");
            assert!(probed.iter().all(|&k| k < len), "{probed:?}");
            assert!(probed.len() <= 8, "logarithmic: {probed:?}");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_stops_at_the_first_error() {
    let mut calls = 0;
    let found = binsearch(100, |k| {
        calls += 1;
        if k < 50 { Ok(false) } else { Err(k) }
    });
    assert_eq!(found, Err(50), "the midpoint is probed first");
    assert_eq!(calls, 1);
    // C's last probe, of index 0, reads a failure as `true`; the error still wins.
    let found = binsearch(3, |k| if k == 0 { Err("probe 0") } else { Ok(true) });
    assert_eq!(found, Err("probe 0"));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_resumes_a_panic_of_its_predicate() {
    let caught = panic::catch_unwind(|| {
        binsearch(10, |k| -> Result<bool, ()> {
            assert!(k != 5, "probe {k}");
            Ok(false)
        })
    });
    let payload = caught.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<String>().map(String::as_str),
        Some("probe 5")
    );
}

#[test]
fn errors_carry_reftable_codes_and_messages() {
    for (error, code, message) in [
        (
            Error::Format,
            ffi::REFTABLE_FORMAT_ERROR,
            "corrupt reftable file",
        ),
        (
            Error::Api,
            ffi::REFTABLE_API_ERROR,
            "misuse of the reftable API",
        ),
        (
            Error::OutOfMemory,
            ffi::REFTABLE_OUT_OF_MEMORY_ERROR,
            "out of memory",
        ),
        (
            Error::Other(-2),
            ffi::REFTABLE_IO_ERROR,
            "reftable error code -2",
        ),
    ] {
        assert_eq!(error.code(), code);
        assert_eq!(Error::from_code(code), error);
        assert_eq!(error.to_string(), message);
        assert!(error.source().is_none());
    }
    let set: HashSet<Error> = [Error::Format, Error::Format, Error::Other(-1)].into();
    assert_eq!(set.len(), 2);
}

#[test]
fn hash_sizes_are_those_of_sha1_and_sha256_ids() {
    use libgit2::libgit2::deps::reftable::basics::{
        HASH_SIZE_MAX, HASH_SIZE_SHA1, HASH_SIZE_SHA256,
    };

    assert_eq!(HASH_SIZE_SHA1, 20);
    assert_eq!(HASH_SIZE_SHA256, 32);
    assert_eq!(HASH_SIZE_MAX, HASH_SIZE_SHA256);
    // Records hold their ids inline in arrays of the longest size.
    let id = [0u8; HASH_SIZE_MAX];
    assert!(id.len() >= HASH_SIZE_SHA1);
}

#[test]
fn format_ids_are_their_names_read_big_endian() {
    use libgit2::libgit2::deps::reftable::basics::{FORMAT_ID_SHA1, FORMAT_ID_SHA256, get_be32};

    // `basics.h`'s values, which are on disk.
    assert_eq!(FORMAT_ID_SHA1, 0x7368_6131);
    assert_eq!(FORMAT_ID_SHA256, 0x7332_3536);
    assert_eq!(FORMAT_ID_SHA1.to_be_bytes(), *b"sha1");
    assert_eq!(FORMAT_ID_SHA256.to_be_bytes(), *b"s256");
    assert_eq!(get_be32(b"sha1"), FORMAT_ID_SHA1);
    assert_ne!(FORMAT_ID_SHA1, FORMAT_ID_SHA256);
}

#[test]
fn big_endian_helpers_write_the_most_significant_byte_first() {
    use libgit2::libgit2::deps::reftable::basics::{put_be16, put_be24, put_be32, put_be64};

    let mut b2 = [0u8; 2];
    put_be16(&mut b2, 0x0102);
    assert_eq!(b2, [0x01, 0x02]);
    let mut b3 = [0u8; 3];
    put_be24(&mut b3, 0x01_0203);
    assert_eq!(b3, [0x01, 0x02, 0x03]);
    let mut b4 = [0u8; 4];
    put_be32(&mut b4, 0x0102_0304);
    assert_eq!(b4, [0x01, 0x02, 0x03, 0x04]);
    let mut b8 = [0u8; 8];
    put_be64(&mut b8, 0x0102_0304_0506_0708);
    assert_eq!(b8, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
}

#[test]
fn put_be24_drops_the_top_byte() {
    use libgit2::libgit2::deps::reftable::basics::{get_be24, put_be24};

    let mut out = [0xAAu8; 3];
    put_be24(&mut out, 0xFF12_3456);
    assert_eq!(out, [0x12, 0x34, 0x56]);
    assert_eq!(get_be24(&out), 0x12_3456);
    put_be24(&mut out, u32::MAX);
    assert_eq!(get_be24(&out), (1 << 24) - 1);
}

#[test]
fn big_endian_helpers_round_trip_at_their_edges() {
    use libgit2::libgit2::deps::reftable::basics::{
        get_be16, get_be24, get_be32, get_be64, put_be16, put_be24, put_be32, put_be64,
    };

    for v in [0, 1, 0x7F, 0x80, 0xFF, 0x100, 0x7FFF, 0x8000, u16::MAX] {
        let mut b = [0; 2];
        put_be16(&mut b, v);
        assert_eq!(get_be16(&b), v);
    }
    for v in [
        0, 1, 0xFF, 0x100, 0xFFFF, 0x1_0000, 0x7F_FFFF, 0x80_0000, 0xFF_FFFF,
    ] {
        let mut b = [0; 3];
        put_be24(&mut b, v);
        assert_eq!(get_be24(&b), v);
    }
    for v in [0, 1, 0xFF, 0x1_0000, 0x7FFF_FFFF, 0x8000_0000, u32::MAX] {
        let mut b = [0; 4];
        put_be32(&mut b, v);
        assert_eq!(get_be32(&b), v);
    }
    for v in [0, 1, 0xFF, 1 << 32, i64::MAX as u64, 1 << 63, u64::MAX] {
        let mut b = [0; 8];
        put_be64(&mut b, v);
        assert_eq!(get_be64(&b), v);
    }
}

#[test]
fn big_endian_helpers_read_every_byte_into_its_place() {
    use libgit2::libgit2::deps::reftable::basics::{get_be16, get_be24, get_be32, get_be64};

    assert_eq!(get_be16(&[0xFF, 0x00]), 0xFF00);
    assert_eq!(get_be24(&[0x00, 0xFF, 0x00]), 0x00_FF00);
    assert_eq!(get_be32(&[0x80, 0, 0, 1]), 0x8000_0001);
    assert_eq!(
        get_be64(&[0x80, 0, 0, 0, 0, 0, 0, 1]),
        0x8000_0000_0000_0001
    );
}

#[test]
fn big_endian_helpers_evaluate_at_compile_time() {
    use libgit2::libgit2::deps::reftable::basics::{get_be24, put_be24};

    const ENCODED: [u8; 3] = {
        let mut out = [0; 3];
        put_be24(&mut out, 4096);
        out
    };
    const BLOCK_SIZE: u32 = get_be24(&ENCODED);
    assert_eq!(ENCODED, [0x00, 0x10, 0x00]);
    assert_eq!(BLOCK_SIZE, 4096);
}

#[test]
fn alloc_size_multiplies_or_reports_overflow() {
    use libgit2::libgit2::deps::reftable::basics::alloc_size;

    assert_eq!(alloc_size(0, 0), Some(0));
    assert_eq!(alloc_size(0, usize::MAX), Some(0));
    assert_eq!(alloc_size(usize::MAX, 0), Some(0));
    assert_eq!(alloc_size(3, 8), Some(24));
    assert_eq!(alloc_size(1, usize::MAX), Some(usize::MAX));
    assert_eq!(alloc_size(usize::MAX, 1), Some(usize::MAX));
    assert_eq!(alloc_size(2, usize::MAX / 2), Some(usize::MAX - 1));
    assert_eq!(alloc_size(2, usize::MAX / 2 + 1), None);
    assert_eq!(alloc_size(usize::MAX, usize::MAX), None);
    let half = 1usize << (usize::BITS / 2);
    assert_eq!(alloc_size(half, half), None);
    assert_eq!(alloc_size(half - 1, half + 1), Some(usize::MAX));
}

#[test]
fn swap_exchanges_values_of_one_type() {
    use libgit2::libgit2::deps::reftable::basics::swap;

    let (mut a, mut b) = (1u32, 2u32);
    swap(&mut a, &mut b);
    assert_eq!((a, b), (2, 1));
    // As the priority queue swaps heap entries in place.
    let mut heap = vec![String::from("refs/b"), String::from("refs/a")];
    let (first, rest) = heap.split_at_mut(1);
    swap(&mut first[0], &mut rest[0]);
    assert_eq!(heap, ["refs/a", "refs/b"]);
}

#[test]
fn free_and_null_drops_what_the_slot_owns_once() {
    use std::rc::Rc;

    use libgit2::libgit2::deps::reftable::basics::free_and_null;

    let owned = Rc::new(());
    let mut slot = Some(Rc::clone(&owned));
    assert_eq!(Rc::strong_count(&owned), 2);
    free_and_null(&mut slot);
    assert!(slot.is_none());
    assert_eq!(Rc::strong_count(&owned), 1);
    // An empty slot stays empty, as `reftable_free(NULL)` frees nothing.
    free_and_null(&mut slot);
    assert!(slot.is_none());
    assert_eq!(Rc::strong_count(&owned), 1);
}

#[test]
fn buf_init_is_the_empty_buffer() {
    let init = Buf::INIT;
    let zeroed = Buf::zeroed();
    assert_eq!(format!("{init:?}"), format!("{zeroed:?}"));
    let r = init.as_ref();
    assert_eq!((r.len(), r.capacity(), r.as_bytes()), (0, 0, &b""[..]));
    assert_eq!(r.as_c_str(), Some(c""));
    // A constant: each use is a value of its own.
    let mut bufs = [Buf::INIT, Buf::INIT];
    assert!(bufs.iter_mut().all(|b| b.as_mut().as_ref().is_empty()));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn names_count_their_entries() {
    use libgit2::libgit2::deps::reftable::basics::NamesVec;

    let none = NamesVec::new([]);
    assert_eq!(none.as_ref().len(), 0);
    assert!(none.as_ref().is_empty());
    let two = NamesVec::new([c"0x01-0x02-a.ref", c"0x03-0x03-b.ref"]);
    assert_eq!(two.as_ref().len(), 2);
    assert!(!two.as_ref().is_empty());
    // An empty name is a name: the list ends at the NULL slot, not at an empty string.
    let with_empty = NamesVec::new([c"", c"x", c""]);
    assert_eq!(with_empty.as_ref().len(), 3);
}

#[test]
fn names_iterate_in_order() {
    use libgit2::libgit2::deps::reftable::basics::NamesVec;

    let names = [c"a.ref", c"", c"b.ref"];
    let list = NamesVec::from(&names[..]);
    let read: Vec<&CStr> = list.as_ref().iter().collect();
    assert_eq!(read, names);
    let r = list.as_ref();
    assert_eq!((&r).into_iter().count(), 3);
    assert_eq!(r.into_iter().last(), Some(c"b.ref"));
    assert_eq!(format!("{list:?}"), r#"["a.ref", "", "b.ref"]"#);
    assert_eq!(format!("{:?}", NamesVec::new([])), "[]");
    assert!(NamesVec::new([]).as_ref().iter().next().is_none());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn names_compare_by_content_and_order() {
    use libgit2::libgit2::deps::reftable::basics::NamesVec;

    let a: NamesVec<'_> = [c"t1.ref", c"t2.ref"].into_iter().collect();
    let same_content = [c"t1.ref".to_owned(), c"t2.ref".to_owned()];
    let b = NamesVec::new(same_content.iter().map(std::ffi::CString::as_c_str));
    assert!(a.as_ref().equals(b.as_ref()));
    assert_eq!(a.as_ref(), b.as_ref());

    let reordered = NamesVec::new([c"t2.ref", c"t1.ref"]);
    let prefix = NamesVec::new([c"t1.ref"]);
    let longer = NamesVec::new([c"t1.ref", c"t2.ref", c"t3.ref"]);
    let changed = NamesVec::new([c"t1.ref", c"t2.reF"]);
    let empty = NamesVec::new([]);
    for other in [&reordered, &prefix, &longer, &changed, &empty] {
        assert!(!a.as_ref().equals(other.as_ref()), "{other:?}");
        assert!(!other.as_ref().equals(a.as_ref()), "{other:?}");
        assert_ne!(a.as_ref(), other.as_ref());
    }
    assert_eq!(empty.as_ref(), NamesVec::new([]).as_ref());
    // Equal handles to one list.
    let r = a.as_ref();
    assert_eq!(r, r);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn alloc_array_allocates_uninitialised_runs_or_nothing() {
    let mut run = ReftableArrayFree::alloc_array::<u64>(3).unwrap();
    assert_eq!(run.len(), 3);
    for slot in &mut run {
        slot.write(10);
    }
    // `reftable_malloc(0)` returns NULL: an empty run allocates nothing.
    let empty = ReftableArrayFree::alloc_array::<u64>(0).unwrap();
    assert!(empty.is_empty());
    // `REFTABLE_ALLOC_ARRAY`'s overflow check refuses the request.
    assert!(ReftableArrayFree::alloc_array::<u64>(usize::MAX / 4).is_none());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn calloc_array_and_alloc_grow_are_the_array_owner_s_constructors() {
    let zeroed = ReftableArrayFree::new_zeroed::<u16>(4).unwrap();
    assert_eq!(&zeroed[..], [0; 4]);
    let mut run = ReftableArrayFree::alloc_array::<u8>(1).unwrap();
    run[0].write(7);
    ReftableArrayFree::grow(&mut run, 2).unwrap();
    // `alloc * 2 + 1`, at least `nr`.
    assert_eq!(run.len(), 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn grow_or_clear_grows_or_leaves_nothing_to_free() {
    let mut run = ReftableArrayFree::alloc_array::<u32>(2).unwrap();
    run[0].write(0xABCD);
    ReftableArrayFree::grow_or_clear(&mut run, 2).unwrap();
    assert_eq!(run.len(), 2, "already long enough");
    ReftableArrayFree::grow_or_clear(&mut run, 3).unwrap();
    assert_eq!(run.len(), 5);
    // A size that overflows fails, and the run is released and left empty.
    assert_eq!(
        ReftableArrayFree::grow_or_clear(&mut run, usize::MAX),
        Err(Error::OutOfMemory)
    );
    assert!(run.is_empty());
    // An empty run grows from nothing.
    ReftableArrayFree::grow_or_clear(&mut run, 1).unwrap();
    assert_eq!(run.len(), 1);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parsed_names_compare_with_names_built_in_rust() {
    use libgit2::libgit2::deps::reftable::basics::NamesVec;

    let parsed = parse_names(b"t1.ref\n\nt2.ref\n").unwrap();
    assert_eq!(parsed.as_ref().len(), 2);
    assert_eq!(
        parsed.as_ref(),
        NamesVec::new([c"t1.ref", c"t2.ref"]).as_ref()
    );
    assert_ne!(parsed.as_ref(), NamesVec::new([c"t1.ref"]).as_ref());
    let read: Vec<&CStr> = parsed.as_ref().iter().collect();
    assert_eq!(read, [c"t1.ref", c"t2.ref"]);
    let none = parse_names(b"").unwrap();
    assert!(none.as_ref().is_empty());
    assert_eq!(none.as_ref().len(), 0);
}
