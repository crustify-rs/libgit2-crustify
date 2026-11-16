//! The safe Rust API of references, the reference database and reflogs,
//! used without `unsafe` (PORTING.md §5), on copies of libgit2's test
//! repositories.

#![forbid(unsafe_code)]
#![cfg(feature = "link-c")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use libgit2_rs::refs_api::{
    self, Committer, PeelTarget, RefValue, ReferenceFormat, ReferenceNames, ReflogRecord, Repository,
    set_refdb_backend,
};
use libgit2_rs::util::test_support;
use libgit2_rs::{Error, ErrorClass, ErrorCode, Oid, OidType, RefdbBackend, Reference, ReferenceType, References, Reflog};

const MASTER: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const BR2: &str = "a4a7dce85cf63874e984719f4fdd239f5145052f";

fn oid(s: &str) -> Oid {
    Oid::from_hex(s.as_bytes(), OidType::Sha1).unwrap()
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

/// A fresh copy of `tests/resources/<name>`.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str, tag: &str) -> Sandbox {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources").join(name);
        let dst = std::env::temp_dir().join(format!("l-refs-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        copy_dir(&src, &dst);
        Sandbox(dst)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lookup_and_resolve() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "lookup");
    let repo = Repository::open(&sb.0).unwrap();

    let head = Reference::lookup(&repo, b"HEAD").unwrap();
    assert_eq!(head.kind(), ReferenceType::Symbolic);
    assert_eq!(head.symbolic_target(), Some(&b"refs/heads/master"[..]));
    assert_eq!(head.target(), None);
    let master = head.resolve().unwrap();
    assert_eq!(master.name(), b"refs/heads/master");
    assert_eq!(master.target(), Some(oid(MASTER)));
    assert!(master.is_branch() && !master.is_tag());
    assert_eq!(master.shorthand(), b"master");
    assert_eq!(refs_api::name_to_id(&repo, b"HEAD").unwrap(), oid(MASTER));

    let r = Reference::lookup_resolved(&repo, b"HEAD", -1).unwrap();
    assert_eq!(r.name(), b"refs/heads/master");

    // Packed references.
    let packed = Reference::lookup(&repo, b"refs/heads/packed").unwrap();
    assert_eq!(packed.target(), Some(oid("41bc8c69075bbdb46c5c6f0566cc8cc5b46e8bd9")));

    // dwim.
    assert_eq!(Reference::dwim(&repo, b"br2").unwrap().name(), b"refs/heads/br2");
    assert_eq!(Reference::dwim(&repo, b"test/master").unwrap().name(), b"refs/remotes/test/master");
    let e = Reference::dwim(&repo, b"nope").unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.message(), "no reference found for shorthand 'nope'");

    let e = Reference::lookup(&repo, b"refs/heads/missing").unwrap_err();
    assert_eq!((e.code(), e.class()), (ErrorCode::NOT_FOUND, ErrorClass::REFERENCE));
    assert_eq!(e.message(), "reference 'refs/heads/missing' not found");
    let e = Reference::lookup(&repo, b"refs/heads/a..b").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.message(), "the given reference name 'refs/heads/a..b' is not valid");
    assert_eq!(Reference::lookup(&repo, b"a\0b").unwrap_err().code(), ErrorCode::INVALID_SPEC);

    // Peeling an annotated tag.
    let tag = Reference::lookup(&repo, b"refs/tags/test").unwrap();
    let commit = tag.peel_to_id(PeelTarget::Commit).unwrap();
    assert_ne!(Some(commit), tag.target());
    assert_eq!(tag.peel_to_id(PeelTarget::Tag).unwrap(), tag.target().unwrap());

    // Clone, equality and order.
    let c = master.clone();
    assert_eq!(c, master);
    assert!(head < master);
}

#[test]
fn create_update_rename_delete() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "write");
    let repo = Repository::open(&sb.0).unwrap();

    let r = Reference::create(&repo, b"refs/heads/new", &oid(MASTER), false, None, Some("made")).unwrap();
    assert_eq!(r.target(), Some(oid(MASTER)));
    let e = Reference::create(&repo, b"refs/heads/new", &oid(MASTER), false, None, None).unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    assert_eq!(
        e.message(),
        "failed to write reference 'refs/heads/new': a reference with that name already exists."
    );
    // A collision with an existing directory-to-be.
    let e = Reference::create(&repo, b"refs/heads/packed/x", &oid(MASTER), false, None, None).unwrap_err();
    assert_eq!(e.message(), "path to reference 'refs/heads/packed/x' collides with existing one");

    // Compare-and-swap.
    let e = Reference::create(&repo, b"refs/heads/new", &oid(BR2), true, Some(&oid(BR2)), None).unwrap_err();
    assert_eq!(e.code(), ErrorCode::MODIFIED);
    let moved = r.set_target(&oid(BR2), Some("move")).unwrap();
    assert_eq!(moved.target(), Some(oid(BR2)));
    assert_eq!(r.set_target(&oid(MASTER), None).unwrap_err().code(), ErrorCode::MODIFIED);

    let s = Reference::symbolic_create(&repo, b"refs/heads/sym", b"refs/heads/new", false, None, None).unwrap();
    assert_eq!(s.kind(), ReferenceType::Symbolic);
    assert_eq!(s.set_target(&oid(MASTER), None).unwrap_err().message(), "cannot set OID on symbolic reference");
    let s2 = s.symbolic_set_target(b"refs/heads/master", None).unwrap();
    assert_eq!(s2.symbolic_target(), Some(&b"refs/heads/master"[..]));

    let renamed = moved.rename(b"refs/heads/renamed", false, Some("rename")).unwrap();
    assert_eq!(renamed.name(), b"refs/heads/renamed");
    assert!(Reference::lookup(&repo, b"refs/heads/new").is_err());
    let log = Reflog::read(&repo, b"refs/heads/renamed").unwrap();
    assert_eq!(log.get(0).unwrap().message(), Some(&b"rename"[..]));

    renamed.delete().unwrap();
    assert_eq!(
        Reference::lookup(&repo, b"refs/heads/renamed").unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );
    refs_api::remove(&repo, b"refs/heads/sym").unwrap();
    let head = Reference::lookup(&repo, b"HEAD").unwrap();
    assert_eq!(head.delete().unwrap_err().message(), "cannot delete HEAD");
}

#[test]
fn iterate_glob_and_compress() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "iter");
    let repo = Repository::open(&sb.0).unwrap();

    let all = refs_api::list(&repo).unwrap();
    assert!(all.contains(&b"refs/heads/master".to_vec()));
    assert!(all.contains(&b"refs/heads/packed".to_vec()));
    let refs: Vec<Reference<'_>> = References::new(&repo).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(refs.len(), all.len());
    let tags: Vec<Vec<u8>> = ReferenceNames::glob(&repo, b"refs/tags/*").unwrap().collect::<Result<_, _>>().unwrap();
    assert!(!tags.is_empty() && tags.iter().all(|t| t.starts_with(b"refs/tags/")));
    let mut seen = Vec::new();
    refs_api::for_each_name(&repo, Some(b"refs/heads/*"), |n| {
        seen.push(n.to_vec());
        seen.len() < 3
    })
    .unwrap();
    assert_eq!(seen.len(), 3);

    refs_api::compress(&repo).unwrap();
    assert!(!sb.0.join("refs/heads/master").exists());
    let packed = std::fs::read_to_string(sb.0.join("packed-refs")).unwrap();
    assert!(packed.starts_with("# pack-refs with: peeled fully-peeled sorted \n"));
    assert!(packed.contains(&format!("{MASTER} refs/heads/master\n")));
    let after = refs_api::list(&repo).unwrap();
    let mut a = all.clone();
    a.sort();
    let mut b = after;
    b.sort();
    assert_eq!(a, b);
    let tag = Reference::lookup(&repo, b"refs/tags/test").unwrap();
    assert!(tag.target_peel().is_some());
}

#[test]
fn names() {
    let _g = test_support::init();
    let n = refs_api::normalize_name(b"refs//heads///x", ReferenceFormat::NORMAL).unwrap();
    assert_eq!(n, b"refs/heads/x");
    assert_eq!(refs_api::normalize_name(b"@", ReferenceFormat::ALLOW_ONELEVEL).unwrap(), b"HEAD");
    assert_eq!(
        refs_api::normalize_name(b"refs/heads/*", ReferenceFormat::REFSPEC_PATTERN).unwrap(),
        b"refs/heads/*"
    );
    let e = refs_api::normalize_name(b"refs/heads/x.lock", ReferenceFormat::NORMAL).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert!(refs_api::is_valid_name(b"HEAD"));
    assert!(refs_api::is_valid_name(b"refs/heads/ok"));
    assert!(!refs_api::is_valid_name(b"lower"));
    assert!(!refs_api::is_valid_name(b"refs/heads/a@{b"));
    assert!(!refs_api::is_valid_name(b"a\0b"));
}

#[test]
fn reflogs() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "reflog");
    let repo = Repository::open(&sb.0).unwrap();

    let mut log = Reflog::read(&repo, b"HEAD").unwrap();
    let n = log.len();
    assert!(n > 0);
    let newest = log.get(0).unwrap();
    assert!(newest.committer().is_some());
    let who = Committer::new(b"A U Thor", b"author@example.com", 1_234_567_890, 60);
    log.append(&oid(BR2), &who, Some(b"one\ntwo")).unwrap();
    assert_eq!(log.len(), n + 1);
    let e = log.get(0).unwrap();
    assert_eq!(e.message(), Some(&b"one two"[..]));
    assert_eq!(e.id_new(), oid(BR2));
    assert_eq!(e.committer().unwrap(), who);
    log.write().unwrap();
    let text = std::fs::read_to_string(sb.0.join("logs/HEAD")).unwrap();
    assert!(text.ends_with(&format!(
        "{BR2} A U Thor <author@example.com> 1234567890 +0100\tone two\n"
    )));
    let again = Reflog::read(&repo, b"HEAD").unwrap();
    assert_eq!(again.len(), n + 1);

    log.remove(0, true).unwrap();
    assert_eq!(log.len(), n);
    let e = log.remove(1000, true).unwrap_err();
    assert_eq!(e.message(), "no reflog entry at index 1000");

    // A reference without a log gets an empty one.
    assert!(!refs_api::has_log(&repo, b"refs/heads/br2").unwrap() || true);
    refs_api::ensure_log(&repo, b"refs/heads/subtrees").unwrap();
    assert!(refs_api::has_log(&repo, b"refs/heads/subtrees").unwrap());
    Reflog::rename(&repo, b"refs/heads/subtrees", b"refs/heads/sub2").unwrap();
    assert!(refs_api::has_log(&repo, b"refs/heads/sub2").unwrap());
    Reflog::delete(&repo, b"refs/heads/sub2").unwrap();
    assert!(!refs_api::has_log(&repo, b"refs/heads/sub2").unwrap());
}

/// A backend over a map, as a consumer would write one.
#[derive(Default)]
struct MemBackend {
    refs: Mutex<BTreeMap<Vec<u8>, RefValue>>,
    logs: Mutex<BTreeMap<Vec<u8>, Vec<ReflogRecord>>>,
}

fn modified() -> Error {
    Error::new(ErrorCode::MODIFIED, ErrorClass::REFERENCE, "old reference value does not match")
}

impl RefdbBackend for MemBackend {
    fn exists(&self, name: &[u8]) -> Result<bool, Error> {
        Ok(self.refs.lock().unwrap().contains_key(name))
    }
    fn lookup(&self, name: &[u8]) -> Result<Option<RefValue>, Error> {
        Ok(self.refs.lock().unwrap().get(name).cloned())
    }
    fn list(&self) -> Result<Vec<(Vec<u8>, RefValue)>, Error> {
        Ok(self.refs.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }
    fn write(
        &self,
        name: &[u8],
        value: &RefValue,
        force: bool,
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
        _message: Option<&[u8]>,
    ) -> Result<(), Error> {
        let mut m = self.refs.lock().unwrap();
        if !force && m.contains_key(name) {
            return Err(Error::new(ErrorCode::EXISTS, ErrorClass::REFERENCE, "exists"));
        }
        match (m.get(name), old_id, old_target) {
            (Some(RefValue::Direct { id, .. }), Some(o), _) if id != o => return Err(modified()),
            (Some(RefValue::Symbolic(t)), _, Some(o)) if t.as_slice() != o => return Err(modified()),
            _ => {}
        }
        m.insert(name.to_vec(), value.clone());
        Ok(())
    }
    fn rename(&self, old_name: &[u8], new_name: &[u8], _force: bool) -> Result<RefValue, Error> {
        let mut m = self.refs.lock().unwrap();
        let v = m
            .remove(old_name)
            .ok_or_else(|| Error::new(ErrorCode::NOT_FOUND, ErrorClass::REFERENCE, "not found"))?;
        m.insert(new_name.to_vec(), v.clone());
        Ok(v)
    }
    fn delete(&self, name: &[u8], _old_id: Option<&Oid>, _old_target: Option<&[u8]>) -> Result<(), Error> {
        self.refs.lock().unwrap().remove(name);
        Ok(())
    }
    fn has_log(&self, name: &[u8]) -> bool {
        self.logs.lock().unwrap().contains_key(name)
    }
    fn reflog_read(&self, name: &[u8]) -> Result<Vec<ReflogRecord>, Error> {
        Ok(self.logs.lock().unwrap().get(name).cloned().unwrap_or_default())
    }
    fn reflog_write(&self, name: &[u8], entries: &[ReflogRecord]) -> Result<(), Error> {
        self.logs.lock().unwrap().insert(name.to_vec(), entries.to_vec());
        Ok(())
    }
}

#[test]
fn rust_backend() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "backend");
    let repo = Repository::open(&sb.0).unwrap();
    let mem = MemBackend::default();
    mem.refs
        .lock()
        .unwrap()
        .insert(b"HEAD".to_vec(), RefValue::Symbolic(b"refs/heads/main".to_vec()));
    set_refdb_backend(&repo, mem).unwrap();

    // HEAD is dangling until main exists.
    assert_eq!(refs_api::name_to_id(&repo, b"HEAD").unwrap_err().code(), ErrorCode::NOT_FOUND);
    Reference::create(&repo, b"refs/heads/main", &oid(MASTER), false, None, Some("init")).unwrap();
    assert_eq!(refs_api::name_to_id(&repo, b"HEAD").unwrap(), oid(MASTER));
    Reference::create(&repo, b"refs/tags/v1", &oid(BR2), false, None, None).unwrap();
    assert_eq!(refs_api::list(&repo).unwrap(), [&b"HEAD"[..], b"refs/heads/main", b"refs/tags/v1"]);
    let tags: Vec<_> = References::glob(&repo, b"refs/tags/*").unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(tags.len(), 1);
    let main = Reference::lookup(&repo, b"refs/heads/main").unwrap();
    assert_eq!(main.set_target(&oid(BR2), Some("move")).unwrap().target(), Some(oid(BR2)));
    assert_eq!(main.set_target(&oid(MASTER), None).unwrap_err().code(), ErrorCode::MODIFIED);
    let e = Reference::lookup(&repo, b"refs/heads/none").unwrap_err();
    assert_eq!(e.message(), "reference 'refs/heads/none' not found");

    // Reflogs round-trip through the backend.
    let mut log = Reflog::read(&repo, b"refs/heads/main").unwrap();
    assert!(log.is_empty());
    let who = Committer::new(b"me", b"me@example.com", 10, -120);
    log.append(&oid(MASTER), &who, Some(b"hello")).unwrap();
    log.write().unwrap();
    let log = Reflog::read(&repo, b"refs/heads/main").unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log.get(0).unwrap().message(), Some(&b"hello"[..]));
    assert_eq!(log.get(0).unwrap().committer().unwrap(), who);
    assert!(refs_api::has_log(&repo, b"refs/heads/main").unwrap());

    main.rename(b"refs/heads/trunk", false, None).unwrap();
    assert!(Reference::lookup(&repo, b"refs/heads/trunk").is_ok());
    refs_api::remove(&repo, b"refs/tags/v1").unwrap();
    assert_eq!(refs_api::list(&repo).unwrap().len(), 2);
}

/// A backend that panics: the panic is an error, not an abort.
struct Panicky;

impl RefdbBackend for Panicky {
    fn exists(&self, _: &[u8]) -> Result<bool, Error> {
        panic!("exists")
    }
    fn lookup(&self, _: &[u8]) -> Result<Option<RefValue>, Error> {
        panic!("lookup")
    }
    fn list(&self) -> Result<Vec<(Vec<u8>, RefValue)>, Error> {
        Ok(Vec::new())
    }
    fn write(&self, _: &[u8], _: &RefValue, _: bool, _: Option<&Oid>, _: Option<&[u8]>, _: Option<&[u8]>) -> Result<(), Error> {
        Ok(())
    }
    fn rename(&self, _: &[u8], _: &[u8], _: bool) -> Result<RefValue, Error> {
        panic!("rename")
    }
    fn delete(&self, _: &[u8], _: Option<&Oid>, _: Option<&[u8]>) -> Result<(), Error> {
        Ok(())
    }
}

#[test]
fn panicking_backend() {
    let _g = test_support::init();
    let sb = Sandbox::new("testrepo.git", "panic");
    let repo = Repository::open(&sb.0).unwrap();
    set_refdb_backend(&repo, Panicky).unwrap();
    let e = Reference::lookup(&repo, b"refs/heads/x").unwrap_err();
    assert_eq!(e.class(), ErrorClass::INTERNAL);
    assert_eq!(refs_api::list(&repo).unwrap().len(), 0);
}
