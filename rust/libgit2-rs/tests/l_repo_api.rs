//! The safe Rust API of repositories, library init and options (WP l-repo).
//! It reaches the C that stays (config, refs, odb), so it runs with
//! `--features link-c` and not under Miri.
#![cfg(all(feature = "link-c", not(miri)))]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use libgit2_rs::repository::api::{RefStorage, RepositoryInitFlags, RepositoryItem};
use libgit2_rs::settings::opts;
use libgit2_rs::{
    ErrorCode, Libgit2, OidType, Repository, RepositoryInitOptions, RepositoryOpenFlags,
    RepositoryState,
};

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("l-repo-api-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn slash(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push("/");
    PathBuf::from(s)
}

#[test]
fn init_open_discover() {
    let _lib = Libgit2::init().unwrap();
    let d = tmpdir("basic");
    let w = d.join("w");
    let repo = Repository::init(&w, false).unwrap();
    assert_eq!(repo.path(), slash(&w.join(".git")));
    assert_eq!(repo.workdir(), Some(slash(&w).as_path()));
    assert!(!repo.is_bare());
    assert!(!repo.is_worktree());
    assert!(repo.is_empty().unwrap());
    assert!(repo.head_unborn().unwrap());
    assert!(!repo.head_detached().unwrap());
    assert_eq!(repo.state().unwrap(), RepositoryState::Clean);
    assert_eq!(repo.oid_type(), OidType::Sha1);
    assert!(!repo.is_shallow().unwrap());
    let e = repo.head().unwrap_err();
    assert_eq!(e.code(), ErrorCode::UNBORN_BRANCH);
    assert_eq!(
        repo.item_path(RepositoryItem::Objects).unwrap(),
        slash(&w.join(".git/objects"))
    );
    drop(repo);

    std::fs::create_dir_all(w.join("x/y")).unwrap();
    let found = Repository::discover(w.join("x/y"), false, &[]).unwrap();
    assert_eq!(found, slash(&w.join(".git")));
    let e = Repository::discover(w.join("x/y"), false, &[&w.join("x")]).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);

    let r = Repository::open_ext(Some(&w.join("x/y")), RepositoryOpenFlags::empty(), &[]).unwrap();
    assert_eq!(r.path(), slash(&w.join(".git")));
    let e = Repository::open(w.join("x/y")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert!(e.message().starts_with("could not find repository at"));
    let b = Repository::open_bare(w.join(".git")).unwrap();
    assert!(b.is_bare());
    assert_eq!(b.workdir(), None);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn init_options_builder() {
    let _lib = Libgit2::init().unwrap();
    let d = tmpdir("opts");
    let mut o = RepositoryInitOptions::new();
    o.bare(true)
        .initial_head("main")
        .description("desc\n")
        .oid_type(OidType::Sha256)
        .ref_storage(RefStorage::Files);
    let repo = Repository::init_opts(d.join("b.git"), &o).unwrap();
    assert!(repo.is_bare());
    assert_eq!(repo.oid_type(), OidType::Sha256);
    assert_eq!(
        std::fs::read_to_string(d.join("b.git/HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
    let mut again = RepositoryInitOptions::new();
    again.flags(RepositoryInitFlags::BARE | RepositoryInitFlags::NO_REINIT);
    let e = Repository::init_opts(d.join("b.git"), &again).unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    // A path with an embedded NUL is rejected, not truncated.
    assert!(Repository::open("a\0b").is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn lazy_owners_and_setters() {
    let _lib = Libgit2::init().unwrap();
    let d = tmpdir("owners");
    let mut repo = Repository::init(d.join("r"), false).unwrap();
    let odb = repo.odb().unwrap();
    let odb2 = repo.odb().unwrap();
    drop(odb2);
    let _cfg = repo.config().unwrap();
    let _snap = repo.config_snapshot().unwrap();
    let _refdb = repo.refdb().unwrap();
    let index = repo.index().unwrap();
    // The owners outlive the repository.
    let wrapped = Repository::wrap_odb(&odb).unwrap();
    // As C: a wrapped odb is not marked bare, and has no paths.
    assert!(!wrapped.is_bare());
    assert_eq!(wrapped.workdir(), None);
    assert_eq!(wrapped.path(), Path::new(""));
    drop(wrapped);

    repo.set_namespace(Some(b"ns")).unwrap();
    assert_eq!(repo.namespace(), Some(&b"ns"[..]));
    assert!(repo.set_namespace(Some(b"a\0b")).is_err());
    repo.set_namespace(None).unwrap();
    assert_eq!(repo.namespace(), None);
    repo.set_ident(Some("Me"), Some("me@example.com")).unwrap();
    assert_eq!(
        repo.ident(),
        (Some(&b"Me"[..]), Some(&b"me@example.com"[..]))
    );

    repo.set_bare().unwrap();
    assert!(repo.is_bare());
    repo.set_workdir(d.join("r"), false).unwrap();
    assert!(!repo.is_bare());
    repo.set_index(None);
    repo.set_index(Some(&index));
    drop(repo);
    drop(index);
    drop(odb);

    let mem = Repository::new_in_memory(OidType::Sha1).unwrap();
    assert!(mem.is_bare());
    assert!(mem.item_path(RepositoryItem::Config).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn state_and_message() {
    let _lib = Libgit2::init().unwrap();
    let d = tmpdir("state");
    let repo = Repository::init(d.join("s"), false).unwrap();
    let g = d.join("s/.git");
    std::fs::write(g.join("MERGE_HEAD"), "x\n").unwrap();
    std::fs::write(g.join("MERGE_MSG"), "msg\n").unwrap();
    assert_eq!(repo.state().unwrap(), RepositoryState::Merge);
    assert_eq!(repo.message().unwrap(), b"msg\n");
    repo.remove_message().unwrap();
    assert_eq!(repo.message().unwrap_err().code(), ErrorCode::NOT_FOUND);
    std::fs::create_dir_all(g.join("rebase-apply")).unwrap();
    std::fs::remove_file(g.join("MERGE_HEAD")).unwrap();
    assert_eq!(repo.state().unwrap(), RepositoryState::ApplyMailboxOrRebase);
    repo.state_cleanup().unwrap();
    assert_eq!(repo.state().unwrap(), RepositoryState::Clean);

    repo.set_head("refs/heads/topic").unwrap();
    assert!(repo.head_unborn().unwrap());
    assert_eq!(
        repo.set_head("refs/tags/v1").unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );

    // `hashfile` of a file in the working directory.
    std::fs::write(d.join("s/f.txt"), "hello\n").unwrap();
    let id = repo
        .hashfile(d.join("s/f.txt"), 3, Some(Path::new("")))
        .unwrap();
    assert_eq!(
        id.hex().as_str(),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn options() {
    let _lib = Libgit2::init().unwrap();
    assert!(opts::user_agent().starts_with(b"libgit2 "));
    assert_eq!(opts::user_agent_product(), b"git/2.0");
    assert!(opts::extensions().iter().any(|e| e == b"objectformat"));
    let (maj, min, rev) = libgit2_rs::libgit2::version();
    assert_eq!((maj, min, rev), (1, 9, 0));
    assert_eq!(
        libgit2_rs::libgit2::feature_backend(libgit2_rs::libgit2::feature::THREADS),
        Some(c"pthread")
    );
}
