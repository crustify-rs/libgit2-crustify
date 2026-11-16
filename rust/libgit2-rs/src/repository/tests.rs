//! Unit tests of the repository's pure logic (run under Miri), and the
//! C-compiled check of the bitfield's bit positions.

use super::open::{
    check_valid_extension, extensions, find_ceiling_dir_offset, free_extensions,
    refdb_type_fromstr, refdb_type_name, safe_directory_matches, set_extensions,
    worktree_repo_path,
};
use super::*;
use crate::util::errors;
use crate::util::test_support;

fn dirs<'a>(g: Option<&'a [u8]>, w: Option<&'a [u8]>, c: Option<&'a [u8]>) -> ItemDirs<'a> {
    ItemDirs {
        gitdir: g,
        workdir: w,
        commondir: c,
    }
}

fn path_of(d: &ItemDirs<'_>, item: ItemT) -> core::result::Result<Vec<u8>, ErrorCode> {
    let mut s = GitStr::new();
    item_path(&mut s, d, item)?;
    Ok(s.as_bytes().to_vec())
}

#[test]
fn item_paths_of_a_worktree() {
    let _g = test_support::init();
    let d = dirs(
        Some(b"/r/.git/worktrees/w/"),
        Some(b"/w/"),
        Some(b"/r/.git/"),
    );
    assert_eq!(path_of(&d, ITEM_GITDIR).unwrap(), b"/r/.git/worktrees/w/");
    assert_eq!(path_of(&d, ITEM_WORKDIR).unwrap(), b"/w/");
    assert_eq!(path_of(&d, ITEM_COMMONDIR).unwrap(), b"/r/.git/");
    assert_eq!(
        path_of(&d, ITEM_INDEX).unwrap(),
        b"/r/.git/worktrees/w/index"
    );
    assert_eq!(path_of(&d, ITEM_OBJECTS).unwrap(), b"/r/.git/objects/");
    assert_eq!(path_of(&d, 6).unwrap(), b"/r/.git/packed-refs");
    assert_eq!(path_of(&d, ITEM_CONFIG).unwrap(), b"/r/.git/config");
    assert_eq!(path_of(&d, 12).unwrap(), b"/r/.git/worktrees/w/modules/");
    assert_eq!(
        path_of(&d, ITEM_WORKTREE_CONFIG).unwrap(),
        b"/r/.git/worktrees/w/config.worktree"
    );
}

#[test]
fn item_paths_fall_back_and_fail_as_c() {
    let _g = test_support::init();
    // No commondir: the items under it fall back to the gitdir.
    let d = dirs(Some(b"/r/"), None, None);
    assert_eq!(path_of(&d, ITEM_OBJECTS).unwrap(), b"/r/objects/");
    assert_eq!(path_of(&d, 11).unwrap(), b"/r/logs/");
    // No workdir: not found, with C's message.
    assert_eq!(path_of(&d, ITEM_WORKDIR), Err(ErrorCode::NOT_FOUND));
    assert_eq!(
        errors::last_message().unwrap(),
        b"path cannot exist in repository"
    );
    assert_eq!(errors::last_class(), Some(errors::ErrorClass::INVALID));
    // An in-memory repository has no gitdir either.
    let d = dirs(None, None, None);
    assert_eq!(path_of(&d, ITEM_INFO), Err(ErrorCode::NOT_FOUND));
    // Past the table (C reads out of bounds there).
    assert_eq!(path_of(&d, ITEM_LAST), Err(ErrorCode::GENERIC));
    assert_eq!(errors::last_message().unwrap(), b"invalid item directory");
}

fn safe(value: Option<&[u8]>, repo: &[u8]) -> bool {
    let mut is_safe = false;
    safe_directory_matches(value, repo, &mut is_safe).unwrap();
    is_safe
}

#[test]
fn safe_directory_values() {
    let _g = test_support::init();
    let repo = b"/home/u/repo/";
    assert!(safe(Some(b"*"), repo));
    assert!(!safe(Some(b""), repo));
    assert!(!safe(None, repo));
    // Exact match: the value gets a trailing slash; one with a trailing
    // slash is ignored.
    assert!(safe(Some(b"/home/u/repo"), repo));
    assert!(!safe(Some(b"/home/u/repo/"), repo));
    assert!(!safe(Some(b"/home/u/rep"), repo));
    // Prefix match with `/*`.
    assert!(safe(Some(b"/home/u/*"), repo));
    assert!(!safe(Some(b"/home/v/*"), repo));
    assert!(safe(Some(b"/*"), repo));
    // `%(prefix)//` means an absolute path.
    assert!(safe(Some(b"%(prefix)//home/u/repo"), repo));
    assert!(!safe(Some(b"%(prefix)///home/u/repo"), repo));
    assert!(!safe(Some(b"%(prefix)/home/u/repo"), repo));
    // The root itself.
    assert!(safe(Some(b"/"), b"/"));
    // An empty value resets an earlier match.
    let mut s = false;
    safe_directory_matches(Some(b"*"), repo, &mut s).unwrap();
    safe_directory_matches(Some(b""), repo, &mut s).unwrap();
    assert!(!s);
}

#[test]
fn extensions_are_checked_as_c() {
    let _g = test_support::init();
    let _x = EXT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    free_extensions();
    for b in [
        "noop",
        "objectformat",
        "worktreeconfig",
        "preciousobjects",
        "refstorage",
        "relativeworktrees",
    ] {
        check_valid_extension(format!("extensions.{b}").as_bytes()).unwrap();
    }
    assert!(check_valid_extension(b"extensions.unknown").is_err());
    assert_eq!(
        errors::last_message().unwrap(),
        b"unsupported extension name extensions.unknown"
    );
    assert_eq!(errors::last_class(), Some(errors::ErrorClass::REPOSITORY));
    // The name must match exactly (config names are lowercased).
    assert!(check_valid_extension(b"extensions.NOOP").is_err());
    assert!(check_valid_extension(b"extension.noop").is_err());

    set_extensions(&[b"unknown", b"!noop", b"noop", b"unknown", b"aaa"]);
    check_valid_extension(b"extensions.unknown").unwrap();
    assert!(check_valid_extension(b"extensions.noop").is_err());
    let list = extensions().unwrap();
    let list: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
    assert_eq!(
        list,
        [
            &b"aaa"[..],
            b"objectformat",
            b"preciousobjects",
            b"refstorage",
            b"relativeworktrees",
            b"unknown",
            b"worktreeconfig"
        ]
    );
    free_extensions();
    assert_eq!(extensions().unwrap().len(), 6);
}

/// Serializes the tests that change the process-wide extensions.
pub(crate) static EXT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn small_helpers() {
    assert_eq!(worktree_repo_path(b"/w/.git"), Some(&b"/w/"[..]));
    assert_eq!(worktree_repo_path(b"/w/.GIT"), Some(&b"/w/"[..]));
    assert_eq!(worktree_repo_path(b".git"), None);
    assert_eq!(worktree_repo_path(b"/w/git"), None);
    assert_eq!(refdb_type_fromstr(b"files"), GIT_REFDB_FILES);
    assert_eq!(refdb_type_fromstr(b"reftable"), GIT_REFDB_REFTABLE);
    assert_eq!(refdb_type_fromstr(b"other"), 0);
    assert_eq!(refdb_type_name(GIT_REFDB_REFTABLE), c"reftable");
    assert_eq!(refdb_type_name(7), c"unknown");
}

#[test]
#[cfg_attr(miri, ignore = "realpath(3)")]
fn ceiling_dirs() {
    let tmp = std::env::temp_dir();
    let t = std::fs::canonicalize(&tmp).unwrap();
    let t = t.as_os_str().as_encoded_bytes().to_vec();
    let mut path = t.clone();
    path.extend_from_slice(b"/a/b");
    // No ceiling: the root's length.
    assert_eq!(find_ceiling_dir_offset(&path, None), 1);
    // A ceiling that is a prefix at a component boundary.
    assert_eq!(find_ceiling_dir_offset(&path, Some(&t)), t.len());
    let mut with_slash = t.clone();
    with_slash.push(b'/');
    assert_eq!(find_ceiling_dir_offset(&path, Some(&with_slash)), t.len());
    // Relative, empty and missing entries are skipped; the longest wins.
    let mut list = b"relative::/nonexistent/x:".to_vec();
    list.extend_from_slice(&t);
    list.extend_from_slice(b":/");
    assert_eq!(find_ceiling_dir_offset(&path, Some(&list)), t.len());
    // Not at a component boundary.
    let mut other = path.clone();
    other.extend_from_slice(b"c");
    let mut p2 = t.clone();
    p2.extend_from_slice(b"/a/bc/d");
    assert_eq!(find_ceiling_dir_offset(&p2, Some(&other)), 1);
    // A relative path has no root.
    assert_eq!(find_ceiling_dir_offset(b"a/b", Some(&t)), 0);
}

#[test]
fn reserved_names_statics() {
    let n: &[StaticStr] = &git_repository__reserved_names_win32;
    assert_eq!(n.len(), git_repository__reserved_names_win32_len);
    assert_eq!(n[1].size, 5);
    // SAFETY: a static C string.
    assert_eq!(unsafe { core::ffi::CStr::from_ptr(n[1].ptr) }, c"GIT~1");
    assert_eq!(git_repository__reserved_names_posix_len, 1);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C (git_cache_init)")]
fn alloc_and_free() {
    let _g = test_support::init();
    let p = GitRepository::alloc();
    assert!(!p.is_null());
    // SAFETY: the fresh repository.
    let r = unsafe { &*p };
    assert!(r.gitdir().is_none());
    assert!(!r.is_bare() && !r.use_env() && !r.is_worktree());
    r.set_is_bare(true);
    r.set_use_env(true);
    assert!(r.is_bare() && r.use_env() && !r.is_worktree());
    r.set_is_bare(false);
    assert!(!r.is_bare() && r.use_env());
    r.init_string(StrField::Gitdir, alloc_cstr(b"/x/").unwrap());
    assert_eq!(r.gitdir(), Some(c"/x/"));
    // SAFETY: no borrow of the namespace is live.
    unsafe { r.replace_string(StrField::Namespace, Some(alloc_cstr(b"ns").unwrap())) };
    assert_eq!(r.namespace(), Some(c"ns"));
    r.set_ident(Some(c"n"), None).unwrap();
    assert_eq!(r.ident_name(), Some(c"n"));
    assert_eq!(r.ident_email(), None);
    // SAFETY: ours alone.
    unsafe { GitRepository::free(p) };
}

/// The bitfield's bits, as the C compiler lays them out: compile and run a
/// program against the build's headers.
#[test]
#[cfg_attr(
    any(miri, not(feature = "link-c")),
    ignore = "runs clang on LIBGIT2_BUILD_DIR"
)]
fn bitfield_bits_match_c() {
    let dir =
        std::path::PathBuf::from(std::env::var_os("LIBGIT2_BUILD_DIR").expect("LIBGIT2_BUILD_DIR"));
    let tmp = std::env::temp_dir().join(format!("l-repo-bits-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let src = tmp.join("bits.c");
    std::fs::write(
        &src,
        "#include \"common.h\"\n#include \"repository.h\"\n#include <stdio.h>\n#include <string.h>\n\
         static unsigned word(git_repository *r) { unsigned w; memcpy(&w, (char *)r + 232, sizeof w); return w; }\n\
         int main(void) { static git_repository r; unsigned a, b, c;\n\
         memset(&r, 0, sizeof r); r.use_env = 1; a = word(&r);\n\
         memset(&r, 0, sizeof r); r.is_bare = 1; b = word(&r);\n\
         memset(&r, 0, sizeof r); r.is_worktree = 1; c = word(&r);\n\
         printf(\"%u %u %u\", a, b, c); return 0; }\n",
    )
    .unwrap();
    let exe = tmp.join("bits");
    let st = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "clang".into()))
        .current_dir(&dir)
        .args(crate::util::layout::build_flags(&dir))
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());
    let out = std::process::Command::new(&exe).output().unwrap();
    let expected = format!("{} {} {}", BIT_USE_ENV, BIT_IS_BARE, BIT_IS_WORKTREE);
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    let _ = std::fs::remove_dir_all(&tmp);
}
