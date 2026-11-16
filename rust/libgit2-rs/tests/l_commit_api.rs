//! The safe Rust API of commits and trees, used without `unsafe` and
//! through the crate root only (PORTING.md §5). Reads
//! `tests/resources/testrepo.git`; writes into copies of the fixtures.
//! Expected values are the C's (clar: `commit/create.c`, `commit/amend.c`,
//! `commit/write.c`, `commit/parse.c`, `object/commit/commitstagedfile.c`,
//! `repo/getters.c`).

#![forbid(unsafe_code)]
#![cfg(feature = "link-c")]

use libgit2_rs::{
    AmendOptions, Commit, CommitBuilder, CommitCreateOptions, Error, ErrorClass, ErrorCode,
    ObjectType, Oid, OidType, Repository, Signature, SignatureRef, Time, Tree, TreeBuilder,
    TreeUpdate, TreeWalk, TreeWalkMode,
};
use std::path::{Path, PathBuf};

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn oid(s: &str) -> Oid {
    Oid::from_hex(s, OidType::Sha1).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

/// A writable copy of a fixture, removed on drop.
struct Sandbox(PathBuf);

impl Sandbox {
    /// A copy of a bare fixture.
    fn bare(name: &str, fixture: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("l_commit_api_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&resources().join(fixture), &dir);
        Sandbox(dir)
    }

    /// A copy of a work-tree fixture (`.gitted` renamed to `.git`, as
    /// clar's `cl_git_sandbox_init` does).
    fn workdir(name: &str, fixture: &str) -> Sandbox {
        let s = Sandbox::bare(name, fixture);
        std::fs::rename(s.0.join(".gitted"), s.0.join(".git")).unwrap();
        s
    }

    fn write(&self, path: &str, data: &str) {
        std::fs::write(self.0.join(path), data).unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn head_tree(repo: &Repository) -> Oid {
    let head = repo.head().unwrap();
    let id = head.target().unwrap();
    *Commit::lookup(repo, &id).unwrap().tree_id()
}

fn head_id(repo: &Repository) -> Oid {
    repo.head().unwrap().target().unwrap()
}

/// `commit/parse.c`'s signature.
const GPGSIG: &str = "-----BEGIN PGP SIGNATURE-----
Version: GnuPG v1.4.12 (Darwin)

iQIcBAABAgAGBQJQ+FMIAAoJEH+LfPdZDSs1e3EQAJMjhqjWF+WkGLHju7pTw2al
o6IoMAhv0Z/LHlWhzBd9e7JeCnanRt12bAU7yvYp9+Z+z+dbwqLwDoFp8LVuigl8
JGLcnwiUW3rSvhjdCp9irdb4+bhKUnKUzSdsR2CK4/hC0N2i/HOvMYX+BRsvqweq
AsAkA6dAWh+gAfedrBUkCTGhlNYoetjdakWqlGL1TiKAefEZrtA1TpPkGn92vbLq
SphFRUY9hVn1ZBWrT3hEpvAIcZag3rTOiRVT1X1flj8B2vGCEr3RrcwOIZikpdaW
who/X3xh/DGbI2RbuxmmJpxxP/8dsVchRJJzBwG+yhwU/iN3MlV2c5D69tls/Dok
6VbyU4lm/ae0y3yR83D9dUlkycOnmmlBAHKIZ9qUts9X7mWJf0+yy2QxJVpjaTGG
cmnQKKPeNIhGJk2ENnnnzjEve7L7YJQF6itbx5VCOcsGh3Ocb3YR7DMdWjt7f8pu
c6j+q1rP7EpE2afUN/geSlp5i3x8aXZPDj67jImbVCE/Q1X9voCtyzGJH7MXR0N9
ZpRF8yzveRfMH8bwAJjSOGAFF5XkcR/RNY95o+J+QcgBLdX48h+ZdNmUf6jqlu3J
7KmTXXQcOVpN6dD3CmRFsbjq+x6RHwa8u1iGn+oIkX908r97ckfB/kHKH7ZdXIJc
cpxtDQQMGYFpXK/71stq
=ozeK
-----END PGP SIGNATURE-----";

/// `commit/write.c`'s unsigned commit (on testrepo).
const UNSIGNED: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
parent 8496071c1b46c854b31185ea97743be6a8774479
author Ben Burkert <ben@benburkert.com> 1358451456 -0800
committer Ben Burkert <ben@benburkert.com> 1358451456 -0800

a simple commit which works
";

/// [`UNSIGNED`] with [`GPGSIG`] as its `gpgsig` header (`signed_data` of
/// `commit/write.c`).
fn signed() -> String {
    let (head, msg) = UNSIGNED.split_once("\n\n").unwrap();
    format!("{head}\ngpgsig {}\n\n{msg}", GPGSIG.replace('\n', "\n "))
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn commits() {
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let head = Commit::lookup(&repo, &oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750")).unwrap();
    assert_eq!(head.id(), &oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750"));
    assert_eq!(
        head.tree_id(),
        &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")
    );
    assert_eq!(
        head.parent_ids(),
        [oid("be3563ae3f795b2b4353bcce3a527ad0a4f7f644")]
    );
    let a = head.author();
    assert_eq!(
        (a.name(), a.email(), a.when()),
        (
            &b"Scott Chacon"[..],
            &b"schacon@gmail.com"[..],
            Time::new(1312943626, -420)
        )
    );

    let p = head.parent(0).unwrap();
    assert_eq!(p.summary(), b"Merge branch 'br2'");
    assert_eq!(p.body(), None);
    assert_eq!(p.message(), b"Merge branch 'br2'\n");
    assert_eq!(p.committer().when().seconds(), 1274813907);
    assert_eq!(p.time(), 1274813907);
    assert_eq!(p.time_offset(), -420);
    assert_eq!(p.parent_count(), 2);
    assert_eq!(
        p.header_field("tree").unwrap(),
        p.tree_id().to_string().into_bytes()
    );
    assert_eq!(
        p.header_field(b"nope").unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );
    let e = head.parent(5).unwrap_err();
    assert_eq!(
        (e.code(), e.message().into_owned()),
        (ErrorCode::NOT_FOUND, "parent 5 does not exist".to_string())
    );

    let anc = head.nth_gen_ancestor(2).unwrap();
    assert_eq!(anc.id(), p.parent(0).unwrap().id());
    assert_eq!(head.nth_gen_ancestor(0).unwrap().id(), head.id());
    let c2 = head.clone();
    drop(head);
    assert_eq!(
        c2.tree().unwrap().id(),
        &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")
    );
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn parsed_commits() {
    let data = b"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nauthor A <a@b> 1 +0100\ncommitter C <c@d> 2 -0000\nencoding UTF-8\n\nSum\nmary\n\n  Body  \n";
    let c = Commit::parse(data, OidType::Sha1).unwrap();
    assert_eq!(c.summary(), b"Sum mary");
    assert_eq!(c.body(), Some(&b"Body"[..]));
    assert_eq!(c.message_encoding(), Some(&b"UTF-8"[..]));
    assert_eq!(c.committer().when().sign(), b'-');
    assert_eq!(c.author().when(), Time::new(1, 60));
    assert!(c.tree().is_err());
    assert!(c.parent(0).is_err() && c.nth_gen_ancestor(1).is_err());
    assert!(c.amend(&AmendOptions::new()).is_err());
    // The id is the bytes' (`git_object__from_raw` hashes them).
    assert_eq!(
        c.id(),
        &libgit2_rs::Object::id_from_buffer(
            data,
            libgit2_rs::ObjectIdOptions::new().object_type(ObjectType::Commit)
        )
        .unwrap()
    );
    let e = Commit::parse(b"tree x\n", OidType::Sha1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID);
    assert_eq!(e.message(), "failed to parse bad commit object");
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn trees_and_walks() {
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let t = Tree::lookup(&repo, &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")).unwrap();
    let names: Vec<_> = t.iter().map(|e| e.name().to_vec()).collect();
    assert_eq!(names, [&b"README"[..], b"branch_file.txt", b"new.txt"]);
    let e = t.get_name("new.txt").unwrap();
    assert_eq!(
        (e.id(), e.filemode(), e.kind()),
        (
            &oid("a71586c1dfe8a71c6cbf6c129f404c5642ff31bd"),
            0o100644,
            ObjectType::Blob
        )
    );
    assert!(t.get_name(b"nope").is_none());
    assert_eq!(t.get_id(e.id()).unwrap().name(), b"new.txt");
    // `git_tree_entry_to_object`.
    let o = e.to_object(&repo).unwrap();
    assert_eq!(o.kind(), ObjectType::Blob);
    assert_eq!(o.into_blob().unwrap().content(), b"my new file\n");

    // The "subtrees" tree.
    let st = Tree::lookup(&repo, &oid("ae90f12eea699729ed24555e40b9fd669da12a12")).unwrap();
    let ab = st.get_name("ab").unwrap();
    assert_eq!(ab.kind(), ObjectType::Tree);
    assert_eq!(ab.to_object(&repo).unwrap().into_tree().unwrap().len(), 3);
    let mut pre = Vec::new();
    st.walk(TreeWalkMode::PreOrder, |root, e| {
        pre.push(format!(
            "{}{}",
            String::from_utf8_lossy(root),
            String::from_utf8_lossy(e.name())
        ));
        TreeWalk::Continue
    })
    .unwrap();
    assert_eq!(
        pre,
        [
            "README",
            "ab",
            "ab/4.txt",
            "ab/c",
            "ab/c/3.txt",
            "ab/de",
            "ab/de/2.txt",
            "ab/de/fgh",
            "ab/de/fgh/1.txt",
            "branch_file.txt",
            "new.txt"
        ]
    );
    let mut post = Vec::new();
    st.walk(TreeWalkMode::PostOrder, |root, e| {
        post.push(format!(
            "{}{}",
            String::from_utf8_lossy(root),
            String::from_utf8_lossy(e.name())
        ));
        TreeWalk::Continue
    })
    .unwrap();
    assert_eq!(
        post,
        [
            "README",
            "ab/4.txt",
            "ab/c/3.txt",
            "ab/c",
            "ab/de/2.txt",
            "ab/de/fgh/1.txt",
            "ab/de/fgh",
            "ab/de",
            "ab",
            "branch_file.txt",
            "new.txt"
        ]
    );
    let mut n = 0;
    let e = st
        .walk(TreeWalkMode::PreOrder, |_, _| {
            n += 1;
            if n == 3 {
                TreeWalk::Stop
            } else {
                TreeWalk::Continue
            }
        })
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::USER);
    let mut skipped = 0;
    st.walk(TreeWalkMode::PreOrder, |_, e| {
        skipped += 1;
        if e.kind() == ObjectType::Tree {
            TreeWalk::Skip
        } else {
            TreeWalk::Continue
        }
    })
    .unwrap();
    assert_eq!(skipped, 4);

    let deep = st.get_path("ab/de/fgh/1.txt").unwrap();
    assert_eq!(deep.name(), b"1.txt");
    assert_eq!(deep.kind(), ObjectType::Blob);
    assert_eq!(deep.to_object(&repo).unwrap().kind(), ObjectType::Blob);
    let cloned = deep.clone();
    assert_eq!(cloned.as_ref().id(), deep.id());
    assert_eq!(
        st.get_path(b"ab/x").unwrap_err().message(),
        "the path 'x' does not exist in the given tree"
    );

    // Parsed from bytes: the entries borrow them.
    let mut raw = b"100644 a\0".to_vec();
    raw.extend_from_slice(&[1; 20]);
    let pt = Tree::parse(&raw, OidType::Sha1).unwrap();
    assert_eq!(pt.get(0).unwrap().name(), b"a");
    // Its entry's object is looked up in the repository given.
    assert_eq!(
        pt.get(0).unwrap().to_object(&repo).unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );
    assert_eq!(
        Tree::parse(b"100644 a", OidType::Sha1)
            .unwrap_err()
            .message(),
        "failed to parse tree: object is corrupted"
    );
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn builders_and_writes() {
    let dir = Sandbox::bare("write", "testrepo.git");
    let repo = Repository::open(&dir.0).unwrap();
    let base = Tree::lookup(&repo, &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")).unwrap();
    let readme = *base.get_name(b"README").unwrap().id();

    let mut b = TreeBuilder::new(&repo, Some(&base)).unwrap();
    assert_eq!(b.len(), 3);
    // Writing it back gives the same tree.
    assert_eq!(&b.write().unwrap(), base.id());
    b.insert("copy", &readme, 0o100644).unwrap();
    let e = b.insert(b"bad/name", &readme, 0o100644).unwrap_err();
    assert_eq!(
        e.message(),
        "failed to insert entry: invalid name for a tree entry - bad/name"
    );
    let e = b.insert(b"x", &readme, 0o100664).unwrap_err();
    assert_eq!(
        e.message(),
        "failed to insert entry: invalid filemode for file - x"
    );
    let e = b.insert(b"x", &readme, 0o040000).unwrap_err();
    assert_eq!(
        e.message(),
        "failed to insert entry: invalid object specified - x"
    );
    assert_eq!(b.get("copy").unwrap().id(), &readme);
    b.remove("new.txt").unwrap();
    assert!(b.remove(b"new.txt").is_err());
    b.filter(|e| e.name() == b"branch_file.txt");
    assert_eq!(b.len(), 2);
    let new_tree = b.write().unwrap();
    let t = Tree::lookup(&repo, &new_tree).unwrap();
    let names: Vec<_> = t.iter().map(|e| e.name().to_vec()).collect();
    assert_eq!(names, [&b"README"[..], b"copy"]);
    b.clear();
    assert!(b.is_empty());

    // create_updated: add a nested file, remove one, then a D/F conflict.
    let updated = Tree::create_updated(
        &repo,
        Some(&base),
        &[
            TreeUpdate::Upsert {
                path: b"dir/sub/file",
                id: readme,
                filemode: 0o100644,
            },
            TreeUpdate::Remove { path: b"new.txt" },
        ],
    )
    .unwrap();
    let ut = Tree::lookup(&repo, &updated).unwrap();
    assert_eq!(ut.get_path(b"dir/sub/file").unwrap().id(), &readme);
    assert!(ut.get_name(b"new.txt").is_none());
    let e = Tree::create_updated(
        &repo,
        Some(&base),
        &[TreeUpdate::Upsert {
            path: b"README/x",
            id: readme,
            filemode: 0o100644,
        }],
    )
    .unwrap_err();
    assert_eq!(e.message(), "D/F conflict when updating tree");
    let e = Tree::create_updated(
        &repo,
        None,
        &[
            TreeUpdate::Remove { path: b"a" },
            TreeUpdate::Remove { path: b"a" },
        ],
    )
    .unwrap_err();
    assert_eq!(e.message(), "duplicate entries given for update");

    // A commit of the new tree on top of HEAD.
    let sig = SignatureRef::new(b"Me", b"me@example.com", Time::new(1700000000, 60));
    let head = oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
    let builder = CommitBuilder::new(new_tree, sig, sig, b"A commit\n\nwith a body\n")
        .parent(head)
        .header(b"x-extra", b"one\ntwo");
    let buf = builder.to_buffer().unwrap();
    assert!(
        buf.starts_with(
            format!(
                "tree {new_tree}\nparent {head}\nauthor Me <me@example.com> 1700000000 +0100\n"
            )
            .as_bytes()
        )
    );
    assert!(buf.ends_with(b"x-extra one\n two\n\nA commit\n\nwith a body\n"));
    let id = builder
        .clone()
        .update_ref(b"refs/heads/master")
        .write(&repo)
        .unwrap();
    let c = Commit::lookup(&repo, &id).unwrap();
    assert_eq!(c.summary(), b"A commit");
    assert_eq!(c.body(), Some(&b"with a body"[..]));
    assert_eq!(c.header_field(b"x-extra").unwrap(), b"one\ntwo");
    assert_eq!(c.parent_ids(), [head]);
    // Updating the ref again with a commit whose first parent is not its tip.
    let e = builder
        .update_ref(b"refs/heads/master")
        .write(&repo)
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::MODIFIED);
    // A missing parent is refused (strict validation).
    let e = CommitBuilder::new(new_tree, sig, sig, b"m")
        .parent(oid("0123456789012345678901234567890123456789"))
        .write(&repo)
        .unwrap_err();
    assert!(e.code() != ErrorCode::MODIFIED);
}

/// `commit/write.c`: `signature_callback`, `attach_*_signature`,
/// `attach_signature_checks_objects`; `commit/parse.c`:
/// `extract_signature`.
#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn signed_commits() {
    let s = Sandbox::workdir("signed", "testrepo");
    let repo = Repository::open(&s.0).unwrap();
    let odb = repo.odb().unwrap();
    let ben = Signature::new(
        "Ben Burkert",
        "ben@benburkert.com",
        Time::new(1358451456, -480),
    )
    .unwrap();
    let builder = CommitBuilder::new(
        oid("4b825dc642cb6eb9a060e54bf8d69288fbee4904"),
        &ben,
        &ben,
        b"a simple commit which works\n",
    )
    .parent(oid("8496071c1b46c854b31185ea97743be6a8774479"));
    assert_eq!(builder.to_buffer().unwrap(), UNSIGNED.as_bytes());

    // A signing closure (`git_commit_create_ext` + `sign_cb`).
    let mut seen = Vec::new();
    let id = builder
        .write_signed(&repo, "gpgsig", |content| {
            seen = content.to_vec();
            Ok(Some(GPGSIG.as_bytes().to_vec()))
        })
        .unwrap();
    assert_eq!(seen, UNSIGNED.as_bytes());
    assert_eq!(odb.read(&id).unwrap().data(), signed().as_bytes());
    // `None` leaves it unsigned; an error fails with it; a panic resumes.
    let unsigned_id = builder.write_signed(&repo, "gpgsig", |_| Ok(None)).unwrap();
    assert_eq!(odb.read(&unsigned_id).unwrap().data(), UNSIGNED.as_bytes());
    let e = builder
        .write_signed(&repo, "gpgsig", |_| {
            Err(Error::new(ErrorCode::AUTH, ErrorClass::SSH, "no key"))
        })
        .unwrap_err();
    assert_eq!(
        (e.code(), e.class(), e.message_bytes()),
        (ErrorCode::AUTH, ErrorClass::SSH, &b"no key"[..])
    );
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = builder.write_signed(&repo, "gpgsig", |_| panic!("signer"));
    }));
    assert!(r.is_err());
    assert!(
        builder
            .write_signed(&repo, "gpgsig", |_| Ok(Some(b"a\0b".to_vec())))
            .is_err()
    );

    // `git_commit_extract_signature`.
    let (sig, data) = Commit::extract_signature(&repo, &id, None).unwrap();
    assert_eq!(sig, GPGSIG.as_bytes());
    assert_eq!(data, UNSIGNED.as_bytes());
    let (sig2, _) = Commit::extract_signature(&repo, &id, Some(b"gpgsig")).unwrap();
    assert_eq!(sig2, sig);
    let e = Commit::extract_signature(&repo, &unsigned_id, None).unwrap_err();
    assert_eq!(
        (e.code(), e.class()),
        (ErrorCode::NOT_FOUND, ErrorClass::OBJECT)
    );
    assert_eq!(e.message(), "this commit is not signed");
    // A tree is not a commit.
    let e = Commit::extract_signature(
        &repo,
        &oid("4b825dc642cb6eb9a060e54bf8d69288fbee4904"),
        None,
    )
    .unwrap_err();
    assert_eq!(
        (e.code(), e.class()),
        (ErrorCode::NOT_FOUND, ErrorClass::INVALID)
    );

    // `git_commit_create_with_signature`.
    let one =
        Commit::create_with_signature(&repo, UNSIGNED, Some(GPGSIG.as_bytes()), Some(b"gpgsig"))
            .unwrap();
    let two =
        Commit::create_with_signature(&repo, UNSIGNED, Some(GPGSIG.as_bytes()), None).unwrap();
    assert_eq!(one, two);
    assert_eq!(one, id);
    let magic = Commit::create_with_signature(
        &repo,
        UNSIGNED,
        Some(b"magic word: pretty please"),
        Some(b"magicsig"),
    )
    .unwrap();
    let (head, msg) = UNSIGNED.split_once("\n\n").unwrap();
    assert_eq!(
        odb.read(&magic).unwrap().data(),
        format!("{head}\nmagicsig magic word: pretty please\n\n{msg}").as_bytes()
    );
    let plain = Commit::create_with_signature(&repo, UNSIGNED, None, None).unwrap();
    assert_eq!(plain, unsigned_id);
    let badtree = UNSIGNED.replace(
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
        "6b79e22d69bf46e289df0345a14ca059dfc9bdf6",
    );
    let e =
        Commit::create_with_signature(&repo, badtree, Some(b"s"), Some(b"magicsig")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    let badparent = UNSIGNED.replace(
        "8496071c1b46c854b31185ea97743be6a8774479",
        "34734e478d6cf50c27c9d69026d93974d052c454",
    );
    let e =
        Commit::create_with_signature(&repo, badparent, Some(b"s"), Some(b"magicsig")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert!(Commit::create_with_signature(&repo, "a\0b", None, None).is_err());
}

fn g_author() -> Signature {
    Signature::new(
        "Edward Thomson",
        "ethomson@edwardthomson.com",
        Time::new(123456789, 60),
    )
    .unwrap()
}

fn g_committer() -> Signature {
    Signature::new(
        "libgit2 user",
        "nobody@noreply.libgit2.org",
        Time::new(987654321, 90),
    )
    .unwrap()
}

/// `commit/create.c` and `commit/amend.c`'s edits of testrepo2.
fn stage_changes(s: &Sandbox, repo: &Repository) -> Oid {
    s.write("newfile.txt", "This is a new file.\n");
    s.write("newfile2.txt", "This is a new file.\n");
    s.write("README", "hello, world.\n");
    s.write("new.txt", "hi there.\n");
    let index = repo.index().unwrap();
    index.add_path("newfile2.txt").unwrap();
    index.add_path("README").unwrap();
    index.write().unwrap();
    index.write_tree().unwrap()
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn create_from_stage_and_tree() {
    let (author, committer) = (g_author(), g_committer());

    // from_stage_simple
    let s = Sandbox::workdir("from_stage", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    stage_changes(&s, &repo);
    let opts = CommitCreateOptions::new()
        .author(&author)
        .committer(&committer);
    let id = Commit::create_from_stage(&repo, "This is the message.", opts).unwrap();
    assert_eq!(id, oid("241b5b04e847bc38dd7b4b9f49f21e55da40f3a6"));
    assert_eq!(
        head_tree(&repo),
        oid("b27210772d0633870b4f486d04ed3eb5ebbef5e7")
    );
    drop(repo);

    // from_stage_nochanges
    let s = Sandbox::workdir("from_stage_nochanges", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    let e = Commit::create_from_stage(
        &repo,
        "Message goes here.",
        CommitCreateOptions::new()
            .author(&author)
            .committer(&committer),
    )
    .unwrap_err();
    assert_eq!(e.code(), ErrorCode::UNCHANGED);
    assert_eq!(e.message(), "no changes are staged for commit");
    let id = Commit::create_from_stage(
        &repo,
        "Message goes here.",
        CommitCreateOptions::new()
            .author(&author)
            .committer(&committer)
            .allow_empty(true),
    )
    .unwrap();
    assert_eq!(id, oid("f776dc4c7fd8164b7127dc8e4f9b44421cb01b56"));
    assert_eq!(
        head_tree(&repo),
        oid("c4dc1555e4d4fa0e0c9c3fc46734c7c35b3ce90b")
    );
    drop(repo);

    // from_stage_newrepo: no parents.
    let d = std::env::temp_dir().join(format!("l_commit_api_{}_newrepo", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let newrepo = Repository::init(&d, false).unwrap();
    assert!(newrepo.commit_parents().unwrap().is_empty());
    std::fs::write(d.join("hello.txt"), "hello, world.\n").unwrap();
    std::fs::write(d.join("hi.txt"), "hi there.\n").unwrap();
    std::fs::write(d.join("foo.txt"), "bar.\n").unwrap();
    let index = newrepo.index().unwrap();
    index.add_path("hello.txt").unwrap();
    index.add_path("foo.txt").unwrap();
    index.write().unwrap();
    let id = Commit::create_from_stage(
        &newrepo,
        "Initial commit.",
        CommitCreateOptions::new()
            .author(&author)
            .committer(&committer),
    )
    .unwrap();
    let c = Commit::lookup(&newrepo, &id).unwrap();
    assert_eq!(head_id(&newrepo), id);
    assert_eq!(
        c.tree_id(),
        &oid("b2fa96a4f191c76eb172437281c66aa29609dcaa")
    );
    assert_eq!(c.parent_count(), 0);
    drop(c);
    drop(index);
    drop(newrepo);
    let _ = std::fs::remove_dir_all(&d);

    // from_tree
    let s = Sandbox::workdir("from_tree", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    let tree_id = stage_changes(&s, &repo);
    assert_eq!(tree_id, oid("b27210772d0633870b4f486d04ed3eb5ebbef5e7"));
    let tree = Tree::lookup(&repo, &tree_id).unwrap();
    let parents_before = repo.commit_parents().unwrap();
    assert_eq!(parents_before.len(), 1);
    let id = Commit::create_from_tree(
        &repo,
        &tree,
        "This is the message.",
        CommitCreateOptions::new()
            .author(&author)
            .committer(&committer),
    )
    .unwrap();
    assert_eq!(id, oid("241b5b04e847bc38dd7b4b9f49f21e55da40f3a6"));
    assert_eq!(head_tree(&repo), tree_id);
    let c = Commit::lookup(&repo, &id).unwrap();
    assert_eq!(c.parent_ids(), [*parents_before[0].id()]);
    // The same tree again is empty (`create_from_tree` folds the code to
    // -1, unlike `create_from_stage`).
    let e =
        Commit::create_from_tree(&repo, &tree, "again", CommitCreateOptions::new()).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "no changes are staged for commit");
    // A tree of another repository (or of none) is refused.
    let mut raw = b"100644 a\0".to_vec();
    raw.extend_from_slice(&[1; 20]);
    let parsed = Tree::parse(&raw, OidType::Sha1).unwrap();
    let e = Commit::create_from_tree(&repo, &parsed, "m", CommitCreateOptions::new()).unwrap_err();
    assert_eq!(
        e.message(),
        "invalid argument: 'git_tree_owner(tree) == repo'"
    );
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn create_with_options() {
    let s = Sandbox::workdir("create_opts", "testrepo2");
    let (author, committer) = (g_author(), g_committer());
    // Default signatures: the repository's (user.name / user.email).
    let cfg = s.0.join(".git/config");
    let mut text = std::fs::read(&cfg).unwrap();
    text.extend_from_slice(b"[user]\n\tname = Repo User\n\temail = repo@example.com\n");
    std::fs::write(&cfg, text).unwrap();
    let repo = Repository::open(&s.0).unwrap();
    let tree_id = stage_changes(&s, &repo);
    let tree = Tree::lookup(&repo, &tree_id).unwrap();

    // Headers, encoding and a signer.
    let mut content = Vec::new();
    let id = Commit::create_from_tree(
        &repo,
        &tree,
        "signed\n",
        CommitCreateOptions::new()
            .committer(&committer)
            .encoding(b"ISO-8859-1")
            .header(b"x-one", b"1")
            .signer(b"gpgsig", |c| {
                content = c.to_vec();
                Ok(Some(b"SIG\nNATURE".to_vec()))
            }),
    )
    .unwrap();
    assert_eq!(head_id(&repo), id);
    let c = Commit::lookup(&repo, &id).unwrap();
    assert_eq!(c.author().name(), b"Repo User");
    assert_eq!(c.author().email(), b"repo@example.com");
    assert_eq!(c.committer(), committer.as_ref());
    assert_eq!(c.message_encoding(), Some(&b"ISO-8859-1"[..]));
    assert_eq!(c.header_field("x-one").unwrap(), b"1");
    let (sig, data) = Commit::extract_signature(&repo, &id, None).unwrap();
    assert_eq!(sig, b"SIG\nNATURE");
    assert_eq!(data, content);

    // An erroring signer fails the commit; HEAD stays.
    let e = Commit::amend_from_tree(
        &repo,
        &tree,
        Some(b"x"),
        CommitCreateOptions::new()
            .author(&author)
            .signer(b"gpgsig", |_| {
                Err(Error::new(ErrorCode::AUTH, ErrorClass::SSH, "nope"))
            }),
    )
    .unwrap_err();
    assert_eq!(
        (e.code(), e.message_bytes()),
        (ErrorCode::AUTH, &b"nope"[..])
    );
    assert_eq!(head_id(&repo), id);
    // A panicking one is resumed.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = Commit::amend_from_tree(
            &repo,
            &tree,
            None,
            CommitCreateOptions::new().signer(b"gpgsig", |_| panic!("signer")),
        );
    }));
    assert!(r.is_err());
    assert_eq!(head_id(&repo), id);
    // NUL bytes are refused.
    let e = Commit::create_from_tree(&repo, &tree, "a\0b", CommitCreateOptions::new()).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn amend_from_stage_and_tree() {
    let committer = g_committer();
    let opts = || CommitCreateOptions::new().committer(&committer);

    // from_stage_simple
    let s = Sandbox::workdir("amend_stage", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    stage_changes(&s, &repo);
    let id = Commit::amend_from_stage(&repo, None, opts()).unwrap();
    assert_eq!(id, oid("63ec0b083fd14c22a68fd2b1794f26e4b396b6b3"));
    assert_eq!(
        head_tree(&repo),
        oid("b27210772d0633870b4f486d04ed3eb5ebbef5e7")
    );
    drop(repo);

    // from_stage_newmessage
    let s = Sandbox::workdir("amend_newmessage", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    let id = Commit::amend_from_stage(&repo, Some(b"New message goes here."), opts()).unwrap();
    assert_eq!(id, oid("8b0e1cacc8380023705192466aaef8a15ddae7b3"));
    assert_eq!(
        head_tree(&repo),
        oid("c4dc1555e4d4fa0e0c9c3fc46734c7c35b3ce90b")
    );
    drop(repo);

    // from_stage_nochanges
    let s = Sandbox::workdir("amend_nochanges", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    let id = Commit::amend_from_stage(&repo, None, opts()).unwrap();
    assert_eq!(id, oid("da86907c6d505a92c5683bece08f23d68ac785bd"));
    drop(repo);

    // from_tree
    let s = Sandbox::workdir("amend_tree", "testrepo2");
    let repo = Repository::open(&s.0).unwrap();
    let tree_id = stage_changes(&s, &repo);
    let tree = Tree::lookup(&repo, &tree_id).unwrap();
    let id = Commit::amend_from_tree(&repo, &tree, None, opts()).unwrap();
    assert_eq!(id, oid("63ec0b083fd14c22a68fd2b1794f26e4b396b6b3"));
    assert_eq!(head_tree(&repo), tree_id);
}

/// `object/commit/commitstagedfile.c`: `amend_commit`, on testrepo.git.
#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn amend() {
    let s = Sandbox::bare("amend", "testrepo.git");
    let repo = Repository::open(&s.0).unwrap();
    let old = Commit::lookup(&repo, &head_id(&repo)).unwrap();
    let id = old
        .amend(
            &AmendOptions::new()
                .update_ref(b"HEAD")
                .message(b"Initial commit"),
        )
        .unwrap();
    // The old commit is no longer the tip.
    let e = old
        .amend(
            &AmendOptions::new()
                .update_ref(b"HEAD")
                .message(b"Initial commit"),
        )
        .unwrap_err();
    assert_eq!(
        e.message(),
        "commit to amend is not the tip of the given branch"
    );
    let new = Commit::lookup(&repo, &id).unwrap();
    assert_eq!(head_id(&repo), id);
    assert_eq!(new.message(), b"Initial commit");
    assert_eq!(new.parent_ids(), old.parent_ids());
    assert_eq!(new.tree_id(), old.tree_id());
    assert_eq!(new.author(), old.author());
    assert_eq!(new.committer(), old.committer());

    // A new tree, author and committer; a missing ref.
    let other = Tree::lookup(&repo, &oid("ae90f12eea699729ed24555e40b9fd669da12a12")).unwrap();
    let me = Signature::new("Me", "me@example.com", Time::new(1, 0)).unwrap();
    let e = new
        .amend(
            &AmendOptions::new()
                .update_ref(b"refs/heads/nope")
                .tree(&other),
        )
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let id2 = new
        .amend(
            &AmendOptions::new()
                .update_ref(b"HEAD")
                .tree(&other)
                .author(&me)
                .committer(me.as_ref())
                .encoding(b"UTF-8"),
        )
        .unwrap();
    let c2 = Commit::lookup(&repo, &id2).unwrap();
    assert_eq!(c2.tree_id(), other.id());
    assert_eq!(c2.author(), me.as_ref());
    assert_eq!(c2.message(), b"Initial commit");
    assert_eq!(c2.message_encoding(), Some(&b"UTF-8"[..]));
    assert_eq!(head_id(&repo), id2);
    // No reference: only the object is written.
    let id3 = c2.amend(&AmendOptions::new().message(b"loose")).unwrap();
    assert_eq!(head_id(&repo), id2);
    assert_eq!(Commit::lookup(&repo, &id3).unwrap().message(), b"loose");
}

/// `repo/getters.c`: `commit_parents`.
#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn commit_parents() {
    let s = Sandbox::workdir("parents", "testrepo");
    let repo = Repository::open(&s.0).unwrap();
    let p = repo.commit_parents().unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].id(), &oid("099fabac3a9ea935598528c27f866e34089c2eff"));
    s.write(
        ".git/MERGE_HEAD",
        "8496071c1b46c854b31185ea97743be6a8774479\n\
         5b5b025afb0b4c913b4c338a42934a3863bf3644\n\
         4a202b346bb0fb0db7eff3cffeb3c70babbd2045\n\
         9fd738e8f7967c078dceed8190330fc8648ee56a\n",
    );
    let p = repo.commit_parents().unwrap();
    let ids: Vec<Oid> = p.iter().map(|c| *c.id()).collect();
    assert_eq!(
        ids,
        [
            oid("099fabac3a9ea935598528c27f866e34089c2eff"),
            oid("8496071c1b46c854b31185ea97743be6a8774479"),
            oid("5b5b025afb0b4c913b4c338a42934a3863bf3644"),
            oid("4a202b346bb0fb0db7eff3cffeb3c70babbd2045"),
            oid("9fd738e8f7967c078dceed8190330fc8648ee56a"),
        ]
    );
}
