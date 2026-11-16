//! The safe Rust API of the objects component (`Object`, `Blob`, `Tag`,
//! `Signature`, `Time`, grafts), used without `unsafe` (PORTING.md §5), on
//! copies of the clar fixtures.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use libgit2_rs::grafts::Grafts;
use libgit2_rs::object::Repository;
use libgit2_rs::util::test_support;
use libgit2_rs::{Blob, ErrorClass, ErrorCode, Object, ObjectType, Oid, OidType, Signature, Tag, Time};

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

/// A scratch copy of a fixture repository.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, fixture: &str) -> Scratch {
        let d = std::env::temp_dir().join(format!("l_obj_api-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        copy_dir(&resources().join(fixture), &d);
        Scratch(d)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn oid(s: &str) -> Oid {
    Oid::from_str_hex(s, OidType::Sha1).unwrap()
}

const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
const TAG: &str = "7b4384978d2493e851f9cca7858815fac9b10980";

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn lookup_peel_and_ids() {
    let _g = test_support::init();
    let repo = Repository::open(&resources().join("testrepo.git")).unwrap();
    let c = Object::lookup(&repo, &oid(COMMIT), ObjectType::Any).unwrap();
    assert_eq!(c.kind(), ObjectType::Commit);
    assert_eq!(c.id(), &oid(COMMIT));
    let c2 = c.clone();
    assert_eq!(c2.id(), c.id());

    // The cache returns the same object; a wrong type is GIT_ENOTFOUND.
    let e = Object::lookup(&repo, &oid(COMMIT), ObjectType::Blob).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.message_bytes(), b"the requested type does not match the type in the ODB");

    let t = c.peel(ObjectType::Tree).unwrap();
    assert_eq!(t.kind(), ObjectType::Tree);
    assert_eq!(t.id(), &oid(TREE));
    let e = t.peel(ObjectType::Commit).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.class(), ErrorClass::OBJECT);

    // Short ids and prefixes.
    let short = c.short_id().unwrap();
    assert_eq!(short, b"a65fedf");
    let p = Object::lookup_prefix(&repo, &oid(COMMIT), 7, ObjectType::Commit).unwrap();
    assert_eq!(p.id(), &oid(COMMIT));
    let e = Object::lookup_prefix(&repo, &oid(COMMIT), 3, ObjectType::Any).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);

    // By path.
    let readme = c.lookup_bypath(b"README", ObjectType::Blob).unwrap();
    assert_eq!(readme.kind(), ObjectType::Blob);
    let e = c.lookup_bypath(b"README", ObjectType::Tree).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.message_bytes(), b"object at path 'README' is not of the asked-for type 2");

    // Conversions.
    assert!(c.clone().into_blob().is_err());
    let b = readme.into_blob().unwrap();
    assert!(!b.content().is_empty());
    assert_eq!(b.size(), b.content().len() as u64);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn blobs() {
    let _g = test_support::init();
    let s = Scratch::new("blobs", "testrepo.git");
    let repo = Repository::open(&s.0).unwrap();
    let b = Blob::lookup(&repo, &oid(BLOB)).unwrap();
    assert_eq!(b.content(), b"hey\n");
    assert!(!b.is_binary());
    assert_eq!(
        b.filter(b"file.txt", &Default::default()).unwrap().unwrap(),
        b"hey\n"
    );

    let id = Blob::create_from_buffer(&repo, b"hello\0world").unwrap();
    let nb = Blob::lookup(&repo, &id).unwrap();
    assert!(nb.is_binary());
    assert_eq!(nb.filter(b"x", &Default::default()).unwrap(), None);
    assert_eq!(
        nb.filter(b"x", &libgit2_rs::blob::BlobFilterOptions::default().check_for_binary(false))
            .unwrap()
            .unwrap(),
        b"hello\0world"
    );
    assert_eq!(
        id,
        libgit2_rs::object::id_from_buffer(b"hello\0world", ObjectType::Blob, OidType::Sha1).unwrap()
    );

    let mut w = Blob::writer(&repo, Some(b"streamed.txt")).unwrap();
    w.write(b"from a ").unwrap();
    w.write(b"stream\n").unwrap();
    let sid = w.commit().unwrap();
    assert_eq!(Blob::lookup(&repo, &sid).unwrap().content(), b"from a stream\n");
    // A writer dropped without commit leaves nothing behind.
    let mut w = Blob::writer(&repo, None).unwrap();
    w.write(b"abandoned").unwrap();
    drop(w);
    let leftovers: Vec<_> = std::fs::read_dir(s.0.join("objects"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("streamed"))
        .collect();
    assert!(leftovers.is_empty());

    let f = s.0.join("on_disk.txt");
    std::fs::write(&f, b"disk\n").unwrap();
    let did = Blob::create_from_disk(&repo, &f).unwrap();
    assert_eq!(Blob::lookup(&repo, &did).unwrap().content(), b"disk\n");
    let e = Blob::create_from_disk(&repo, &s.0.join("objects")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::DIRECTORY);
    assert!(libgit2_rs::blob::data_is_binary(b"\0"));
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn tags() {
    let _g = test_support::init();
    let s = Scratch::new("tags", "testrepo.git");
    let repo = Repository::open(&s.0).unwrap();
    let t = Tag::lookup(&repo, &oid(TAG)).unwrap();
    assert_eq!(t.name(), b"e90810b");
    assert_eq!(t.message(), Some(&b"This is a very simple tag.\n"[..]));
    let tagger = t.tagger().unwrap();
    assert_eq!(tagger.name(), b"Vicent Marti");
    assert_eq!(tagger.when(), Time::new(1281578357, 120));
    assert_eq!(t.target_type(), ObjectType::Commit);
    let target = t.target().unwrap();
    assert_eq!(target.id(), t.target_id());
    assert_eq!(t.peel().unwrap().kind(), ObjectType::Commit);

    let mut names = Tag::list(&repo, None).unwrap();
    names.sort();
    assert!(names.contains(&b"e90810b".to_vec()) && names.contains(&b"point_to_blob".to_vec()));
    assert_eq!(Tag::list(&repo, Some(b"e9*")).unwrap(), vec![b"e90810b".to_vec()]);
    let mut seen = 0;
    Tag::foreach(&repo, |name, _| {
        assert!(name.starts_with(b"refs/tags/"));
        seen += 1;
        0
    })
    .unwrap();
    assert_eq!(seen, names.len());
    let e = Tag::foreach(&repo, |_, _| 42).unwrap_err();
    assert_eq!(e.code().raw(), 42);

    let sig = Signature::new(b"Tagger", b"tagger@example.com", Time::new(123456789, 60)).unwrap();
    let id = Tag::create(&repo, b"the-tag", &target, &sig, b"a message\n", false).unwrap();
    let nt = Tag::lookup(&repo, &id).unwrap();
    assert_eq!(nt.name(), b"the-tag");
    assert_eq!(nt.tagger().unwrap(), &sig);
    let e = Tag::create(&repo, b"the-tag", &target, &sig, b"again", false).unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    assert_eq!(e.message_bytes(), b"tag already exists");
    let e = Tag::create(&repo, b"-bad", &target, &sig, b"m", false).unwrap_err();
    assert_eq!(e.message_bytes(), b"'-bad' is not a valid tag name");

    let lw = Tag::create_lightweight(&repo, b"light", &target, false).unwrap();
    assert_eq!(&lw, target.id());
    Tag::delete(&repo, b"light").unwrap();
    assert_eq!(Tag::delete(&repo, b"light").unwrap_err().code(), ErrorCode::NOT_FOUND);

    let text = format!(
        "object {COMMIT}\ntype commit\ntag from-buffer\ntagger A <a@b> 1 +0000\n\nmsg\n"
    );
    let bid = Tag::create_from_buffer(&repo, text.as_bytes(), false).unwrap();
    assert_eq!(Tag::lookup(&repo, &bid).unwrap().name(), b"from-buffer");
    let bad = format!("object {COMMIT}\ntype tree\ntag x\n\nmsg\n");
    let e = Tag::create_from_buffer(&repo, bad.as_bytes(), false).unwrap_err();
    assert_eq!(e.message_bytes(), b"the type for the given target is invalid");

    assert!(Tag::name_is_valid(b"v1.0").unwrap());
    assert!(!Tag::name_is_valid(b"HEAD").unwrap());
    assert!(!Tag::name_is_valid(b"a..b").unwrap());

    let annotation = Tag::annotation_create(&repo, b"loose", &target, &sig, b"m").unwrap();
    assert_eq!(Tag::lookup(&repo, &annotation).unwrap().name(), b"loose");
}

#[test]
fn signatures() {
    let _g = test_support::init();
    let s = Signature::from_buffer(b"Name <e@mail> 1600000000 -0130").unwrap();
    assert_eq!(s.name(), b"Name");
    assert_eq!(s.email(), b"e@mail");
    assert_eq!(s.when().seconds(), 1600000000);
    assert_eq!(s.when().offset_minutes(), -90);
    assert_eq!(s.when().sign(), b'-');
    assert_eq!(s.to_header(b"author "), b"author Name <e@mail> 1600000000 -0130\n");
    assert_eq!(s.to_string(), "Name <e@mail>");
    let e = Signature::from_buffer(b"no email here").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(e.message_bytes(), b"failed to parse signature - malformed e-mail");
    let e = Signature::new(b"a<b", b"c", Time::new(0, 0)).unwrap_err();
    assert_eq!(
        e.message_bytes(),
        b"failed to parse signature - Neither `name` nor `email` should contain angle brackets chars."
    );
    assert!(Signature::new(b"a\0b", b"c", Time::new(0, 0)).is_err());
    let c = s.clone();
    assert_eq!(c, s);
}

#[test]
#[cfg_attr(miri, ignore = "calls the C library's mktime")]
fn signature_now() {
    let _g = test_support::init();
    let now = Signature::now(b"N", b"E").unwrap();
    assert!(now.when().seconds() > 1_600_000_000);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn default_signatures() {
    let _g = test_support::init();
    let s = Scratch::new("defsig", "testrepo.git");
    let cfg = s.0.join("config");
    let mut text = std::fs::read(&cfg).unwrap();
    text.extend_from_slice(b"[user]\n\tname = The User\n\temail = user@example.com\n");
    std::fs::write(&cfg, text).unwrap();
    let repo = Repository::open(&s.0).unwrap();
    let sig = Signature::default_for(&repo).unwrap();
    assert_eq!(sig.name(), b"The User");
    assert_eq!(sig.email(), b"user@example.com");
    // (The environment is the test process's: only checked to succeed.)
    let (a, c) = Signature::default_from_env(&repo).unwrap();
    assert!(!a.name().is_empty() && !c.email().is_empty());
}

#[test]
fn grafts() {
    let _g = test_support::init();
    let mut g = Grafts::new(OidType::Sha1);
    g.parse(format!("{COMMIT} {TREE}\n{BLOB}\n").as_bytes()).unwrap();
    assert_eq!(g.len(), 2);
    assert_eq!(g.parents(&oid(COMMIT)).unwrap(), &[oid(TREE)]);
    assert_eq!(g.parents(&oid(BLOB)).unwrap(), &[]);
    assert!(g.parents(&oid(TREE)).is_none());
    let ids = g.oids().unwrap();
    assert_eq!(ids.len(), 2);
    let e = g.parse(b"nope\n").unwrap_err();
    assert_eq!(e, ErrorCode::GENERIC);
}

#[test]
#[cfg_attr(miri, ignore = "reaches the filesystem")]
fn grafts_file_refresh() {
    let _g = test_support::init();
    let d = std::env::temp_dir().join(format!("l_obj_api-grafts-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("shallow");
    let c = std::ffi::CString::new(f.to_str().unwrap()).unwrap();
    let mut g = Grafts::open(&c, OidType::Sha1).unwrap();
    assert!(g.is_empty());
    std::fs::write(&f, format!("{COMMIT}\n")).unwrap();
    g.refresh().unwrap();
    assert_eq!(g.len(), 1);
    std::fs::write(&f, format!("{COMMIT}\n{BLOB}\n")).unwrap();
    g.refresh().unwrap();
    assert_eq!(g.len(), 2);
    std::fs::remove_file(&f).unwrap();
    g.refresh().unwrap();
    assert!(g.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn object_types() {
    assert_eq!(ObjectType::from_str_bytes(b"tag"), Some(ObjectType::Tag));
    assert_eq!(ObjectType::from_str_bytes(b"x"), None);
    assert_eq!(ObjectType::Tree.to_string(), "tree");
    assert!(!ObjectType::Any.is_valid());
    assert!(
        libgit2_rs::object::rawcontent_is_valid(b"anything", ObjectType::Blob, OidType::Sha1).unwrap()
    );
}
