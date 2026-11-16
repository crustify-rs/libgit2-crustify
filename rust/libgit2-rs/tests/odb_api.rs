//! The safe API of the object database (WP l-odb), used as a Rust consumer
//! would: no `unsafe`. The mempack and Rust-backend tests run under Miri;
//! the loose backend's need the file system and C (link-c).

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use libgit2_rs::util::test_support;
use libgit2_rs::{
    Backend, Error, ErrorClass, ErrorCode, ExpandId, LooseOptions, ObjectType, Odb, OdbBackend,
    Oid, OidType,
};

const HELLO: &str = "ce013625030ba8dba906f756967f9e9ca394464a";
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn oid(s: &str) -> Oid {
    Oid::from_str_hex(s, OidType::Sha1).unwrap()
}

#[test]
fn object_types() {
    assert_eq!(ObjectType::from_raw(3), Some(ObjectType::Blob));
    assert_eq!(ObjectType::from_raw(5), None);
    assert_eq!(ObjectType::Commit.name(), "commit");
    assert_eq!(ObjectType::OfsDelta.name(), "");
    assert_eq!(ObjectType::from_name(b"tree"), Some(ObjectType::Tree));
    assert_eq!(ObjectType::from_name(b"tagx"), Some(ObjectType::Tag));
    assert_eq!(ObjectType::from_name(b"ta"), None);
    assert!(ObjectType::Blob.is_loose());
    assert!(!ObjectType::RefDelta.is_loose());
}

#[test]
fn hashing() -> Result<(), Error> {
    let _g = test_support::init();
    assert_eq!(
        Odb::hash(b"hello\n", ObjectType::Blob, OidType::Sha1)?,
        oid(HELLO)
    );
    let e = Odb::hash(b"xx", ObjectType::OfsDelta, OidType::Sha1).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(e.message(), "invalid object type");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "SHA-256 is OpenSSL's")]
fn hashing_sha256() -> Result<(), Error> {
    let _g = test_support::init();
    assert_eq!(
        Odb::hash(b"hello\n", ObjectType::Blob, OidType::Sha256)?.to_string(),
        "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"
    );
    Ok(())
}

#[test]
fn mempack_round_trip() -> Result<(), Error> {
    let _g = test_support::init();
    let odb = Odb::new()?;
    let (backend, mempack) = Backend::mempack();
    odb.add_backend(backend, 1)?;
    assert_eq!(odb.num_backends(), 1);

    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(id, oid(HELLO));
    assert!(odb.exists(&id));
    assert!(mempack.contains(&id));
    let obj = odb.read(&id)?;
    assert_eq!(obj.data(), b"hello\n");
    assert_eq!(obj.kind(), Some(ObjectType::Blob));
    assert_eq!(obj.id(), &id);
    let obj2 = obj.clone();
    drop(obj);
    assert_eq!(obj2.len(), 6);
    assert_eq!(odb.read_header(&id)?, (6, Some(ObjectType::Blob)));

    // The hard-coded empty tree is always there.
    let t = odb.read(&oid(EMPTY_TREE))?;
    assert_eq!(t.kind(), Some(ObjectType::Tree));
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
    let _g = test_support::init();
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
            .filter(|(i, _, _)| i.ncmp(short, len) == 0)
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
    let _g = test_support::init();
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
    assert_eq!(odb.read_header(&b)?, (6, Some(ObjectType::Blob)));
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

#[test]
fn corrupt_backend_data_is_detected() -> Result<(), Error> {
    struct Liar;
    impl OdbBackend for Liar {
        fn read(&self, _: &Oid) -> Result<(Vec<u8>, ObjectType), Error> {
            Ok((b"not hello".to_vec(), ObjectType::Blob))
        }
        fn write(&self, _: &Oid, _: &[u8], _: ObjectType) -> Result<(), Error> {
            Ok(())
        }
        fn exists(&self, _: &Oid) -> bool {
            true
        }
    }
    let _g = test_support::init();
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
    let _g = test_support::init();
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
                    assert_eq!(odb.read(&c).unwrap().kind(), Some(ObjectType::Commit));
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

fn tempdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("l-odb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn loose_backend_on_disk() -> Result<(), Error> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let _g = test_support::init();
    let dir = tempdir("loose");
    let odb = Odb::new()?;
    let opts = LooseOptions::default();
    odb.add_backend(Backend::loose(dir.as_os_str().as_bytes(), &opts)?, 1)?;
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
    assert_eq!(odb.read_header(&id)?, (6, Some(ObjectType::Blob)));

    // Streams.
    let big: Vec<u8> = (0..100_000u32).flat_map(|i| i.to_le_bytes()).collect();
    let mut w = odb.writer(big.len() as u64, ObjectType::Blob)?;
    for c in big.chunks(7777) {
        w.write_all(c).unwrap();
    }
    let bid = w.finalize()?;
    assert_eq!(bid, Odb::hash(&big, ObjectType::Blob, OidType::Sha1)?);
    let (mut r, len, kind) = odb.reader(&bid)?;
    assert_eq!((len, kind), (big.len(), Some(ObjectType::Blob)));
    let mut back = Vec::new();
    let mut buf = [0u8; 1000];
    loop {
        let n = r.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        back.extend_from_slice(&buf[..n]);
    }
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
    odb2.add_backend(Backend::loose(dir.as_os_str().as_bytes(), &opts)?, 1)?;
    let e = odb2.read(&id).unwrap_err();
    assert_eq!(e.class(), ErrorClass::ZLIB);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "file system and mmap")]
fn loose_sha256() -> Result<(), Error> {
    use std::os::unix::ffi::OsStrExt;
    let _g = test_support::init();
    let dir = tempdir("loose256");
    let odb = Odb::with_oid_type(OidType::Sha256)?;
    let opts = LooseOptions {
        oid_type: OidType::Sha256,
        compression_level: Some(9),
        ..LooseOptions::default()
    };
    odb.add_backend(Backend::loose(dir.as_os_str().as_bytes(), &opts)?, 1)?;
    let id = odb.write(b"hello\n", ObjectType::Blob)?;
    assert_eq!(
        id.to_string(),
        "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"
    );
    let s = id.to_string();
    let bytes = std::fs::read(dir.join(&s[..2]).join(&s[2..])).unwrap();
    assert_eq!(&bytes[..2], &[0x78, 0xda]);
    assert_eq!(odb.read(&id)?.data(), b"hello\n");
    assert_eq!(odb.read_prefix(&id, 10)?.id(), &id);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
