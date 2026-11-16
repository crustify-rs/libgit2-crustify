//! The safe Rust API of commits and trees, used without `unsafe`
//! (PORTING.md §5). Reads `tests/resources/testrepo.git`; writes into a
//! copy of it. Expected values are the C reference's.

#![forbid(unsafe_code)]
#![cfg(feature = "link-c")]

use libgit2_rs::Repository;
use libgit2_rs::commit::api::SignatureRef;
use libgit2_rs::tree::api::{EntryKind, TreeUpdate, TreeWalk, TreeWalkMode};
use libgit2_rs::util::test_support;
use libgit2_rs::{Commit, CommitBuilder, ErrorCode, Oid, OidType, Tree, TreeBuilder};
use std::path::{Path, PathBuf};

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn oid(s: &str) -> Oid {
    Oid::from_str_hex(s, OidType::Sha1).unwrap()
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

/// A writable copy of testrepo.git.
fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("l_commit_api_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&resources().join("testrepo.git"), &dir);
    dir
}

#[test]
fn commits() {
    let _g = test_support::init();
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let head = Commit::lookup(&repo, &oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750")).unwrap();
    assert_eq!(head.id(), oid("a65fedf39aefe402d3bb6e24df4d4f5fe4547750"));
    assert_eq!(
        head.tree_id(),
        oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")
    );
    assert_eq!(
        head.parent_ids(),
        [oid("be3563ae3f795b2b4353bcce3a527ad0a4f7f644")]
    );
    let a = head.author();
    assert_eq!(
        (a.name, a.email, a.time, a.offset),
        (
            &b"Scott Chacon"[..],
            &b"schacon@gmail.com"[..],
            1312943626,
            -420
        )
    );

    let p = head.parent(0).unwrap();
    assert_eq!(p.summary(), b"Merge branch 'br2'");
    assert_eq!(p.body(), None);
    assert_eq!(p.message(), b"Merge branch 'br2'\n");
    assert_eq!(p.committer().time, 1274813907);
    assert_eq!(p.time_offset(), -420);
    assert_eq!(p.parent_count(), 2);
    assert_eq!(
        p.header_field(b"tree").unwrap(),
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
        oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")
    );
}

#[test]
fn parsed_commits() {
    let _g = test_support::init();
    let data = b"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nauthor A <a@b> 1 +0100\ncommitter C <c@d> 2 -0000\nencoding UTF-8\n\nSum\nmary\n\n  Body  \n";
    let c = Commit::parse(data, OidType::Sha1).unwrap();
    assert_eq!(c.summary(), b"Sum mary");
    assert_eq!(c.body(), Some(&b"Body"[..]));
    assert_eq!(c.message_encoding(), Some(&b"UTF-8"[..]));
    assert_eq!(c.committer().sign, b'-');
    assert!(c.tree().is_err());
    assert!(c.parent(0).is_err() && c.nth_gen_ancestor(1).is_err());
    let e = Commit::parse(b"tree x\n", OidType::Sha1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID);
    assert_eq!(e.message(), "failed to parse bad commit object");
}

#[test]
fn trees_and_walks() {
    let _g = test_support::init();
    let repo = Repository::open(resources().join("testrepo.git")).unwrap();
    let t = Tree::lookup(&repo, &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")).unwrap();
    let names: Vec<_> = t.iter().map(|e| e.name().to_vec()).collect();
    assert_eq!(names, [&b"README"[..], b"branch_file.txt", b"new.txt"]);
    let e = t.get_name(b"new.txt").unwrap();
    assert_eq!(
        (e.id(), e.filemode(), e.kind()),
        (
            oid("a71586c1dfe8a71c6cbf6c129f404c5642ff31bd"),
            0o100644,
            EntryKind::Blob
        )
    );
    assert!(t.get_name(b"nope").is_none());
    assert_eq!(t.get_id(&e.id()).unwrap().name(), b"new.txt");

    // The "subtrees" tree.
    let st = Tree::lookup(&repo, &oid("ae90f12eea699729ed24555e40b9fd669da12a12")).unwrap();
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
        if e.kind() == EntryKind::Tree {
            TreeWalk::Skip
        } else {
            TreeWalk::Continue
        }
    })
    .unwrap();
    assert_eq!(skipped, 4);

    let deep = st.get_path(b"ab/de/fgh/1.txt").unwrap();
    assert_eq!(deep.as_ref().name(), b"1.txt");
    let cloned = deep.clone();
    assert_eq!(cloned.as_ref().id(), deep.as_ref().id());
    assert_eq!(
        st.get_path(b"ab/x").unwrap_err().message(),
        "the path 'x' does not exist in the given tree"
    );

    // Parsed from bytes: the entries borrow them.
    let mut raw = b"100644 a\0".to_vec();
    raw.extend_from_slice(&[1; 20]);
    let pt = Tree::parse(&raw, OidType::Sha1).unwrap();
    assert_eq!(pt.get(0).unwrap().name(), b"a");
    assert_eq!(
        Tree::parse(b"100644 a", OidType::Sha1)
            .unwrap_err()
            .message(),
        "failed to parse tree: object is corrupted"
    );
}

#[test]
fn builders_and_writes() {
    let _g = test_support::init();
    let dir = sandbox("write");
    let repo = Repository::open(&dir).unwrap();
    let base = Tree::lookup(&repo, &oid("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162")).unwrap();
    let readme = base.get_name(b"README").unwrap().id();

    let mut b = TreeBuilder::new(&repo, Some(&base)).unwrap();
    assert_eq!(b.len(), 3);
    // Writing it back gives the same tree.
    assert_eq!(b.write().unwrap(), base.id());
    b.insert(b"copy", &readme, 0o100644).unwrap();
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
    assert_eq!(b.get(b"copy").unwrap().id(), readme);
    b.remove(b"new.txt").unwrap();
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
    assert_eq!(ut.get_path(b"dir/sub/file").unwrap().as_ref().id(), readme);
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
    let sig = SignatureRef::new(b"Me", b"me@example.com", 1700000000, 60);
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
