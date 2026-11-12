//! Unit tests for `deps/reftable/basics.c`: `HashId`'s conversions, sizes and their error,
//! the buffer `Buf` through its owner and handles, the owners of reftable's allocator,
//! `parse_names`, `binsearch` and the error types, decided by Rust assertions.
//!
//! Every test that allocates or calls C is ignored under Miri, which cannot call libgit2.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::error::Error as _;
use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::panic;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{
    Buf, BufRef, BufVal, Error, HashId, LenOutOfRange, ReftableArrayFree, ReftableBox, ReftableStr,
    ReftableVec, UnnamedHashId, binsearch, parse_names,
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
    assert_eq!(HashId::Sha1.size(), 20);
    assert_eq!(HashId::Sha256.size(), 32);
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
