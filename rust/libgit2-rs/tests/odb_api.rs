//! The safe API of the object database, used as a Rust consumer would:
//! root items only, no `unsafe`, no init (the library initializes
//! itself). The mempack and Rust-backend tests run under Miri; the loose
//! and pack backends' need the file system and C (link-c).

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use libgit2_rs::{
    Backend, Error, ErrorClass, ErrorCode, ExpandId, LooseOptions, ObjectType, Odb, OdbBackend,
    Oid, OidType, Repository,
};

const HELLO: &str = "ce013625030ba8dba906f756967f9e9ca394464a";
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/resources");

fn oid(s: &str) -> Oid {
    Oid::from_hex(s, OidType::Sha1).unwrap()
}

static N: AtomicUsize = AtomicUsize::new(0);

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "odb-api-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn object_types() {
    assert_eq!(ObjectType::Commit.name(), "commit");
    assert_eq!(ObjectType::from_name(b"tree"), Some(ObjectType::Tree));
    assert_eq!(ObjectType::from_name("blob"), Some(ObjectType::Blob));
    assert_eq!(ObjectType::from_name(b"ta"), None);
    assert_eq!("tag".parse::<ObjectType>().unwrap(), ObjectType::Tag);
    assert_eq!(ObjectType::Tag.to_string(), "tag");
}

#[test]
fn hashing() -> Result<(), Error> {
    assert_eq!(
        Odb::hash(b"hello\n", ObjectType::Blob, OidType::Sha1)?,
        oid(HELLO)
    );
    assert_eq!(
        Odb::hash(b"", ObjectType::Tree, OidType::Sha1)?,
        oid(EMPTY_TREE)
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "SHA-256 is OpenSSL's")]
fn hashing_sha256() -> Result<(), Error> {
    assert_eq!(
        Odb::hash(b"hello\n", ObjectType::Blob, OidType::Sha256)?.to_string(),
        "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"
    );
    Ok(())
}

#[test]
fn mempack_round_trip() -> Result<(), Error> {
    let odb = Odb::new()?;
    let (backend, mempack) = Backend::mempack();
    odb.add_backend(backend, 1)?;
    assert_eq!(odb.num_backends(), 1);

    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(id, oid(HELLO));
    assert!(odb.exists(&id));
    assert!(odb.exists_no_refresh(&id));
    assert!(mempack.contains(&id));
    let obj = odb.read(&id)?;
    assert_eq!(obj.data(), b"hello\n");
    assert_eq!(obj.kind(), ObjectType::Blob);
    assert_eq!(obj.id(), &id);
    let obj2 = obj.clone();
    drop(obj);
    assert_eq!(obj2.len(), 6);
    assert_eq!(odb.read_header(&id)?, (6, ObjectType::Blob));

    // The hard-coded empty tree is always there.
    let t = odb.read(&oid(EMPTY_TREE))?;
    assert_eq!(t.kind(), ObjectType::Tree);
    assert!(t.is_empty());

    // Writing again does not duplicate.
    odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(mempack.object_count(), 1);
    let c = odb.write(b"abc", ObjectType::Commit)?;
    assert_eq!(c.to_string(), "3cffb60786e7da2208160c4f4b915999386b64a2");
    assert_eq!(mempack.ids()?.len(), 2);

    // Missing objects, with C's messages.
    let missing = oid("f6ea0495187600e7b2288c8ac19c5886383a4633");
    let e = odb.read(&missing).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.class(), ErrorClass::ODB);
    assert_eq!(
        e.message(),
        "object not found - no match for id (f6ea0495187600e7b2288c8ac19c5886383a4633)"
    );
    assert!(!odb.exists(&missing));
    let e = odb.read(&Oid::SHA1_ZERO).unwrap_err();
    assert_eq!(
        e.message(),
        "odb: cannot read object: null OID cannot exist"
    );

    // A mempack has no prefix lookup.
    let e = odb.read_prefix(&id, 7).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(
        e.message(),
        "object not found - no match for prefix (ce01362)"
    );
    let e = odb.read_prefix(&id, 3).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    assert_eq!(
        e.message(),
        "ambiguous OID prefix - prefix length too short"
    );

    drop(odb.read(&c)?);
    mempack.reset();
    assert_eq!(mempack.object_count(), 0);
    // Blobs are not cached; commits (under 4096 bytes) are.
    assert!(!odb.exists(&id));
    assert!(odb.exists(&c));
    Ok(())
}

#[test]
fn streams_through_a_write_only_backend() -> Result<(), Error> {
    let odb = Odb::new()?;
    let (backend, mempack) = Backend::mempack();
    odb.add_backend(backend, 1)?;
    let mut w = odb.writer(6, ObjectType::Blob)?;
    w.write_all(b"hel").unwrap();
    w.write_all(b"lo\n").unwrap();
    // Too much is refused.
    let e = w.write(b"!").unwrap_err();
    let e = e.into_inner().unwrap().downcast::<Error>().unwrap();
    assert_eq!(
        e.message(),
        "cannot stream_write() - Invalid length. 6 was expected. The total size of the received chunks amounts to 7."
    );
    drop(w);

    let mut w = odb.writer(6, ObjectType::Blob)?;
    w.write_all(b"hello\n").unwrap();
    assert_eq!(w.finalize()?, oid(HELLO));
    assert_eq!(mempack.object_count(), 1);

    let mut short = odb.writer(6, ObjectType::Blob)?;
    short.write_all(b"he").unwrap();
    let e = short.finalize().unwrap_err();
    assert!(
        e.message()
            .starts_with("cannot stream_finalize_write() - Invalid length.")
    );

    // No backend can stream reads.
    let e = odb.reader(&oid(HELLO)).unwrap_err();
    assert_eq!(
        e.message(),
        "cannot read object streamed - unsupported in the loaded odb backends"
    );
    Ok(())
}

/// A backend in Rust: a map, counting its calls.
#[derive(Default)]
struct MapState {
    objects: Mutex<Vec<(Oid, Vec<u8>, ObjectType)>>,
    reads: Mutex<usize>,
}

#[derive(Clone, Default)]
struct MapBackend(Arc<MapState>);

impl std::ops::Deref for MapBackend {
    type Target = MapState;
    fn deref(&self) -> &MapState {
        &self.0
    }
}

impl OdbBackend for MapBackend {
    fn read(&self, id: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
        *self.reads.lock().unwrap() += 1;
        let o = self.objects.lock().unwrap();
        o.iter()
            .find(|(i, _, _)| i == id)
            .map(|(_, d, t)| (d.clone(), *t))
            .ok_or_else(|| Error::new(ErrorCode::NOT_FOUND, ErrorClass::ODB, ""))
    }

    fn write(&self, id: &Oid, data: &[u8], kind: ObjectType) -> Result<(), Error> {
        self.objects
            .lock()
            .unwrap()
            .push((*id, data.to_vec(), kind));
        Ok(())
    }

    fn exists(&self, id: &Oid) -> bool {
        self.objects.lock().unwrap().iter().any(|(i, _, _)| i == id)
    }

    fn exists_prefix(&self, short: &Oid, len: usize) -> Result<Oid, Error> {
        let o = self.objects.lock().unwrap();
        let found: Vec<_> = o
            .iter()
            .filter(|(i, _, _)| i.ncmp(short, len).is_eq())
            .collect();
        match found.as_slice() {
            [] => Err(Error::new(ErrorCode::NOT_FOUND, ErrorClass::ODB, "")),
            [one] => Ok(one.0),
            _ => Err(Error::new(
                ErrorCode::AMBIGUOUS,
                ErrorClass::ODB,
                "ambiguous",
            )),
        }
    }

    fn foreach(&self, f: &mut dyn FnMut(&Oid) -> bool) -> Result<(), Error> {
        for (i, _, _) in self.objects.lock().unwrap().clone() {
            if !f(&i) {
                break;
            }
        }
        Ok(())
    }
}

#[test]
fn rust_backend() -> Result<(), Error> {
    let odb = Odb::new()?;
    let m = MapBackend::default();
    odb.add_backend(Backend::custom(m.clone()), 5)?;
    let a = odb.write(b"hello\n", ObjectType::Blob)?;
    let b = odb.write(b"world\n", ObjectType::Blob)?;
    let obj = odb.read(&a)?;
    assert_eq!(obj.data(), b"hello\n");
    // Blobs are not cached: a second read reaches the backend.
    drop(odb.read(&a)?);
    assert_eq!(*m.reads.lock().unwrap(), 2);
    // `read_header` falls back to reading.
    assert_eq!(odb.read_header(&b)?, (6, ObjectType::Blob));
    assert_eq!(odb.exists_prefix(&a, 7)?, a);

    let mut q = [
        ExpandId {
            id: a.prefix(8),
            length: 8,
            kind: None,
        },
        ExpandId {
            id: b,
            length: 40,
            kind: Some(ObjectType::Commit),
        },
    ];
    odb.expand_ids(&mut q)?;
    assert_eq!(q[0].id, a);
    assert_eq!(q[0].length, 40);
    assert_eq!(q[0].kind, Some(ObjectType::Blob));
    assert_eq!(q[1].length, 0);
    assert_eq!(q[1].kind, None);

    let mut seen = Vec::new();
    odb.foreach(|id| {
        seen.push(*id);
        true
    })?;
    assert_eq!(seen, vec![a, b]);
    let mut n = 0;
    odb.foreach(|_| {
        n += 1;
        false
    })?;
    assert_eq!(n, 1);
    Ok(())
}

/// A backend that implements nothing: every method has a default.
struct Nothing;
impl OdbBackend for Nothing {}

#[test]
fn backend_defaults() -> Result<(), Error> {
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Nothing), 2)?;
    let (mp, mempack) = Backend::mempack();
    odb.add_backend(mp, 1)?;
    // Not here: the read falls through to "not found".
    let e = odb.read(&oid(HELLO)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert!(!odb.exists(&oid(HELLO)));
    // Writes pass on to the mempack.
    assert_eq!(odb.write(b"hello\n", ObjectType::Blob)?, oid(HELLO));
    assert_eq!(mempack.object_count(), 1);
    odb.foreach(|_| true)?;
    odb.refresh()?;
    Ok(())
}

#[test]
fn get_backend_reaches_the_backends() -> Result<(), Error> {
    let odb = Odb::new()?;
    let m = MapBackend::default();
    odb.add_backend(Backend::custom(m.clone()), 1)?;
    let (mp, mempack) = Backend::mempack();
    odb.add_backend(mp, 5)?;
    // Priority order: the mempack first.
    let first = odb.get_backend(0)?;
    let got = first.as_mempack().expect("a mempack");
    assert!(first.downcast_ref::<MapBackend>().is_none());
    let second = odb.get_backend(1)?;
    assert!(second.as_mempack().is_none());
    let back = second
        .downcast_ref::<MapBackend>()
        .expect("the map backend");
    assert!(Arc::ptr_eq(&back.0, &m.0));
    assert!(second.downcast_ref::<Nothing>().is_none());

    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(got.object_count(), 1);
    assert_eq!(mempack.object_count(), 1);
    assert!(back.objects.lock().unwrap().is_empty());

    let e = odb.get_backend(2).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.message(), "no ODB backend loaded at index 2");

    // The handle outlives the odb's other owners' use.
    drop(mempack);
    got.reset();
    assert!(!odb.exists(&id));
    Ok(())
}

#[test]
fn corrupt_backend_data_is_detected() -> Result<(), Error> {
    struct Liar;
    impl OdbBackend for Liar {
        fn read(&self, _: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
            Ok((b"not hello".to_vec(), ObjectType::Blob))
        }
        fn exists(&self, _: &Oid) -> bool {
            true
        }
    }
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Liar), 1)?;
    let e = odb.read(&oid(HELLO)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::MISMATCH);
    assert!(e.message().starts_with(
        "object hash mismatch - expected ce013625030ba8dba906f756967f9e9ca394464a but got "
    ));
    Ok(())
}

#[test]
fn shared_between_threads() -> Result<(), Error> {
    let odb = Odb::new()?;
    let (backend, mempack) = Backend::mempack();
    odb.add_backend(backend, 1)?;
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let odb = odb.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    let data = format!("thread {t} object {i}\n");
                    let id = odb.write(data.as_bytes(), ObjectType::Blob).unwrap();
                    assert_eq!(odb.read(&id).unwrap().data(), data.as_bytes());
                    let c = odb.write(data.as_bytes(), ObjectType::Commit).unwrap();
                    assert_eq!(odb.read(&c).unwrap().kind(), ObjectType::Commit);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(mempack.object_count(), 160);
    Ok(())
}

// ---------------------------------------------------------------------------
// Misbehaving backends (review api-r1, S-3, S-6a/b/c/g)
// ---------------------------------------------------------------------------

/// A backend whose every read panics, and whose foreach ignores "stop".
struct Bad;
impl OdbBackend for Bad {
    fn read(&self, _: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
        panic!("backend read panics")
    }
    fn read_header(&self, _: &Oid) -> Result<(usize, ObjectType), Error> {
        panic!("header panics")
    }
    fn exists(&self, _: &Oid) -> bool {
        panic!("exists panics")
    }
    fn foreach(&self, f: &mut dyn FnMut(&Oid) -> bool) -> Result<(), Error> {
        // Ignores `false`: keeps calling.
        for i in 1..=5u8 {
            let _ = f(&oid(&format!("{i:02x}").repeat(20)));
        }
        Ok(())
    }
}

/// S-6c: a panicking backend gives an error of class INTERNAL.
#[test]
fn panicking_backend_is_an_internal_error() -> Result<(), Error> {
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Bad), 1)?;
    let id = oid(&"12".repeat(20));
    let e = odb.read(&id).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.class(), ErrorClass::INTERNAL);
    let e = odb.read_header(&id).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INTERNAL);
    // `exists` cannot fail in C: a panic is "not there".
    assert!(!odb.exists(&id));
    Ok(())
}

/// S-6b: a backend's foreach that ignores `false` does not call the
/// user's closure again.
#[test]
fn foreach_stop_is_latched() -> Result<(), Error> {
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Bad), 1)?;
    let mut n = 0;
    odb.foreach(|_| {
        n += 1;
        false
    })?;
    assert_eq!(n, 1);
    let mut all = 0;
    odb.foreach(|_| {
        all += 1;
        true
    })?;
    assert_eq!(all, 5);
    Ok(())
}

/// A panic in the user's foreach closure is resumed; the odb stays usable.
#[test]
fn foreach_closure_panic_is_resumed() -> Result<(), Error> {
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Bad), 1)?;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        odb.foreach(|_| panic!("user foreach panics"))
    }));
    assert!(r.is_err());
    odb.foreach(|_| true)?;
    Ok(())
}

/// S-6g: a success or ITER_OVER code in a backend's error is a failure.
#[test]
fn backend_error_codes_are_sanitized() -> Result<(), Error> {
    struct Codes(Mutex<i32>);
    impl OdbBackend for Codes {
        fn read(&self, _: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
            let c = *self.0.lock().unwrap();
            let code = if c == 0 {
                ErrorCode::ITER_OVER
            } else {
                ErrorCode::PASSTHROUGH
            };
            Err(Error::new(code, ErrorClass::ODB, "odd"))
        }
    }
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(Codes(Mutex::new(0))), 1)?;
    let e = odb.read(&oid(HELLO)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "odd");
    Ok(())
}

/// A backend that re-enters the odb it is in, through a `'static` clone.
struct Reentrant(Arc<OnceLock<Odb<'static>>>, AtomicBool);
impl OdbBackend for Reentrant {
    fn read(&self, id: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
        if !self.1.swap(true, Ordering::SeqCst)
            && let Some(o) = self.0.get()
        {
            // Re-entry: fails instead of deadlocking.
            let e = o.read(id).unwrap_err();
            assert_eq!(e.class(), ErrorClass::ODB);
            assert!(e.message().starts_with("failed to acquire the odb lock"));
        }
        Err(Error::new(ErrorCode::NOT_FOUND, ErrorClass::ODB, "x"))
    }
}

/// S-6a: a backend that reads its own odb gets an error, not a hang.
#[test]
fn reentrant_backend_does_not_hang() -> Result<(), Error> {
    let cell = Arc::new(OnceLock::new());
    let odb = Odb::new()?;
    odb.add_backend(
        Backend::custom(Reentrant(cell.clone(), AtomicBool::new(false))),
        1,
    )?;
    let _ = cell.set(odb.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let o = odb.clone();
    std::thread::spawn(move || {
        let r = o.read(&oid(&"21".repeat(20))).map(|_| ());
        let _ = tx.send(r.map_err(|e| e.code()));
    });
    let r = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the re-entrant read returns");
    assert_eq!(r, Err(ErrorCode::NOT_FOUND));
    Ok(())
}

struct PanicDrop;
impl Drop for PanicDrop {
    fn drop(&mut self) {
        panic!("backend Drop panics");
    }
}
struct DropPanics(#[allow(dead_code)] PanicDrop);
impl OdbBackend for DropPanics {}

/// S-3: a backend whose `Drop` panics, freed by its odb: no abort.
#[test]
fn backend_drop_panic_added() -> Result<(), Error> {
    let odb = Odb::new()?;
    odb.add_backend(Backend::custom(DropPanics(PanicDrop)), 1)?;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(odb)));
    assert!(r.is_ok(), "the panic does not escape the drop");
    Ok(())
}

/// S-3: the same, for a backend never added.
#[test]
fn backend_drop_panic_never_added() {
    let b = Backend::custom(DropPanics(PanicDrop));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(b)));
    assert!(r.is_ok());
}

// ---------------------------------------------------------------------------
// On disk
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn loose_backend_on_disk() -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir("loose");
    let odb = Odb::new()?;
    odb.add_backend(Backend::loose(&dir, LooseOptions::new())?, 1)?;
    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    let path = dir.join(&HELLO[..2]).join(&HELLO[2..]);
    let bytes = std::fs::read(&path).unwrap();
    // zlib level 1 (Z_BEST_SPEED) by default, as C.
    assert_eq!(&bytes[..2], &[0x78, 0x01]);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o444
    );
    assert_eq!(odb.read(&id)?.data(), b"hello\n");
    assert_eq!(odb.read_prefix(&id, 6)?.data(), b"hello\n");
    assert_eq!(odb.exists_prefix(&id, 5)?, id);
    assert_eq!(odb.read_header(&id)?, (6, ObjectType::Blob));

    // Streams.
    let big: Vec<u8> = (0..100_000u32).flat_map(|i| i.to_le_bytes()).collect();
    let mut w = odb.writer(big.len() as u64, ObjectType::Blob)?;
    for c in big.chunks(7777) {
        w.write_all(c).unwrap();
    }
    let bid = w.finalize()?;
    assert_eq!(bid, Odb::hash(&big, ObjectType::Blob, OidType::Sha1)?);
    let (mut r, len, kind) = odb.reader(&bid)?;
    assert_eq!((len, kind), (big.len(), ObjectType::Blob));
    let mut back = Vec::new();
    r.read_to_end(&mut back).unwrap();
    assert_eq!(back, big);
    drop(r);

    let mut ids = Vec::new();
    odb.foreach(|i| {
        ids.push(*i);
        true
    })?;
    ids.sort();
    let mut want = vec![id, bid];
    want.sort();
    assert_eq!(ids, want);

    // A corrupt file.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&path, b"\x78\x01garbage").unwrap();
    let odb2 = Odb::new()?;
    odb2.add_backend(Backend::loose(dir.as_path(), LooseOptions::default())?, 1)?;
    let e = odb2.read(&id).unwrap_err();
    assert_eq!(e.class(), ErrorClass::ZLIB);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn loose_options() -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir("loose-opts");
    let odb = Odb::with_oid_type(OidType::Sha256)?;
    let opts = LooseOptions::new()
        .oid_type(OidType::Sha256)
        .compression_level(42)
        .fsync(true)
        .dir_mode(0o700)
        .file_mode(0o400);
    odb.add_backend(Backend::loose(&dir, opts)?, 1)?;
    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(
        id.to_string(),
        "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"
    );
    let s = id.to_string();
    let path = dir.join(&s[..2]).join(&s[2..]);
    let bytes = std::fs::read(&path).unwrap();
    // Level 9 (more is 9): Z_BEST_COMPRESSION.
    assert_eq!(&bytes[..2], &[0x78, 0xda]);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o400);
    assert_eq!(mode(&dir.join(&s[..2])), 0o700);
    assert_eq!(odb.read(&id)?.data(), b"hello\n");
    assert_eq!(odb.read_prefix(&id, 10)?.id(), &id);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn paths_with_nul_are_refused() {
    let e = Odb::open("a\0b", OidType::Sha1).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert!(Backend::loose("a\0b", LooseOptions::new()).is_err());
    assert!(Backend::pack("a\0b", OidType::Sha1).is_err());
    assert!(Backend::one_pack("a\0b.idx", OidType::Sha1).is_err());
    let odb = Odb::new().unwrap();
    assert!(odb.add_disk_alternate("a\0b").is_err());
}

fn copy_packs(to: &Path) {
    let pack_dir = to.join("pack");
    std::fs::create_dir_all(&pack_dir).unwrap();
    let src = format!("{RES}/testrepo.git/objects/pack");
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().into_string().unwrap();
        if n.ends_with(".idx") || n.ends_with(".pack") {
            std::fs::copy(e.path(), pack_dir.join(&n)).unwrap();
        }
    }
}

/// Every object of an odb, read back and hashed again.
fn check_all(odb: &Odb<'_>) -> usize {
    let mut ids = Vec::new();
    odb.foreach(|id| {
        ids.push(*id);
        true
    })
    .unwrap();
    ids.sort();
    ids.dedup();
    // testrepo.git's loose object that is malformed on purpose
    // (`tests/libgit2/object/lookup.c`).
    let invalid = oid("bf3d798a12d2bd09a73b1bbdec6fde6f9836f9ac");
    if let Some(p) = ids.iter().position(|i| *i == invalid) {
        let e = odb.read(&invalid).unwrap_err();
        assert_eq!(
            e.message(),
            "malformed object: body was longer than specified in header"
        );
        ids.remove(p);
    }
    for id in &ids {
        let o = odb.read(id).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert_eq!(Odb::hash(o.data(), o.kind(), OidType::Sha1).unwrap(), *id);
        assert_eq!(odb.read_header(id).unwrap(), (o.len(), o.kind()));
    }
    ids.len()
}

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn pack_backends() -> Result<(), Error> {
    let objects = format!("{RES}/testrepo.git/objects");
    let odb = Odb::new()?;
    odb.add_backend(Backend::pack(&objects, OidType::Sha1)?, 1)?;
    assert!(check_all(&odb) > 1600);
    // A prefix that matches many objects.
    let e = odb.read_prefix(&oid(HELLO), 1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);

    // One pack: only its objects.
    let idx = format!(
        "{RES}/testrepo.git/objects/pack/pack-d7c6adf9f61318f041845b01440d09aa7a91e1b5.idx"
    );
    let one = Odb::new()?;
    one.add_backend(Backend::one_pack(&idx, OidType::Sha1)?, 1)?;
    let n = check_all(&one);
    assert!(n > 0 && n < 20, "{n}");
    // C's -1 (the pack's "not found" is not passed on).
    let e = Backend::one_pack("/nonexistent/pack-x.idx", OidType::Sha1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);

    // A missing directory is an empty backend.
    let empty = Odb::new()?;
    empty.add_backend(Backend::pack("/nonexistent", OidType::Sha1)?, 1)?;
    assert_eq!(check_all(&empty), 0);

    // `open` adds the loose and pack backends.
    let opened = Odb::open(&objects, OidType::Sha1)?;
    assert_eq!(opened.num_backends(), 2);
    assert!(check_all(&opened) > 1600);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn alternates() -> Result<(), Error> {
    let dir = tempdir("alt");
    let odb = Odb::open(&dir, OidType::Sha1)?;
    let head = oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
    assert!(!odb.exists(&head));
    odb.add_disk_alternate(format!("{RES}/testrepo.git/objects"))?;
    assert_eq!(odb.num_backends(), 4);
    assert_eq!(odb.read(&head)?.kind(), ObjectType::Commit);
    // Alternates are not written to.
    let id = odb.write(b"alt\n", ObjectType::Blob)?;
    let s = id.to_string();
    assert!(dir.join(&s[..2]).join(&s[2..]).exists());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "file system and C (the multi-pack-index writer)")]
fn write_a_multi_pack_index() -> Result<(), Error> {
    let tmp = tempdir("midx");
    copy_packs(&tmp);
    let odb = Odb::open(&tmp, OidType::Sha1)?;
    odb.write_multi_pack_index()?;
    assert!(tmp.join("pack/multi-pack-index").exists());
    assert!(check_all(&odb) > 1600);
    // Without a pack backend, nothing can write one.
    let mem = Odb::new()?;
    let e = mem.write_multi_pack_index().unwrap_err();
    assert_eq!(
        e.message(),
        "cannot write multi-pack-index - unsupported in the loaded odb backends"
    );
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "file system and C (the commit-graph)")]
fn commit_graph() -> Result<(), Error> {
    let objects = PathBuf::from(format!("{RES}/testrepo.git/objects"));
    let odb = Odb::open(&objects, OidType::Sha1)?;
    odb.set_commit_graph(Some(&objects))?;
    assert!(odb.exists(&oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750")));
    odb.set_commit_graph(None)?;
    assert!(odb.set_commit_graph(Some(Path::new("a\0b"))).is_err());
    Ok(())
}

/// A tree of one blob `f`, in the tree format.
fn tree_of(blob: &Oid) -> Vec<u8> {
    let mut t = b"100644 f\0".to_vec();
    t.extend_from_slice(blob.as_bytes());
    t
}

/// Mempack dump (packbuilder), then that pack written into another
/// repository's odb (indexer): the objects arrive.
#[test]
#[cfg_attr(miri, ignore = "file system and C (packbuilder, indexer)")]
fn mempack_dump_and_write_pack() -> Result<(), Error> {
    let d1 = tempdir("dump-src");
    let repo = Repository::init(&d1, true)?;
    let (mp, mempack) = Backend::mempack();
    repo.odb()?.add_backend(mp, 1000)?;
    let odb = repo.odb()?;
    let blob = odb.write(b"hello\n", ObjectType::Blob)?;
    let tree = odb.write(&tree_of(&blob), ObjectType::Tree)?;
    let commit_text =
        format!("tree {tree}\nauthor a <a@b> 0 +0000\ncommitter a <a@b> 0 +0000\n\nmsg\n");
    let commit = odb.write(commit_text.as_bytes(), ObjectType::Commit)?;
    assert_eq!(mempack.object_count(), 3);
    // The mempack, found again through the odb.
    let again = odb.get_backend(0)?.as_mempack().expect("the mempack");
    assert_eq!(again.object_count(), 3);

    let pack = again.dump(&repo)?;
    assert_eq!(&pack[..4], b"PACK");

    let d2 = tempdir("dump-dst");
    let repo2 = Repository::init(&d2, true)?;
    let odb2 = repo2.odb()?;
    assert!(!odb2.exists(&commit));
    let mut w = odb2.write_pack()?;
    for chunk in pack.chunks(10) {
        w.write_all(chunk).unwrap();
    }
    // The indexer read the header (`received_bytes` is the transport's).
    assert_eq!(w.progress().total_objects, 3);
    let p = w.commit()?;
    assert_eq!(p.total_objects, 3);
    assert_eq!(p.indexed_objects, 3);
    odb2.refresh()?;
    for id in [blob, tree, commit] {
        assert!(odb2.exists(&id), "{id}");
    }
    assert_eq!(odb2.read(&blob)?.data(), b"hello\n");

    // A writer dropped before its commit leaves nothing.
    let d3 = tempdir("dump-drop");
    let repo3 = Repository::init(&d3, true)?;
    let odb3 = repo3.odb()?;
    let mut w = odb3.write_pack()?;
    w.write_all(&pack).unwrap();
    drop(w);
    odb3.refresh()?;
    assert!(!odb3.exists(&commit));

    // Garbage is not a pack.
    let mut w = odb3.write_pack()?;
    let r = w.write_all(b"not a pack at all, not at all");
    assert!(r.is_err() || w.commit().is_err());

    // An odb without a pack backend cannot take one.
    let e = Odb::new()?.write_pack().unwrap_err();
    assert_eq!(
        e.message(),
        "cannot write pack - unsupported in the loaded odb backends"
    );
    for d in [d1, d2, d3] {
        let _ = std::fs::remove_dir_all(d);
    }
    Ok(())
}
