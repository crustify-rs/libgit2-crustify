//! The safe Rust API of repositories (WP l-repo, WP api), used as a
//! consumer would: root items only, no `unsafe`, no explicit init. It
//! reaches the C that stays (config, refs, worktree, fetchhead, merge), so
//! it runs with `--features link-c` and not under Miri.
#![cfg(all(feature = "link-c", not(miri)))]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use libgit2_rs::{
    Backend, Config, ConfigLevel, ErrorClass, ErrorCode, Index, IndexEntry, Mempack, ObjectType,
    Odb, Oid, OidType, RefStorage, Repository, RepositoryInitFlags, RepositoryInitOptions,
    RepositoryItem, RepositoryOpenFlags, RepositoryState,
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

/// An odb with an in-memory backend, made alone.
fn mem_odb(t: OidType) -> (Odb<'static>, Mempack) {
    let odb = Odb::with_oid_type(t).unwrap();
    let (backend, pack) = Backend::mempack();
    odb.add_backend(backend, 1).unwrap();
    (odb, pack)
}

fn oid(hex: &str) -> Oid {
    Oid::from_hex(hex, OidType::Sha1).unwrap()
}

const SHARED: &str = "the odb is shared: it has another handle, or a repository owns it";

#[test]
fn init_open_discover() {
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
    let e = Repository::open_ext(Some(&w.join("x/y")), RepositoryOpenFlags::NO_SEARCH, &[])
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
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
    let d = tmpdir("opts");
    let o = RepositoryInitOptions::new()
        .bare(true)
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
    assert_eq!(
        std::fs::read_to_string(d.join("b.git/description")).unwrap(),
        "desc\n"
    );
    // `flags` replaces, `bare` toggles one flag
    let again = RepositoryInitOptions::new()
        .flags(RepositoryInitFlags::BARE | RepositoryInitFlags::NO_REINIT)
        .bare(false)
        .bare(true);
    let e = Repository::init_opts(d.join("b.git"), &again).unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    // without MKPATH, a missing parent is an error
    let e = Repository::init_opts(
        d.join("no/such/dir"),
        &RepositoryInitOptions::new().flags(RepositoryInitFlags::empty()),
    )
    .unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    // `Default` is `new` (MKPATH)
    let deep = Repository::init_opts(d.join("m/k/p"), &RepositoryInitOptions::default()).unwrap();
    assert!(!deep.is_bare());
    // A path with an embedded NUL is rejected, not truncated.
    let e = Repository::open("a\0b").unwrap_err();
    assert_eq!(e.message(), "path contains a NUL byte");
    assert_eq!(e.class(), ErrorClass::INVALID);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn lazy_owners_and_setters() {
    let d = tmpdir("owners");
    let mut repo = Repository::init(d.join("r"), false).unwrap();
    {
        // The owners borrow the repository, and are usable together.
        let odb = repo.odb().unwrap();
        let odb2 = repo.odb().unwrap();
        drop(odb2);
        let _cfg = repo.config().unwrap();
        let _snap = repo.config_snapshot().unwrap();
        let _refdb = repo.refdb().unwrap();
        let index = repo.index().unwrap();
        let _clone = index.clone();
        assert!(repo.head_unborn().unwrap());
        drop(odb);
    }

    repo.set_namespace(Some(b"ns")).unwrap();
    assert_eq!(repo.namespace(), Some(&b"ns"[..]));
    let e = repo.set_namespace(Some(b"a\0b")).unwrap_err();
    assert_eq!(e.message(), "namespace contains a NUL byte");
    repo.set_namespace(None).unwrap();
    assert_eq!(repo.namespace(), None);
    repo.set_ident(Some(b"Me"), Some(b"me@example.com"))
        .unwrap();
    // `ident` gives back what `set_ident` took
    assert_eq!(
        repo.ident(),
        (Some(&b"Me"[..]), Some(&b"me@example.com"[..]))
    );
    repo.set_ident(None, Some(b"x@y")).unwrap();
    assert_eq!(repo.ident(), (None, Some(&b"x@y"[..])));
    let e = repo.set_ident(Some(b"a\0b"), None).unwrap_err();
    assert_eq!(e.message(), "name contains a NUL byte");
    let e = repo.set_head("refs/heads/a\0b").unwrap_err();
    assert_eq!(e.message(), "reference name contains a NUL byte");
    // names as bytes or strings
    repo.set_head(b"refs/heads/bytes").unwrap();
    repo.set_head(String::from("refs/heads/string")).unwrap();

    repo.set_bare().unwrap();
    assert!(repo.is_bare());
    repo.set_workdir(d.join("r"), false).unwrap();
    assert!(!repo.is_bare());
    // A repository moves to another thread and drops there.
    std::thread::spawn(move || drop(repo)).join().unwrap();

    let mem = Repository::new_in_memory(OidType::Sha1).unwrap();
    assert!(mem.is_bare());
    assert!(mem.item_path(RepositoryItem::Config).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn state_and_message() {
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
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn hashfile() {
    let d = tmpdir("hashfile");
    let repo = Repository::init(d.join("h"), false).unwrap();
    std::fs::write(d.join("h/f.txt"), "hello\n").unwrap();
    // unfiltered (`Some("")`)
    let id = repo
        .hashfile(d.join("h/f.txt"), ObjectType::Blob, Some(b""))
        .unwrap();
    assert_eq!(id.to_string(), "ce013625030ba8dba906f756967f9e9ca394464a");
    // filtered as its own path (no filter applies here)
    let own = repo
        .hashfile(d.join("h/f.txt"), ObjectType::Blob, None)
        .unwrap();
    assert_eq!(own, id);
    // with CRLF conversion from the attributes
    std::fs::write(d.join("h/.gitattributes"), "*.crlf text eol=crlf\n").unwrap();
    std::fs::write(d.join("h/x.crlf"), "a\r\nb\r\n").unwrap();
    let filtered = repo
        .hashfile(d.join("h/x.crlf"), ObjectType::Blob, Some(b"x.crlf"))
        .unwrap();
    let raw = repo
        .hashfile(d.join("h/x.crlf"), ObjectType::Blob, Some(b""))
        .unwrap();
    assert_ne!(filtered, raw);
    assert_eq!(
        filtered,
        Odb::hash(b"a\nb\n", ObjectType::Blob, OidType::Sha1).unwrap()
    );
    // while an index of the repository is in use (the call takes its lock)
    let index = repo.index().unwrap();
    assert!(index.is_empty());
    assert_eq!(
        repo.hashfile(d.join("h/f.txt"), ObjectType::Blob, Some(b""))
            .unwrap(),
        id
    );
    let e = repo
        .hashfile(d.join("h/missing"), ObjectType::Blob, Some(b""))
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    drop(index);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wrap_odb_and_set_odb() {
    let (odb, pack) = mem_odb(OidType::Sha1);
    let id = odb.write(b"in memory\n", ObjectType::Blob).unwrap();
    let repo = Repository::wrap_odb(odb).unwrap();
    // the mempack handle is not a handle of the odb
    assert_eq!(pack.object_count(), 1);
    // as C: no directory, but not marked bare
    assert!(!repo.is_bare());
    assert_eq!(repo.path(), Path::new(""));
    assert_eq!(repo.workdir(), None);
    assert_eq!(repo.oid_type(), OidType::Sha1);
    assert!(repo.odb().unwrap().exists(&id));

    // a SHA-256 odb makes a SHA-256 repository
    let repo256 = Repository::wrap_odb(Odb::with_oid_type(OidType::Sha256).unwrap()).unwrap();
    assert_eq!(repo256.oid_type(), OidType::Sha256);

    // a shared odb is refused: another handle (a clone) is live
    let odb = Odb::new().unwrap();
    let keep = odb.clone();
    let e = Repository::wrap_odb(odb).unwrap_err();
    assert_eq!(e.message(), SHARED);
    assert_eq!(e.class(), ErrorClass::INVALID);
    drop(keep);

    let d = tmpdir("set-odb");
    let mut disk = Repository::init(d.join("r"), true).unwrap();
    let mine = disk
        .odb()
        .unwrap()
        .write(b"on disk\n", ObjectType::Blob)
        .unwrap();
    let (mem, _pack) = mem_odb(OidType::Sha1);
    let other = mem.write(b"other\n", ObjectType::Blob).unwrap();
    disk.set_odb(mem).unwrap();
    let now = disk.odb().unwrap();
    assert!(now.exists(&other));
    assert!(!now.exists(&mine));
    drop(now);

    // the odb of another repository is refused (that repository owns it)
    let mut third = Repository::new_in_memory(OidType::Sha1).unwrap();
    // `repo.odb()` borrows `repo`, so it is not `'static`: only the
    // repository's own clones exist; the type system already refuses them
    // (see the compile_fail doctests), and an unowned clone left behind
    // makes the odb shared:
    let odb = Odb::new().unwrap();
    let clone = odb.clone();
    assert_eq!(third.set_odb(odb).unwrap_err().message(), SHARED);
    third.set_odb(clone).unwrap();
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn set_config_and_index() {
    let d = tmpdir("set-config");
    let mut repo = Repository::init(d.join("r"), false).unwrap();
    let cfg = Config::new();
    cfg.add_memory(b"[user]\n\tname = From Memory\n", ConfigLevel::Local, false)
        .unwrap();
    let level = cfg.clone();
    let e = repo.set_config(cfg).unwrap_err();
    assert_eq!(
        e.message(),
        "the config is shared: it has another handle, or a repository owns it"
    );
    repo.set_config(level).unwrap();
    assert_eq!(
        repo.config().unwrap().get_string("user.name").unwrap(),
        "From Memory"
    );

    let ix = Index::new().unwrap();
    let blob = repo.odb().unwrap().write(b"x\n", ObjectType::Blob).unwrap();
    ix.add(&IndexEntry::new("from-memory", 0o100644, blob))
        .unwrap();
    let it = ix.iter();
    // an iterator holds the index too
    let e = repo.set_index(ix.clone()).unwrap_err();
    assert_eq!(
        e.message(),
        "the index is shared: it has another handle, or a repository owns it"
    );
    drop(it);
    repo.set_index(ix).unwrap();
    let index = repo.index().unwrap();
    assert_eq!(index.len(), 1);
    assert_eq!(index.get(0).unwrap().path, b"from-memory");
    // the repository owns it now: it writes trees into its odb
    let tree = index.write_tree().unwrap();
    assert!(repo.odb().unwrap().exists(&tree));
    drop(index);

    // cleanup drops the replaced objects; they are loaded again from disk
    repo.cleanup().unwrap();
    assert!(repo.index().unwrap().is_empty());
    assert!(repo.config().unwrap().get_string("user.name").is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn submodule_cache() {
    let d = tmpdir("subcache");
    let mut repo = Repository::init(d.join("r"), false).unwrap();
    repo.submodule_cache_all().unwrap();
    repo.submodule_cache_clear().unwrap();
    repo.submodule_cache_clear().unwrap();
    let _ = std::fs::remove_dir_all(&d);
}

const FETCH_HEAD: &str = "49322bb17d3acc9146f98c97d078513228bbf3c0\t\tbranch 'master' of https://github.com/libgit2/TestGitRepository\n\
0966a434eb1a025db6b71485ab63a3bfbea520b6\tnot-for-merge\tbranch 'first-merge' of https://github.com/libgit2/TestGitRepository\n\
d96c4e80345534eccee5ac7b07fc7603b56124cb\tnot-for-merge\ttag 'annotated_tag' of https://github.com/libgit2/TestGitRepository\n\
42e4e7c5e507e113ebbb7801b16b52cf867b7ce1\tnot-for-merge\tremote_url\n";

#[test]
fn fetchhead_foreach() {
    let d = tmpdir("fetchhead");
    let repo = Repository::init(d.join("r"), false).unwrap();
    // no FETCH_HEAD (C: GIT_ENOTFOUND)
    let e = repo.fetchhead_foreach(|_, _, _, _| true).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);

    std::fs::write(d.join("r/.git/FETCH_HEAD"), FETCH_HEAD).unwrap();
    let mut seen = Vec::new();
    repo.fetchhead_foreach(|name, url, id, is_merge| {
        seen.push((
            name.map(<[u8]>::to_vec),
            url.map(<[u8]>::to_vec),
            *id,
            is_merge,
        ));
        true
    })
    .unwrap();
    let url = Some(b"https://github.com/libgit2/TestGitRepository".to_vec());
    assert_eq!(
        seen,
        vec![
            (
                Some(b"refs/heads/master".to_vec()),
                url.clone(),
                oid("49322bb17d3acc9146f98c97d078513228bbf3c0"),
                true
            ),
            (
                Some(b"refs/heads/first-merge".to_vec()),
                url.clone(),
                oid("0966a434eb1a025db6b71485ab63a3bfbea520b6"),
                false
            ),
            (
                Some(b"refs/tags/annotated_tag".to_vec()),
                url,
                oid("d96c4e80345534eccee5ac7b07fc7603b56124cb"),
                false
            ),
            // a description without a name (C's `name_missing`)
            (
                None,
                Some(b"remote_url".to_vec()),
                oid("42e4e7c5e507e113ebbb7801b16b52cf867b7ce1"),
                false
            ),
        ]
    );

    // `false` stops, and the call succeeds
    let mut n = 0;
    repo.fetchhead_foreach(|_, _, _, _| {
        n += 1;
        n < 2
    })
    .unwrap();
    assert_eq!(n, 2);

    // a panic stops and is resumed after the call
    let mut calls = 0;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repo.fetchhead_foreach(|_, _, _, _| {
            calls += 1;
            panic!("in the callback")
        })
    }));
    assert!(r.is_err());
    assert_eq!(calls, 1);

    // an old-style line: no name, no URL
    std::fs::write(
        d.join("r/.git/FETCH_HEAD"),
        "49322bb17d3acc9146f98c97d078513228bbf3c0\n",
    )
    .unwrap();
    repo.fetchhead_foreach(|name, url, _, is_merge| {
        assert_eq!((name, url, is_merge), (None, None, true));
        true
    })
    .unwrap();

    // C's parse errors
    for (data, msg) in [
        (
            "49322bb17d3acc9146f98c97d078513228bbf3c0\tinvalid-merge\t\n",
            "invalid for-merge",
        ),
        (
            "49322bb17d3acc9146f98c97d078513228bbf3c0\tnot-for-merge\n",
            "invalid description",
        ),
        ("unterminated", "no EOL at line 1"),
    ] {
        std::fs::write(d.join("r/.git/FETCH_HEAD"), data).unwrap();
        let e = repo.fetchhead_foreach(|_, _, _, _| true).unwrap_err();
        assert!(e.message().starts_with(msg), "{}", e.message());
        assert_eq!(e.class(), ErrorClass::FETCHHEAD);
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn mergehead_foreach() {
    let d = tmpdir("mergehead");
    let repo = Repository::init(d.join("r"), false).unwrap();
    let e = repo.mergehead_foreach(|_| true).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);

    let ids = [
        "e9b63f7d76d4b3f8c5a7e4d0e1b3d0a5f1c2b3a4",
        "0966a434eb1a025db6b71485ab63a3bfbea520b6",
        "49322bb17d3acc9146f98c97d078513228bbf3c0",
    ];
    std::fs::write(d.join("r/.git/MERGE_HEAD"), ids.join("\n") + "\n").unwrap();
    let mut seen = Vec::new();
    repo.mergehead_foreach(|id| {
        seen.push(*id);
        true
    })
    .unwrap();
    assert_eq!(seen, ids.map(oid).to_vec());

    let mut n = 0;
    repo.mergehead_foreach(|_| {
        n += 1;
        false
    })
    .unwrap();
    assert_eq!(n, 1);

    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repo.mergehead_foreach(|_| panic!("in the callback"))
    }));
    assert!(r.is_err());

    // C's errors: an invalid id, a missing newline
    std::fs::write(d.join("r/.git/MERGE_HEAD"), "invalid-oid\n").unwrap();
    let e = repo.mergehead_foreach(|_| true).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    std::fs::write(d.join("r/.git/MERGE_HEAD"), ids[0]).unwrap();
    let e = repo.mergehead_foreach(|_| true).unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "no EOL at line 1");
    let _ = std::fs::remove_dir_all(&d);
}

/// A worktree `name` of the repository at `main` (laid out as `git
/// worktree add` does), its HEAD `head`.
fn add_worktree(main: &Path, wt: &Path, name: &str, head: &str) {
    let admin = main.join(".git/worktrees").join(name);
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(wt).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", wt.join(".git").display()),
    )
    .unwrap();
    std::fs::write(admin.join("HEAD"), head).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
}

#[test]
fn head_for_worktree() {
    let d = tmpdir("worktree");
    let main = d.join("main");
    let repo = Repository::init(&main, false).unwrap();
    let tip = "49322bb17d3acc9146f98c97d078513228bbf3c0";
    std::fs::write(main.join(".git/refs/heads/wtbranch"), format!("{tip}\n")).unwrap();
    add_worktree(&main, &d.join("wt"), "wt", "ref: refs/heads/wtbranch\n");
    let det = "0966a434eb1a025db6b71485ab63a3bfbea520b6";
    add_worktree(&main, &d.join("det"), "det", &format!("{det}\n"));

    // HEAD of a worktree on a branch: resolved
    let (name, target) = repo
        .head_for_worktree("wt", |r| (r.name().to_vec(), r.target()))
        .unwrap();
    assert_eq!(name, b"refs/heads/wtbranch");
    assert_eq!(target, Some(oid(tip)));
    // as C: the HEAD it looks at is resolved, so never symbolic
    assert!(repo.head_detached_for_worktree("wt").unwrap());

    // a detached HEAD
    let (name, target) = repo
        .head_for_worktree(b"det", |r| (r.name().to_vec(), r.target()))
        .unwrap();
    assert_eq!(name, b"HEAD");
    assert_eq!(target, Some(oid(det)));
    assert!(repo.head_detached_for_worktree("det").unwrap());

    // the main repository's HEAD is not changed
    assert!(repo.head_unborn().unwrap());
    let wt = Repository::open(d.join("wt")).unwrap();
    assert!(wt.is_worktree());

    let e = repo.head_for_worktree("nope", |_| ()).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let e = repo.head_detached_for_worktree("a\0b").unwrap_err();
    assert_eq!(e.message(), "worktree name contains a NUL byte");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn shared_repository_and_index_threads() {
    // I-1: the repository and its index together, from several threads.
    let d = tmpdir("threads");
    let repo = Repository::init(d.join("r"), false).unwrap();
    std::fs::write(d.join("r/f.txt"), "hello\n").unwrap();
    let index = repo.index().unwrap();
    let file = d.join("r/f.txt");
    std::thread::scope(|s| {
        for t in 0..4 {
            let (repo, index, file) = (&repo, &index, &file);
            s.spawn(move || {
                for i in 0..25 {
                    let id = repo
                        .odb()
                        .unwrap()
                        .write(format!("{t}-{i}").as_bytes(), ObjectType::Blob)
                        .unwrap();
                    index
                        .add(&IndexEntry::new(format!("t{t}/f{i}"), 0o100644, id))
                        .unwrap();
                    assert!(repo.head_unborn().unwrap());
                    repo.hashfile(file, ObjectType::Blob, None).unwrap();
                    let _ = index.get(i);
                }
            });
        }
    });
    assert_eq!(index.len(), 100);
    let tree = index.write_tree().unwrap();
    assert!(repo.odb().unwrap().exists(&tree));
    let _ = std::fs::remove_dir_all(&d);
}
