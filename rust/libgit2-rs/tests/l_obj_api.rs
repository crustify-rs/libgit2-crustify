//! The safe Rust API of the objects component (`Object` and its
//! conversions, `Blob`, `Tag`, `Signature`, `Time`, object ids), used
//! without `unsafe` and through the crate root only (PORTING.md §5), on
//! copies of the clar fixtures. Expected values are the C's (clar tests in
//! `tests/libgit2/object`, `blob`, `tag`, the reference build).

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use libgit2_rs::{
    Blob, BlobFilterOptions, Commit, ErrorClass, ErrorCode, Object, ObjectIdOptions, ObjectType,
    Oid, OidType, Repository, Signature, SignatureRef, Tag, Time, Tree,
};

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

    /// A work-tree fixture (`.gitted` renamed to `.git`, as clar does).
    fn workdir(name: &str, fixture: &str) -> Scratch {
        let s = Scratch::new(name, fixture);
        std::fs::rename(s.0.join(".gitted"), s.0.join(".git")).unwrap();
        s
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn oid(s: &str) -> Oid {
    Oid::from_hex(s, OidType::Sha1).unwrap()
}

const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
const TAG: &str = "7b4384978d2493e851f9cca7858815fac9b10980";
/// `refs/tags/point_to_blob`'s target, a blob.
const TAGGED_BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn lookup_peel_and_ids() {
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let c = Object::lookup(&repo, &oid(COMMIT), None).unwrap();
    assert_eq!(c.kind(), ObjectType::Commit);
    assert_eq!(c.id(), &oid(COMMIT));
    let c2 = c.clone();
    assert_eq!(c2.id(), c.id());

    // A wrong type is GIT_ENOTFOUND.
    let e = Object::lookup(&repo, &oid(COMMIT), Some(ObjectType::Blob)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(
        e.message_bytes(),
        b"the requested type does not match the type in the ODB"
    );
    let e = Object::lookup(
        &repo,
        &oid("0000000000000000000000000000000000000001"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);

    let t = c.peel(Some(ObjectType::Tree)).unwrap();
    assert_eq!(t.kind(), ObjectType::Tree);
    assert_eq!(t.id(), &oid(TREE));
    let e = t.peel(Some(ObjectType::Commit)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.class(), ErrorClass::OBJECT);
    // Peeling to the same type is the object; "any" from a commit is its tree.
    assert_eq!(c.peel(Some(ObjectType::Commit)).unwrap().id(), c.id());
    assert_eq!(c.peel(None).unwrap().id(), &oid(TREE));
    let e = t.peel(None).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);

    // Short ids and prefixes.
    assert_eq!(c.short_id().unwrap(), "a65fedf");
    let p = Object::lookup_prefix(&repo, &oid(COMMIT), 7, Some(ObjectType::Commit)).unwrap();
    assert_eq!(p.id(), &oid(COMMIT));
    let e = Object::lookup_prefix(&repo, &oid(COMMIT), 3, None).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    assert_eq!(
        e.message_bytes(),
        b"ambiguous lookup - OID prefix is too short"
    );

    // By path.
    let readme = c.lookup_bypath("README", Some(ObjectType::Blob)).unwrap();
    assert_eq!(readme.kind(), ObjectType::Blob);
    let e = c
        .lookup_bypath(b"README", Some(ObjectType::Tree))
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(
        e.message_bytes(),
        b"object at path 'README' is not of the asked-for type 2"
    );
    let e = c.lookup_bypath(b"nope", None).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let e = c.lookup_bypath(b"a\0b", None).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
    let b = readme.into_blob().unwrap();
    assert!(!b.content().is_empty());
    assert_eq!(b.size(), b.content().len() as u64);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn conversions() {
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let all = [
        (COMMIT, ObjectType::Commit),
        (TREE, ObjectType::Tree),
        (BLOB, ObjectType::Blob),
        (TAG, ObjectType::Tag),
    ];
    for (id, kind) in all {
        let o = Object::lookup(&repo, &oid(id), None).unwrap();
        assert_eq!(o.kind(), kind);
        // Exactly one conversion succeeds; the others give the object back.
        let r = o.clone().into_commit();
        assert_eq!(r.is_ok(), kind == ObjectType::Commit);
        if let Err(back) = r {
            assert_eq!(back.id(), &oid(id));
            assert_eq!(back.kind(), kind);
        }
        let r = o.clone().into_tree();
        assert_eq!(r.is_ok(), kind == ObjectType::Tree);
        if let Err(back) = r {
            assert_eq!(back.id(), &oid(id));
        }
        let r = o.clone().into_blob();
        assert_eq!(r.is_ok(), kind == ObjectType::Blob);
        if let Err(back) = r {
            assert_eq!(back.id(), &oid(id));
        }
        let r = o.clone().into_tag();
        assert_eq!(r.is_ok(), kind == ObjectType::Tag);
        if let Err(back) = r {
            assert_eq!(back.id(), &oid(id));
        }
        assert_eq!(o.as_tree().is_some(), kind == ObjectType::Tree);
        assert_eq!(o.as_blob().is_some(), kind == ObjectType::Blob);
        assert_eq!(o.as_tag().is_some(), kind == ObjectType::Tag);
        // TryFrom, as the conversions.
        assert_eq!(
            Commit::try_from(o.clone()).is_ok(),
            kind == ObjectType::Commit
        );
        assert_eq!(Tree::try_from(o.clone()).is_ok(), kind == ObjectType::Tree);
        assert_eq!(Blob::try_from(o.clone()).is_ok(), kind == ObjectType::Blob);
        assert_eq!(Tag::try_from(o.clone()).is_ok(), kind == ObjectType::Tag);
    }

    // Round trips: typed -> object -> typed, same object.
    let c = Commit::lookup(&repo, &oid(COMMIT)).unwrap();
    let o = c.clone().into_object();
    assert_eq!(o.id(), c.id());
    assert_eq!(c.as_object().id(), c.id());
    let o2: Object<'_> = c.into();
    assert_eq!(o2.into_commit().unwrap().tree_id(), &oid(TREE));
    let t = Tree::lookup(&repo, &oid(TREE)).unwrap();
    assert_eq!(t.as_object().kind(), ObjectType::Tree);
    assert_eq!(t.into_object().into_tree().unwrap().len(), 3);
    let b = Blob::lookup(&repo, &oid(BLOB)).unwrap();
    let ob = b.into_object();
    assert_eq!(ob.as_blob().unwrap().content(), b"hey\n");

    // A tag's target becomes a commit without a second lookup.
    let tag = Tag::lookup(&repo, &oid(TAG)).unwrap();
    let target = tag.target().unwrap().into_commit().unwrap();
    assert_eq!(target.id(), tag.target_id());
    let tag_obj = tag.clone().into_object();
    assert_eq!(tag_obj.as_tag().unwrap().name(), b"e90810b");
    assert_eq!(Tag::try_from(tag_obj).unwrap().id(), &oid(TAG));
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn prefix_lookups() {
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    assert_eq!(
        Commit::lookup_prefix(&repo, &oid(COMMIT), 8).unwrap().id(),
        &oid(COMMIT)
    );
    assert_eq!(
        Tree::lookup_prefix(&repo, &oid(TREE), 10).unwrap().id(),
        &oid(TREE)
    );
    assert_eq!(
        Blob::lookup_prefix(&repo, &oid(BLOB), 7).unwrap().id(),
        &oid(BLOB)
    );
    assert_eq!(
        Tag::lookup_prefix(&repo, &oid(TAG), 40).unwrap().id(),
        &oid(TAG)
    );
    // The wrong type, too short, and absent.
    let e = Tree::lookup_prefix(&repo, &oid(COMMIT), 8).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let e = Commit::lookup_prefix(&repo, &oid(COMMIT), 3).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    let e = Commit::lookup_prefix(&repo, &oid("deadbeef00000000000000000000000000000000"), 8)
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let e = Tree::lookup(&repo, &oid(BLOB)).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn objects_outside_a_repository() {
    // A parsed tree or commit has no repository: what needs one fails
    // (the C would crash), what does not works.
    let mut raw = b"100644 a\0".to_vec();
    raw.extend_from_slice(&[1; 20]);
    let t = Tree::parse(&raw, OidType::Sha1).unwrap();
    let o = t.as_object();
    assert_eq!(o.kind(), ObjectType::Tree);
    assert_eq!(o.peel(Some(ObjectType::Tree)).unwrap().id(), t.id());
    assert_eq!(
        o.peel(None).unwrap_err().message(),
        "invalid argument: 'repo'"
    );
    assert!(o.short_id().is_err());
    assert!(o.lookup_bypath("a", None).is_err());
    let c = Commit::parse(
        format!("tree {TREE}\nauthor A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n"),
        OidType::Sha1,
    )
    .unwrap();
    let co = c.clone().into_object();
    assert!(co.peel(Some(ObjectType::Tree)).is_err());
    assert!(c.tree().is_err());
    assert_eq!(co.into_commit().unwrap().message(), b"m\n");
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn blobs() {
    let s = Scratch::new("blobs", "testrepo.git");
    let repo = Repository::open(&s.0).unwrap();
    let b = Blob::lookup(&repo, &oid(BLOB)).unwrap();
    assert_eq!(b.content(), b"hey\n");
    assert_eq!(b.id(), &oid(BLOB));
    assert!(!b.is_binary());
    assert_eq!(
        b.filter("file.txt", BlobFilterOptions::default())
            .unwrap()
            .unwrap(),
        b"hey\n"
    );

    let id = Blob::create_from_buffer(&repo, b"hello\0world").unwrap();
    let nb = Blob::lookup(&repo, &id).unwrap();
    assert!(nb.is_binary());
    assert_eq!(nb.filter("x", BlobFilterOptions::new()).unwrap(), None);
    assert_eq!(
        nb.filter(b"x", BlobFilterOptions::new().check_for_binary(false))
            .unwrap()
            .unwrap(),
        b"hello\0world"
    );
    assert!(nb.filter(b"a\0b", BlobFilterOptions::new()).is_err());
    assert_eq!(
        id,
        Object::id_from_buffer(b"hello\0world", ObjectIdOptions::new()).unwrap()
    );

    let mut w = Blob::writer(&repo, Some(b"streamed.txt")).unwrap();
    w.write(b"from a ").unwrap();
    std::io::Write::write_all(&mut w, b"stream\n").unwrap();
    let sid = w.commit().unwrap();
    assert_eq!(
        Blob::lookup(&repo, &sid).unwrap().content(),
        b"from a stream\n"
    );
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
    let e = Blob::create_from_disk(&repo, s.0.join("objects")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::DIRECTORY);
    // A bare repository has no work tree (C folds the code to -1).
    let e = Blob::create_from_workdir(&repo, "on_disk.txt").unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "repository has no working directory");
    assert!(Blob::data_is_binary(b"\0"));
    assert!(!Blob::data_is_binary("text"));
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn blobs_from_the_workdir() {
    let s = Scratch::workdir("workdir", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    std::fs::write(s.0.join("hello.txt"), b"hello, world.\n").unwrap();
    let id = Blob::create_from_workdir(&repo, "hello.txt").unwrap();
    assert_eq!(
        id,
        Object::id_from_buffer("hello, world.\n", ObjectIdOptions::new()).unwrap()
    );
    assert_eq!(
        Blob::lookup(&repo, &id).unwrap().content(),
        b"hello, world.\n"
    );
    let e = Blob::create_from_workdir(&repo, "missing.txt").unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let e = Blob::create_from_workdir(&repo, b"a\0b".as_slice()).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn tags() {
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
    assert_eq!(
        Tag::list(&repo, Some(b"e9*")).unwrap(),
        vec![b"e90810b".to_vec()]
    );
    let mut seen = 0;
    Tag::foreach(&repo, |name, _| {
        assert!(name.starts_with(b"refs/tags/"));
        seen += 1;
        true
    })
    .unwrap();
    assert_eq!(seen, names.len());
    // `false` stops, with Ok.
    let mut n = 0;
    Tag::foreach(&repo, |_, _| {
        n += 1;
        false
    })
    .unwrap();
    assert_eq!(n, 1);
    // A panic is resumed after the walk.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = Tag::foreach(&repo, |_, _| panic!("boom"));
    }));
    assert!(r.is_err());

    let sig = Signature::new(b"Tagger", b"tagger@example.com", Time::new(123456789, 60)).unwrap();
    let id = Tag::create(&repo, "the-tag", &target, &sig, "a message\n", false).unwrap();
    let nt = Tag::lookup(&repo, &id).unwrap();
    assert_eq!(nt.name(), b"the-tag");
    assert_eq!(nt.tagger().unwrap(), sig.as_ref());
    let e = Tag::create(&repo, b"the-tag", &target, &sig, b"again", false).unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    assert_eq!(e.message_bytes(), b"tag already exists");
    let e = Tag::create(&repo, b"-bad", &target, &sig, b"m", false).unwrap_err();
    assert_eq!(e.message_bytes(), b"'-bad' is not a valid tag name");
    // A borrowed signature works too; a NUL in it is refused.
    let r = SignatureRef::new(b"Ref Tagger", b"r@t", Time::new(1, 0));
    let id2 = Tag::create(&repo, "by-ref", &target, r, "m\n", false).unwrap();
    assert_eq!(Tag::lookup(&repo, &id2).unwrap().tagger().unwrap(), r);
    let bad = SignatureRef::new(b"a\0b", b"r@t", Time::new(1, 0));
    let e = Tag::create(&repo, "nul", &target, bad, "m\n", false).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);

    let lw = Tag::create_lightweight(&repo, "light", &target, false).unwrap();
    assert_eq!(&lw, target.id());
    Tag::delete(&repo, "light").unwrap();
    assert_eq!(
        Tag::delete(&repo, "light").unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );

    let text =
        format!("object {COMMIT}\ntype commit\ntag from-buffer\ntagger A <a@b> 1 +0000\n\nmsg\n");
    let bid = Tag::create_from_buffer(&repo, &text, false).unwrap();
    assert_eq!(Tag::lookup(&repo, &bid).unwrap().name(), b"from-buffer");
    let bad = format!("object {COMMIT}\ntype tree\ntag x\n\nmsg\n");
    let e = Tag::create_from_buffer(&repo, bad, false).unwrap_err();
    assert_eq!(
        e.message_bytes(),
        b"the type for the given target is invalid"
    );

    assert!(Tag::name_is_valid("v1.0").unwrap());
    assert!(!Tag::name_is_valid(b"HEAD").unwrap());
    assert!(!Tag::name_is_valid(b"a..b").unwrap());

    let annotation = Tag::annotation_create(&repo, b"loose", &target, &sig, b"m").unwrap();
    assert_eq!(Tag::lookup(&repo, &annotation).unwrap().name(), b"loose");

    // A tag of a blob peels to the blob.
    let blob_tag = Tag::create(
        &repo,
        "to-blob",
        &Object::lookup(&repo, &oid(TAGGED_BLOB), None).unwrap(),
        &sig,
        "b\n",
        false,
    )
    .unwrap();
    let bt = Tag::lookup(&repo, &blob_tag).unwrap();
    assert_eq!(bt.target_type(), ObjectType::Blob);
    assert_eq!(bt.peel().unwrap().into_blob().unwrap().content(), b"hey\n");
}

#[test]
fn signatures() {
    let s = Signature::from_buffer(b"Name <e@mail> 1600000000 -0130").unwrap();
    assert_eq!(s.name(), b"Name");
    assert_eq!(s.email(), b"e@mail");
    assert_eq!(s.when().seconds(), 1600000000);
    assert_eq!(s.when().offset_minutes(), -90);
    assert_eq!(s.when().sign(), b'-');
    assert_eq!(
        s.to_header(b"author "),
        b"author Name <e@mail> 1600000000 -0130\n"
    );
    assert_eq!(s.to_string(), "Name <e@mail>");
    let e = Signature::from_buffer(b"no email here").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(
        e.message_bytes(),
        b"failed to parse signature - malformed e-mail"
    );
    let e = Signature::new(b"a<b", b"c", Time::new(0, 0)).unwrap_err();
    assert_eq!(
        e.message_bytes(),
        b"failed to parse signature - Neither `name` nor `email` should contain angle brackets chars."
    );
    assert!(Signature::new(b"a\0b", b"c", Time::new(0, 0)).is_err());
    let c = s.clone();
    assert_eq!(c, s);
    let r: SignatureRef<'_> = (&s).into();
    assert_eq!(r.to_signature().unwrap(), s);
}

#[test]
#[cfg_attr(miri, ignore = "calls the C library's mktime")]
fn signature_now() {
    let now = Signature::now(b"N", b"E").unwrap();
    assert!(now.when().seconds() > 1_600_000_000);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn default_signatures() {
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
#[cfg_attr(miri, ignore = "hashes through the C-shaped hash backends")]
fn object_ids() {
    // `git_object_id_from_buffer` (tests/libgit2/object/raw).
    let empty_blob = Object::id_from_buffer(b"", ObjectIdOptions::new()).unwrap();
    assert_eq!(empty_blob, oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
    let empty_tree =
        Object::id_from_buffer(b"", ObjectIdOptions::new().object_type(ObjectType::Tree)).unwrap();
    assert_eq!(empty_tree, oid("4b825dc642cb6eb9a060e54bf8d69288fbee4904"));
    let sha256 =
        Object::id_from_buffer(b"", ObjectIdOptions::new().oid_type(OidType::Sha256)).unwrap();
    assert_eq!(
        sha256,
        Oid::from_hex(
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813",
            OidType::Sha256
        )
        .unwrap()
    );
    assert_eq!(ObjectIdOptions::default(), ObjectIdOptions::new());

    // `git_object_id_from_file`: the same over a file.
    let d = std::env::temp_dir().join(format!("l_obj_api-ids-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("content");
    std::fs::write(&f, b"hey\n").unwrap();
    assert_eq!(
        Object::id_from_file(&f, ObjectIdOptions::new()).unwrap(),
        oid(BLOB)
    );
    assert_eq!(
        Object::id_from_file(&f, ObjectIdOptions::new().object_type(ObjectType::Commit)).unwrap(),
        Object::id_from_buffer(
            b"hey\n",
            ObjectIdOptions::new().object_type(ObjectType::Commit)
        )
        .unwrap()
    );
    let e = Object::id_from_file(d.join("missing"), ObjectIdOptions::new()).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let _ = std::fs::remove_dir_all(&d);

    // `git_object_rawcontent_is_valid`.
    let mut tree = b"100644 a\0".to_vec();
    tree.extend_from_slice(&[1; 20]);
    assert!(Object::rawcontent_is_valid(&tree, ObjectType::Tree, OidType::Sha1).unwrap());
    assert!(!Object::rawcontent_is_valid(b"100644 a", ObjectType::Tree, OidType::Sha1).unwrap());
    assert!(!Object::rawcontent_is_valid(b"x", ObjectType::Commit, OidType::Sha1).unwrap());
    assert!(Object::rawcontent_is_valid(b"anything", ObjectType::Blob, OidType::Sha1).unwrap());
}

#[test]
fn object_types() {
    assert_eq!(ObjectType::from_name(b"tag"), Some(ObjectType::Tag));
    assert_eq!(ObjectType::from_name("x"), None);
    assert_eq!(ObjectType::Tree.to_string(), "tree");
    assert_eq!(ObjectType::Commit.name(), "commit");
}

/// The object types cross threads (I-14).
#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn objects_across_threads() {
    fn send_sync<T: Send + Sync>(_: &T) {}
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let c = Commit::lookup(&repo, &oid(COMMIT)).unwrap();
    let t = Tree::lookup(&repo, &oid(TREE)).unwrap();
    let b = Blob::lookup(&repo, &oid(BLOB)).unwrap();
    let g = Tag::lookup(&repo, &oid(TAG)).unwrap();
    let o = Object::lookup(&repo, &oid(COMMIT), None).unwrap();
    send_sync(&c);
    send_sync(&t);
    send_sync(&b);
    send_sync(&g);
    send_sync(&o);
    std::thread::scope(|s| {
        // Shared borrows on several threads at once.
        let mut hs = Vec::new();
        for _ in 0..4 {
            hs.push(s.spawn(|| {
                for _ in 0..50 {
                    assert_eq!((c.summary(), c.parent_count()), (&b""[..], 1));
                    assert_eq!(c.tree().unwrap().id(), t.id());
                    assert_eq!(t.get_name("README").unwrap().kind(), ObjectType::Blob);
                    assert_eq!(b.content(), b"hey\n");
                    assert_eq!(g.target().unwrap().id(), g.target_id());
                    assert_eq!(o.short_id().unwrap(), "a65fedf");
                    // Clones (and their drops) on this thread.
                    let o2 = o.clone();
                    assert_eq!(o2.peel(None).unwrap().id(), t.id());
                }
            }));
        }
        // Owned values moved to (and dropped on) other threads.
        let (c2, t2, b2, g2) = (c.clone(), t.clone(), b.clone(), g.clone());
        let repo = &repo;
        hs.push(s.spawn(move || {
            assert_eq!(c2.parent(0).unwrap().parent_count(), 2);
            let e = t2.get_path("README").unwrap();
            let entry = std::thread::scope(|s2| s2.spawn(move || e).join().unwrap());
            assert_eq!(entry.to_object(&repo).unwrap().kind(), ObjectType::Blob);
            drop((b2, g2));
        }));
        for h in hs {
            h.join().unwrap();
        }
    });
}
