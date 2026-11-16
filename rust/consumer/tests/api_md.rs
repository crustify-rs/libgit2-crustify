//! The examples of `rust/API.md`, compiled (not run: they name paths that
//! need not exist), so the guide stays true to the API.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use libgit2_rs::{
    Commit, CommitBuilder, ConfigLevel, ErrorClass, ErrorCode, IndexEntry, Object, ObjectType,
    Reference, Repository, Signature, Tree, opts,
};

fn errors() -> libgit2_rs::Result<()> {
    let repo = Repository::open("/path/to/repo")?;
    match Reference::lookup(&repo, "refs/heads/nope") {
        Err(e) if e.code() == ErrorCode::NOT_FOUND => {
            assert_eq!(e.class(), ErrorClass::REFERENCE);
            println!("{e}");
        }
        other => {
            other?;
        }
    }
    Ok(())
}

fn objects() -> libgit2_rs::Result<()> {
    let repo = Repository::open("/path/to/repo")?;
    let head = repo.head()?;
    let commit = Commit::lookup(&repo, &head.target().unwrap())?;
    println!(
        "{} by {}",
        String::from_utf8_lossy(commit.summary()),
        commit.author()
    );
    let tree: Tree<'_> = commit.tree()?;
    for entry in tree.iter() {
        println!(
            "{:o} {}",
            entry.filemode(),
            String::from_utf8_lossy(entry.name())
        );
    }
    let obj = Object::lookup(&repo, commit.id(), None)?;
    let _commit: Commit<'_> = Commit::try_from(obj).expect("a commit");
    Ok(())
}

fn create() -> libgit2_rs::Result<()> {
    let repo = Repository::init("/tmp/new-repo", false)?;
    let blob = repo.odb()?.write(b"hello\n", ObjectType::Blob)?;
    let index = repo.index()?;
    index.add(&IndexEntry::new("hello.txt", 0o100644, blob))?;
    let tree = index.write_tree()?;
    let me = Signature::now("Me", "me@example.com")?;
    let _id = CommitBuilder::new(tree, &me, &me, b"first\n")
        .update_ref(b"HEAD")
        .write(&repo)?;
    Ok(())
}

fn threads() -> libgit2_rs::Result<()> {
    let repo = Repository::open("/path/to/repo")?;
    let index = repo.index()?;
    std::thread::scope(|s| {
        s.spawn(|| index.len());
        s.spawn(|| repo.head().map(|h| h.target()));
    });
    Ok(())
}

fn options() -> libgit2_rs::Result<()> {
    opts::set_mwindow_mapped_limit(256 << 20);
    opts::enable_strict_object_creation(false);
    opts::set_search_path(ConfigLevel::Global, Some("/etc/my-git"))?;
    Ok(())
}

#[test]
fn compiles() {}
