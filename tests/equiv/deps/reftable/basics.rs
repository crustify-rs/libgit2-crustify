//! Equivalence tests for `deps/reftable/basics.c` against its C.
//!
//! The types' tests read the header's contract: `enum reftable_hash`'s constants,
//! `REFTABLE_BUF_INIT`, and the fields of `struct reftable_buf` as reftable's consumers read
//! them (`reftable_buf_size` returns `len`; the stack passes `buf` to `unlink` and
//! `tmpfile_rename` as a path). Each buffer is a C struct over Rust storage, in a state
//! `basics.c`'s routines leave one in, read once through its fields and once through a
//! handle; these call no C, so they also run under Miri.
//!
//! The routines' tests run the raw C and the safe API on independently owned inputs built
//! from one description, and compare after each step: a buffer through a sequence of
//! appends, trims, comparisons and detaches; `parse_names`, `binsearch` and `hash_size` on
//! the same inputs; and the allocator's routines on the same requests.
//!
//! The name lists are compared with `names_length` and `names_equal` called on raw arrays
//! built independently of the `NamesVec` the safe API reads, and on those `parse_names`
//! returns. The size constants are compared with their generated bindings. The big-endian
//! helpers and `alloc_size` are ported `static inline` functions of `basics.h`, which the
//! raw bindings do not reach: their tests are `unit_pub`'s, against the bytes the C bodies
//! write.

use std::cmp::Ordering;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::MaybeUninit;
use std::{ptr, slice};

use ffibox::{CRefSeam, CZeroable};
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{
    Buf, BufRef, BufVal, Error, HASH_SIZE_MAX, HASH_SIZE_SHA1, HASH_SIZE_SHA256, HashId, NamesVec,
    ReftableArrayFree, ReftableStr, ReftableVec, binsearch, parse_names,
};

#[test]
fn hash_id_names_exactly_the_constants_of_reftable_basics_h() {
    assert_eq!(HashId::Sha1.raw(), ffi::REFTABLE_HASH_SHA1);
    assert_eq!(HashId::Sha256.raw(), ffi::REFTABLE_HASH_SHA256);
    let named = [ffi::REFTABLE_HASH_SHA1, ffi::REFTABLE_HASH_SHA256];
    for raw in (0..=1024).chain([ffi::reftable_hash::MAX]) {
        assert_eq!(
            HashId::from_raw(raw).is_some(),
            named.contains(&raw),
            "{raw}"
        );
    }
}

#[test]
fn zeroed_buf_is_reftable_buf_init() {
    assert_eq!(size_of::<Buf>(), size_of::<ffi::reftable_buf>());
    assert_eq!(align_of::<Buf>(), align_of::<ffi::reftable_buf>());
    let value = Buf::zeroed();
    // SAFETY: the handle addresses `value`, a live `reftable_buf`, which is read once.
    let raw = unsafe { value.as_ref().as_ptr().read() };
    // `REFTABLE_BUF_INIT` is `{ 0 }`.
    assert_eq!(raw.alloc, 0);
    assert_eq!(raw.len, 0);
    assert!(raw.buf.is_null());
}

/// What a C consumer reads of `raw`'s fields, and what the handle's getters read: `len`,
/// `alloc`, the contents and, where they are terminated, the C string.
fn compare(raw: &mut ffi::reftable_buf) {
    let c_len = raw.len;
    let c_alloc = raw.alloc;
    let c_bytes: Vec<u8> = if raw.alloc == 0 {
        Vec::new()
    } else {
        // SAFETY: `raw.buf` addresses `raw.alloc > raw.len` bytes of live storage.
        unsafe { slice::from_raw_parts(raw.buf.cast::<u8>(), raw.len) }.to_vec()
    };
    let ptr: *mut ffi::reftable_buf = raw;
    // SAFETY: `raw` is a live `reftable_buf` in a state `basics.c` leaves, borrowed for
    // this call.
    let b = unsafe { BufRef::from_ptr(ptr) }.expect("non-null");
    assert_eq!(b.len(), c_len);
    assert_eq!(b.capacity(), c_alloc);
    assert_eq!(b.as_bytes(), c_bytes.as_slice());
    if c_alloc > 0 && !c_bytes.contains(&0) {
        // SAFETY: as above; `buf[len]` is within the allocation, and is NUL here.
        let c_str = unsafe { CStr::from_ptr(raw.buf) };
        assert_eq!(b.as_c_str(), Some(c_str));
    }
}

#[test]
fn getters_read_the_fields_as_c_consumers_do() {
    // `REFTABLE_BUF_INIT`, then a buffer `reftable_buf_addstr` filled with a table name,
    // then that buffer after `reftable_buf_reset` and after `reftable_buf_setlen`.
    let mut init = ffi::reftable_buf {
        alloc: 0,
        len: 0,
        buf: ptr::null_mut(),
    };
    compare(&mut init);

    // Each state over storage of its own: a buffer as `basics.c` leaves it.
    let name = *b"0x000000000001-0x000000000002-1a2b3c4d.ref\0\0\0\0\0";
    for (len, terminator_at) in [(42, None), (14, Some(14)), (0, Some(0))] {
        let mut storage = name;
        if let Some(i) = terminator_at {
            storage[i] = 0;
        }
        let mut raw = ffi::reftable_buf {
            alloc: storage.len(),
            len,
            buf: storage.as_mut_ptr().cast::<c_char>(),
        };
        compare(&mut raw);
    }
}

#[test]
fn contents_with_nul_bytes_read_whole() {
    // `reftable_buf_add` takes arbitrary bytes, such as a binary record key.
    let mut storage = [0x01, 0x00, b'r', 0xFF, 0x00, 0x00];
    let mut raw = ffi::reftable_buf {
        alloc: storage.len(),
        len: 4,
        buf: storage.as_mut_ptr().cast::<c_char>(),
    };
    compare(&mut raw);
    let ptr: *mut ffi::reftable_buf = &raw mut raw;
    // SAFETY: as in `compare`.
    let b = unsafe { BufRef::from_ptr(ptr) }.expect("non-null");
    assert_eq!(b.as_bytes(), [0x01, 0x00, b'r', 0xFF]);
    assert_eq!(b.as_c_str(), None, "a NUL comes before the terminator");
}

/// A C buffer, released by `reftable_buf_release` when dropped.
struct RawBuf(ffi::reftable_buf);

impl RawBuf {
    fn new() -> Self {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: `reftable_buf_init` writes the whole struct without reading it.
        unsafe { ffi::reftable_buf_init(raw.as_mut_ptr()) };
        // SAFETY: initialised above.
        Self(unsafe { raw.assume_init() })
    }

    fn bytes(&self) -> &[u8] {
        if self.0.alloc == 0 {
            return &[];
        }
        // SAFETY: an allocated buffer holds `len` bytes at `buf`, which it owns.
        unsafe { slice::from_raw_parts(self.0.buf.cast::<u8>(), self.0.len) }
    }
}

impl Drop for RawBuf {
    fn drop(&mut self) {
        // SAFETY: the struct owns `buf` (or nothing), released once here.
        unsafe { ffi::reftable_buf_release(&raw mut self.0) }
    }
}

/// One step of a buffer's life.
#[derive(Clone, Copy, Debug)]
enum Step {
    Add(&'static [u8]),
    AddStr(&'static CStr),
    SetLen(usize),
    Reset,
    Detach,
}

/// Applies `step` to the C buffer and to the Rust one, and compares what each reports and
/// leaves: the status, the detached string, the contents, the length and the allocation.
fn apply(step: Step, c: &mut RawBuf, rust: &mut BufVal) {
    match step {
        Step::Add(data) => {
            // SAFETY: `c` is a live buffer and `data` readable for its length.
            let rc =
                unsafe { ffi::reftable_buf_add(&raw mut c.0, data.as_ptr().cast(), data.len()) };
            assert_eq!(rust.as_mut().add(data).map_err(Error::code), status(rc));
        }
        Step::AddStr(s) => {
            // SAFETY: as above, with `s` NUL-terminated.
            let rc = unsafe { ffi::reftable_buf_addstr(&raw mut c.0, s.as_ptr()) };
            assert_eq!(rust.as_mut().add_str(s).map_err(Error::code), status(rc));
        }
        Step::SetLen(len) => {
            // SAFETY: `c` is a live buffer.
            let rc = unsafe { ffi::reftable_buf_setlen(&raw mut c.0, len) };
            assert_eq!(rust.as_mut().truncate(len).is_ok(), rc == 0, "{step:?}");
        }
        Step::Reset => {
            // SAFETY: as above.
            unsafe { ffi::reftable_buf_reset(&raw mut c.0) };
            rust.as_mut().reset();
        }
        Step::Detach => {
            // SAFETY: as above; the caller owns what it returns.
            let detached = unsafe { ffi::reftable_buf_detach(&raw mut c.0) };
            // SAFETY: NULL, or a NUL-terminated allocation of reftable's, now ours.
            let c_str = unsafe { ReftableStr::from_raw(detached) };
            let rust_str = rust.as_mut().detach();
            assert_eq!(
                rust_str.as_ref().map(ReftableStr::as_c_str),
                c_str.as_ref().map(ReftableStr::as_c_str)
            );
        }
    }
    let r = rust.as_ref();
    assert_eq!(r.as_bytes(), c.bytes(), "{step:?}");
    assert_eq!(r.len(), c.0.len);
    assert_eq!(r.capacity(), c.0.alloc, "the same growth");
}

fn status(rc: c_int) -> Result<(), c_int> {
    if rc == 0 { Ok(()) } else { Err(rc) }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn buffer_routines_match_c_step_by_step() {
    let steps = [
        Step::Detach,
        Step::SetLen(0),
        Step::SetLen(1),
        Step::Reset,
        Step::Add(b""),
        Step::Add(b"0x000000000001"),
        Step::AddStr(c"-0x000000000002"),
        Step::Add(b"\0\xFF"),
        Step::SetLen(40),
        Step::SetLen(14),
        Step::SetLen(14),
        Step::AddStr(c"-1a2b3c4d.ref"),
        Step::Detach,
        Step::AddStr(c""),
        Step::Add(&[b'x'; 200]),
        Step::Reset,
        Step::Add(b"tail"),
        Step::Detach,
        Step::Detach,
    ];
    let (mut c, mut rust) = (RawBuf::new(), Buf::new());
    for step in steps {
        apply(step, &mut c, &mut rust);
    }
}

/// A buffer of `bytes`, in C and in Rust.
fn pair(bytes: &[u8]) -> (RawBuf, BufVal) {
    let (mut c, mut rust) = (RawBuf::new(), Buf::new());
    if !bytes.is_empty() {
        // SAFETY: as in `apply`.
        let rc = unsafe { ffi::reftable_buf_add(&raw mut c.0, bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(rc, 0);
        rust.as_mut().add(bytes).unwrap();
    }
    (c, rust)
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn comparisons_match_c() {
    let inputs: [&[u8]; 8] = [b"", b"a", b"ab", b"abc", b"abd", b"b", b"\x80", b"a\0c"];
    let pairs: Vec<_> = inputs.iter().map(|bytes| pair(bytes)).collect();
    for (ca, ra) in &pairs {
        for (cb, rb) in &pairs {
            // SAFETY: both are live buffers, only read.
            let cmp = unsafe { ffi::reftable_buf_cmp(&raw const ca.0, &raw const cb.0) };
            assert_eq!(ra.as_ref().cmp(&rb.as_ref()), cmp.cmp(&0));
            assert_eq!(ra.as_ref() == rb.as_ref(), cmp == 0);
            let (mut a, mut b) = (ca.0, cb.0);
            // SAFETY: copies of live buffers' structs, only read.
            let prefix = unsafe { ffi::common_prefix_size(&raw mut a, &raw mut b) };
            assert_eq!(ra.as_ref().common_prefix_len(rb.as_ref()), prefix);
        }
    }
    let (_, empty) = pair(b"");
    assert_eq!(empty.as_ref().cmp(&Buf::zeroed().as_ref()), Ordering::Equal);
}

/// What C's `parse_names` gives for `input`: the names, or the error code.
fn c_parse_names(input: &[u8]) -> Result<Vec<Vec<u8>>, c_int> {
    let mut buf = input.to_vec();
    buf.push(0);
    let mut out: *mut *mut c_char = ptr::null_mut();
    let size = c_int::try_from(input.len()).unwrap();
    // SAFETY: `buf` holds `size` bytes and a NUL; `out` is written on success alone.
    let rc = unsafe { ffi::parse_names(buf.as_mut_ptr().cast(), size, &raw mut out) };
    if rc != 0 {
        assert!(out.is_null(), "untouched on failure");
        return Err(rc);
    }
    let mut names = Vec::new();
    for i in 0.. {
        // SAFETY: a NULL-terminated array, read up to its terminator.
        let slot = unsafe { out.add(i) };
        // SAFETY: as above; the slot is initialised.
        let name = unsafe { slot.read() };
        if name.is_null() {
            break;
        }
        // SAFETY: each element is a live NUL-terminated string.
        names.push(unsafe { CStr::from_ptr(name) }.to_bytes().to_vec());
    }
    // SAFETY: the array and its strings are ours, released once.
    unsafe { ffi::free_names(out) };
    Ok(names)
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parse_names_matches_c() {
    let inputs: [&[u8]; 11] = [
        b"",
        b"\n",
        b"a\n",
        b"a\nb\n",
        b"\n\na\n\nb\n\n",
        b"a",
        b"a\nb",
        b"a\0b\n",
        b"\0\n",
        b"0x000000000001-0x000000000001-1a2b3c4d.ref\n0x000000000002-0x000000000003-5e6f7a8b.ref\n",
        b"\xC3\xA9\n",
    ];
    for input in inputs {
        let rust = parse_names(input)
            .map(|names| {
                names
                    .iter()
                    .map(|n| n.to_bytes().to_vec())
                    .collect::<Vec<_>>()
            })
            .map_err(Error::code);
        assert_eq!(rust, c_parse_names(input), "{input:?}");
    }
}

/// The context C's `binsearch` passes back to [`c_less`]: the predicate's table, and where
/// it fails.
struct Table {
    first_true: usize,
    fails_at: Option<usize>,
    probed: Vec<usize>,
}

/// A C predicate over a [`Table`].
unsafe extern "C" fn c_less(k: usize, args: *mut c_void) -> c_int {
    // SAFETY: `args` is the `Table` the caller passed to `binsearch`, alive for the call and
    // reached by nothing else meanwhile.
    let table = unsafe { &mut *args.cast::<Table>() };
    table.probed.push(k);
    if table.fails_at == Some(k) {
        -1
    } else {
        c_int::from(k >= table.first_true)
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_matches_c() {
    for len in 0..24_usize {
        for first_true in 0..=len {
            for fails_at in [None, Some(0), Some(len / 2), Some(len.saturating_sub(1))] {
                let mut table = Table {
                    first_true,
                    fails_at,
                    probed: Vec::new(),
                };
                // SAFETY: `c_less` reads `table`, which outlives the call.
                let c = unsafe { ffi::binsearch(len, Some(c_less), (&raw mut table).cast()) };
                let mut probed = Vec::new();
                let rust = binsearch(len, |k| {
                    probed.push(k);
                    if fails_at == Some(k) {
                        Err(k)
                    } else {
                        Ok(k >= first_true)
                    }
                });
                assert_eq!(probed, table.probed, "the same probes");
                let failed = fails_at.is_some_and(|k| table.probed.contains(&k));
                match rust {
                    Ok(found) => {
                        assert!(!failed);
                        assert_eq!(found, c);
                    }
                    // C answers `len` for a failure, but for one at its last probe, of
                    // index 0, which it reads as found: the corrected divergence.
                    Err(k) => {
                        assert!(failed);
                        assert!(c == len || (k == 0 && c == 0), "C answered {c}");
                    }
                }
            }
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn hash_size_matches_c() {
    for id in [HashId::Sha1, HashId::Sha256] {
        // SAFETY: a value `hash_size`'s switch names.
        assert_eq!(id.size(), unsafe { ffi::hash_size(id.raw()) });
    }
    // reftable's unset id reads as SHA-1, which `HashId` leaves to its caller.
    // SAFETY: `hash_size` takes 0 too.
    assert_eq!(unsafe { ffi::hash_size(0) }, HashId::Sha1.size());
    assert_eq!(HashId::from_raw(0), None);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn allocator_routines_match_c() {
    // A `calloc` that overflows is NULL in C, `None` here.
    drop(ReftableStr::from_c_str(c"pins the runtime").unwrap());
    // SAFETY: `reftable_calloc` checks the product before it allocates.
    assert!(unsafe { ffi::reftable_calloc(usize::MAX, 2) }.is_null());
    assert!(ReftableArrayFree::new_zeroed::<u16>(usize::MAX).is_none());
    // SAFETY: as above, for an empty request, which C answers with NULL.
    assert!(unsafe { ffi::reftable_calloc(0, 8) }.is_null());
    assert!(ReftableArrayFree::new_zeroed::<u64>(0).unwrap().is_empty());
    // SAFETY: `reftable_malloc(0)` is NULL, as is `reftable_realloc(NULL, 0)`.
    assert!(unsafe { ffi::reftable_malloc(0) }.is_null());
    // SAFETY: as above.
    assert!(unsafe { ffi::reftable_realloc(ptr::null_mut(), 0) }.is_null());
    let mut run = ReftableVec::<u8>::new_uninit(0).unwrap();
    ReftableArrayFree::resize(&mut run, 0).unwrap();
    assert!(run.is_empty());

    // A copy of a string holds its bytes in either.
    // SAFETY: a NUL-terminated string, read for the call.
    let c = unsafe { ffi::reftable_strdup(c"ref".as_ptr()) };
    // SAFETY: NULL or a fresh copy, which the owner releases with `reftable_free`.
    let c = unsafe { ReftableStr::from_raw(c) }.unwrap();
    assert_eq!(c, ReftableStr::from_c_str(c"ref").unwrap());

    // A zeroed run is zero in either.
    // SAFETY: a fresh allocation of 4 zeroed `u32`s, or NULL.
    let c = unsafe { ffi::reftable_calloc(4, size_of::<u32>()) }.cast::<u32>();
    // SAFETY: as above, owned by the run.
    let c = unsafe { ReftableVec::from_raw_parts(c, 4) }.unwrap();
    assert_eq!(c, ReftableArrayFree::new_zeroed::<u32>(4).unwrap());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn grow_matches_reftable_alloc_grow() {
    // `REFTABLE_ALLOC_GROW`'s capacities, as C computes them inline, against `grow`'s.
    let mut capacity: usize = 0;
    let mut run = ReftableVec::<u64>::new_uninit(0).unwrap();
    for request in [1, 1, 2, 3, 4, 8, 9, 30, 31, 100] {
        if request > capacity {
            capacity = (capacity * 2 + 1).max(request);
        }
        ReftableArrayFree::grow(&mut run, request).unwrap();
        assert_eq!(run.len(), capacity, "request {request}");
    }
    // The bytes move with the run.
    let mut run = ReftableVec::<u32>::new_uninit(1).unwrap();
    run[0] = MaybeUninit::new(0xDEAD_BEEF);
    ReftableArrayFree::grow(&mut run, 2).unwrap();
    // SAFETY: element 0 was written above and kept by the resize.
    assert_eq!(unsafe { run[0].assume_init() }, 0xDEAD_BEEF);
}

#[test]
fn buf_init_is_reftable_buf_init() {
    let init = Buf::INIT;
    // SAFETY: the handle addresses `init`, a live `reftable_buf`, which is read once.
    let raw = unsafe { init.as_ref().as_ptr().read() };
    assert_eq!((raw.alloc, raw.len), (0, 0));
    assert!(raw.buf.is_null());
}

#[test]
fn hash_sizes_are_reftable_basics_h_constants() {
    assert_eq!(
        u32::try_from(HASH_SIZE_SHA1),
        Ok(ffi::REFTABLE_HASH_SIZE_SHA1)
    );
    assert_eq!(
        u32::try_from(HASH_SIZE_SHA256),
        Ok(ffi::REFTABLE_HASH_SIZE_SHA256)
    );
    assert_eq!(
        u32::try_from(HASH_SIZE_MAX),
        Ok(ffi::REFTABLE_HASH_SIZE_MAX)
    );
}

/// A raw NULL-terminated array over `names`, built apart from any `NamesVec`.
fn raw_names(names: &[CString]) -> Vec<*const c_char> {
    names
        .iter()
        .map(|n| n.as_ptr())
        .chain([ptr::null()])
        .collect()
}

fn owned(names: &[&str]) -> Vec<CString> {
    names.iter().map(|n| CString::new(*n).unwrap()).collect()
}

/// The lists a stack compares: its tables before and after a reload, and their edits.
const LISTS: &[&[&str]] = &[
    &[],
    &[""],
    &["0x000000000001-0x000000000001-11111111.ref"],
    &[
        "0x000000000001-0x000000000001-11111111.ref",
        "0x000000000002-0x000000000003-22222222.ref",
    ],
    &[
        "0x000000000002-0x000000000003-22222222.ref",
        "0x000000000001-0x000000000001-11111111.ref",
    ],
    &[
        "0x000000000001-0x000000000001-11111111.ref",
        "0x000000000002-0x000000000003-22222222.ref",
        "0x000000000004-0x000000000004-44444444.ref",
    ],
    &["a", "b", ""],
    &["a", "b"],
    &["a", "c"],
    &["\u{e9}t\u{e9}.ref"],
];

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn names_length_and_equal_match_the_c_on_raw_arrays() {
    for (i, a) in LISTS.iter().enumerate() {
        let a_c = owned(a);
        let mut a_raw = raw_names(&a_c);
        // SAFETY: `a_raw` is a NULL-terminated array of live strings, which `names_length`
        // only reads.
        let c_len = unsafe { ffi::names_length(a_raw.as_mut_ptr()) };
        let a_rust = owned(a);
        let a_list = NamesVec::new(a_rust.iter().map(CString::as_c_str));
        assert_eq!(a_list.as_ref().len(), c_len, "{a:?}");
        assert_eq!(c_len, a.len());

        for (j, b) in LISTS.iter().enumerate() {
            let b_c = owned(b);
            let mut b_raw = raw_names(&b_c);
            // SAFETY: both are NULL-terminated arrays of live strings, which `names_equal`
            // only reads.
            let c_equal = unsafe { ffi::names_equal(a_raw.as_mut_ptr(), b_raw.as_mut_ptr()) };
            let b_rust = owned(b);
            let b_list = NamesVec::new(b_rust.iter().map(CString::as_c_str));
            assert_eq!(
                a_list.as_ref().equals(b_list.as_ref()),
                c_equal != 0,
                "{a:?} {b:?}"
            );
            assert_eq!(c_equal != 0, i == j, "{a:?} {b:?}");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parsed_names_count_and_compare_as_the_c_does() {
    for (input, list) in [
        (&b"a\nb\n"[..], &["a", "b"][..]),
        (b"\n\nx\n", &["x"]),
        (b"", &[]),
    ] {
        let parsed = parse_names(input).unwrap();
        let c_names = owned(list);
        let mut raw = raw_names(&c_names);
        // SAFETY: `raw` is a NULL-terminated array of live strings, which C only reads.
        let c_len = unsafe { ffi::names_length(raw.as_mut_ptr()) };
        assert_eq!(parsed.as_ref().len(), c_len);
        let rust = NamesVec::new(c_names.iter().map(CString::as_c_str));
        assert!(parsed.as_ref().equals(rust.as_ref()), "{list:?}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn grow_or_clear_keeps_elements_as_reftable_alloc_grow_does() {
    // `REFTABLE_ALLOC_GROW_OR_NULL(x, 3, cap)` over a run of 2: `realloc` to `2 * 2 + 1`
    // elements, which keeps the first two.
    let mut run = ReftableArrayFree::alloc_array::<u32>(2).unwrap();
    run[0] = MaybeUninit::new(0xABCD);
    run[1] = MaybeUninit::new(0x1234);
    ReftableArrayFree::grow_or_clear(&mut run, 3).unwrap();
    assert_eq!(run.len(), 2 * 2 + 1);
    // SAFETY: element 0 was written above and kept by the growth.
    let first = unsafe { run[0].assume_init() };
    // SAFETY: as above, for element 1.
    let second = unsafe { run[1].assume_init() };
    assert_eq!((first, second), (0xABCD, 0x1234));
}
