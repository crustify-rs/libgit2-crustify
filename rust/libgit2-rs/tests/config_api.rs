//! The safe Rust API of the config modules, as a consumer uses it: root
//! items only, no `unsafe`, no init (the library initializes itself).
//!
//! The memory and Rust backends, lookups, parsing, snapshots and
//! concurrent readers run under Miri; file and pattern tests reach C
//! (`stat`, PCRE2) and are ignored there.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use libgit2_rs::{
    Config, ConfigBackend, ConfigEntry, ConfigLevel, ConfigMap, ConfigMapKind, Error, ErrorClass,
    ErrorCode,
};

const TEXT: &[u8] = b"[core]\n\tbare = false\n\tabbrev = 12\n[remote \"Origin\"]\n\turl = a\n\turl = b\n[x]\n\tflag\n\tsize = 2k\n";

fn memory() -> Config<'static> {
    let c = Config::new();
    c.add_memory(TEXT, ConfigLevel::Local, false).unwrap();
    c
}

#[test]
fn memory_lookups() {
    let c = memory();
    assert!(!c.get_bool("core.bare").unwrap());
    assert!(c.get_bool("X.FLAG").unwrap());
    assert_eq!(c.get_i32("core.abbrev").unwrap(), 12);
    assert_eq!(c.get_i64("x.size").unwrap(), 2048);
    assert_eq!(c.get_string("remote.Origin.url").unwrap(), "b");
    let e = c.get_entry("remote.Origin.url").unwrap();
    assert_eq!(e.name(), b"remote.Origin.url");
    assert_eq!(e.backend_type(), Some(&b"in-memory"[..]));
    assert_eq!(e.level(), ConfigLevel::Local);
    let err = c.get_string("nope.nope").unwrap_err();
    assert_eq!(err.code(), ErrorCode::NOT_FOUND);
    assert_eq!(err.message(), "config value 'nope.nope' was not found");
    let err = c.get_string("invalid").unwrap_err();
    assert_eq!(err.code(), ErrorCode::INVALID_SPEC);
    // Read-only: no writer.
    assert_eq!(
        c.set_str("core.bare", "true").unwrap_err().code(),
        ErrorCode::READ_ONLY
    );
}

#[test]
fn iteration_and_levels() {
    let c = memory();
    c.add_memory(b"[core]\nbare = true\n", ConfigLevel::System, false)
        .unwrap();
    // The higher level wins.
    assert!(!c.get_bool("core.bare").unwrap());
    let names: Vec<_> = c
        .entries(None)
        .unwrap()
        .map(|e| e.unwrap().name().to_vec())
        .collect();
    // From the lowest level up.
    assert_eq!(names[0], b"core.bare");
    assert_eq!(names.len(), 7);
    let dup = c.add_memory(b"", ConfigLevel::System, false).unwrap_err();
    assert_eq!(dup.code(), ErrorCode::EXISTS);
    assert_eq!(dup.message(), "configuration at level 2 already exists");
    c.add_memory(b"[core]\nbare = maybe\n", ConfigLevel::System, true)
        .unwrap();
    let sys = c.open_level(ConfigLevel::System).unwrap();
    assert!(sys.get_bool("core.bare").is_err());
    let snap = c.snapshot().unwrap();
    assert!(!snap.get_bool("core.bare").unwrap());
    let clone = snap.clone();
    drop(snap);
    assert_eq!(clone.get_i32("core.abbrev").unwrap(), 12);
}

#[test]
fn parse_functions() {
    assert!(Config::parse_bool("yes").unwrap());
    assert!(Config::parse_bool(b"On").unwrap());
    assert!(!Config::parse_bool("").unwrap());
    assert!(!Config::parse_bool("0").unwrap());
    let e = Config::parse_bool("maybe").unwrap_err();
    assert_eq!(e.message(), "failed to parse 'maybe' as a boolean");
    assert_eq!(Config::parse_i32("-3k").unwrap(), -3072);
    assert_eq!(Config::parse_i64("1g").unwrap(), 1 << 30);
    let e = Config::parse_i32("99999999999").unwrap_err();
    assert_eq!(
        e.message(),
        "failed to parse '99999999999' as a 32-bit integer"
    );
    assert!(Config::parse_i64("12x").is_err());
}

#[test]
#[cfg_attr(miri, ignore = "reaches C (the home directory)")]
fn parse_path() {
    assert_eq!(
        Config::parse_path("/a/b").unwrap(),
        std::path::Path::new("/a/b")
    );
    let home = Config::parse_path("~/x").unwrap();
    assert!(home.is_absolute() && home.ends_with("x"), "{home:?}");
    let e = Config::parse_path("~user/x").unwrap_err();
    assert_eq!(e.class(), ErrorClass::CONFIG);
    assert_eq!(e.message(), "retrieving a homedir by name is not supported");
    assert_eq!(
        Config::parse_path("a\0b").unwrap_err().class(),
        ErrorClass::INVALID
    );
}

#[test]
fn mapped_values() {
    let c = memory();
    let maps = [
        ConfigMap::truthy(1),
        ConfigMap::falsy(0),
        ConfigMap::string(b"input", 7),
    ];
    assert_eq!(c.get_mapped("x.flag", &maps).unwrap(), 1);
    assert_eq!(c.get_mapped("core.bare", &maps).unwrap(), 0);
    // A non-zero integer is a true boolean.
    assert_eq!(c.get_mapped("core.abbrev", &maps).unwrap(), 1);
    let e = c.get_mapped("remote.Origin.url", &maps).unwrap_err();
    assert_eq!(e.message(), "failed to map 'b'");
    let with_int = [ConfigMap::string(b"auto", -1), ConfigMap::int32()];
    assert_eq!(c.get_mapped("core.abbrev", &with_int).unwrap(), 12);
    assert_eq!(Config::lookup_map_value(&maps, Some(b"INPUT")).unwrap(), 7);
    // A bare name is true.
    assert_eq!(Config::lookup_map_value(&maps, None).unwrap(), 1);
    let e = Config::lookup_map_value(&maps, Some(b"x")).unwrap_err();
    assert_eq!(e.class(), ErrorClass::CONFIG);
    assert_eq!(e.message(), "failed to map 'x'");
    let custom = ConfigMap {
        kind: ConfigMapKind::String,
        str_match: Some(b"keep"),
        map_value: 3,
    };
    assert_eq!(
        Config::lookup_map_value(&[custom], Some(b"Keep")).unwrap(),
        3
    );
    assert_eq!(ConfigMap::int32().kind, ConfigMapKind::Int32);
}

#[test]
fn names_are_bytes() {
    let c = memory();
    let name = String::from("core.abbrev");
    assert_eq!(c.get_i32(&name).unwrap(), 12);
    assert_eq!(c.get_i32(name.as_bytes()).unwrap(), 12);
    assert_eq!(c.get_i32(Vec::from("core.abbrev")).unwrap(), 12);
    // A NUL is refused, not cut.
    let e = c.get_i32("core.abbrev\0x").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert!(
        c.multivar("remote.origin.url", Some(b"a\0".as_slice()))
            .is_err()
    );
    assert!(c.entries(Some(b"\0".as_slice())).is_err());
}

/// A Rust backend: a list of entries, writable.
struct Vars(Mutex<Vec<(Vec<u8>, Vec<u8>)>>);

impl ConfigBackend for Vars {
    fn get(&self, key: &[u8]) -> Result<Option<ConfigEntry>, Error> {
        let v = self.0.lock().unwrap();
        Ok(v.iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(k, val)| ConfigEntry::new(k.clone(), Some(val.clone()), ConfigLevel::Highest)))
    }
    fn entries(&self) -> Result<Vec<ConfigEntry>, Error> {
        let v = self.0.lock().unwrap();
        Ok(v.iter()
            .map(|(k, val)| {
                ConfigEntry::new(k.clone(), Some(val.clone()), ConfigLevel::App)
                    .with_backend_type("vars")
            })
            .collect())
    }
    fn set(&self, name: &[u8], value: &[u8]) -> Result<(), Error> {
        self.0
            .lock()
            .unwrap()
            .push((name.to_ascii_lowercase(), value.to_vec()));
        Ok(())
    }
}

#[test]
fn rust_backend() {
    let c = Config::new();
    c.add_backend(
        Vars(Mutex::new(vec![(b"a.b".to_vec(), b"1".to_vec())])),
        ConfigLevel::App,
        false,
    )
    .unwrap();
    assert_eq!(c.get_i32("a.b").unwrap(), 1);
    c.set_i32("A.c", 5).unwrap();
    assert_eq!(c.get_i32("a.c").unwrap(), 5);
    let e = c.get_entry("a.c").unwrap();
    assert_eq!(e.level(), ConfigLevel::App);
    let all: Vec<_> = c.entries(None).unwrap().map(Result::unwrap).collect();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].backend_type(), Some(&b"vars"[..]));
    // A snapshot copies the entries.
    let s = c.snapshot().unwrap();
    c.set_i32("a.d", 1).unwrap();
    assert!(s.get_i32("a.d").is_err());
    assert_eq!(s.get_i32("a.c").unwrap(), 5);
    // The backend's writes fail as it says.
    assert_eq!(
        c.remove("a.b").unwrap_err().message(),
        "this backend is read-only"
    );
}

#[test]
fn concurrent_readers() {
    let c = memory();
    std::thread::scope(|s| {
        for t in 0..3 {
            let c = &c;
            s.spawn(move || {
                for _ in 0..if cfg!(miri) { 3 } else { 200 } {
                    assert_eq!(c.get_i32("core.abbrev").unwrap(), 12);
                    let n = c.entries(None).unwrap().count();
                    assert!(n == 6 || n == 7, "{n}");
                    if t == 0 {
                        let s = c.snapshot().unwrap();
                        assert!(!s.get_bool("core.bare").unwrap());
                    }
                }
            });
        }
        // A writer of the backend list meanwhile.
        let c2 = c.clone();
        s.spawn(move || {
            c2.add_memory(b"[y]\nz = 1\n", ConfigLevel::Global, true)
                .unwrap();
        });
    });
}

#[test]
#[cfg_attr(miri, ignore = "reaches C (stat, PCRE2)")]
fn files_and_patterns() {
    let dir = std::env::temp_dir().join(format!("l-config-api-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(&path, "# keep\n[core]\n\tbare = false ; c\n").unwrap();
    let c = Config::open(&path).unwrap();
    c.set_bool("core.bare", true).unwrap();
    c.set_str("New.Sub.key", "a;b").unwrap();
    c.set_multivar("multi.v", "^$", "1").unwrap();
    c.set_multivar("multi.v", "^x$", "2").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# keep\n[core]\n\tbare = true\n[New \"Sub\"]\n\tkey = \"a;b\"\n[multi]\n\tv = 1\n\tv = 2\n"
    );
    let vals: Vec<_> = c
        .multivar("multi.v", None)
        .unwrap()
        .map(|e| e.unwrap().value().unwrap().to_vec())
        .collect();
    assert_eq!(vals, [b"1".to_vec(), b"2".to_vec()]);
    let glob: Vec<_> = c
        .entries(Some(b"^core\\.".as_slice()))
        .unwrap()
        .map(|e| e.unwrap().name().to_vec())
        .collect();
    assert_eq!(glob, [b"core.bare".to_vec()]);
    c.remove_multivar("multi.v", "1").unwrap();
    c.remove("core.bare").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# keep\n[core]\n[New \"Sub\"]\n\tkey = \"a;b\"\n[multi]\n\tv = 2\n"
    );
    // Another writer: reread on change.
    std::fs::write(&path, "[core]\n\tabbrev = 9\n").unwrap();
    assert_eq!(c.get_i32("core.abbrev").unwrap(), 9);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "reaches C (the lock file)")]
fn lock_commits_or_rolls_back() {
    let dir = std::env::temp_dir().join(format!("l-config-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(&path, "[core]\n\tabbrev = 7\n").unwrap();
    let c = Config::open(&path).unwrap();

    // Rolled back on drop.
    let tx = c.lock().unwrap();
    c.set_i32("core.abbrev", 8).unwrap();
    // Not yet on disk.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[core]\n\tabbrev = 7\n"
    );
    drop(tx);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[core]\n\tabbrev = 7\n"
    );
    assert_eq!(c.get_i32("core.abbrev").unwrap(), 7);

    // Committed.
    let tx = c.lock().unwrap();
    c.set_i32("core.abbrev", 9).unwrap();
    c.set_str("core.x", "y").unwrap();
    tx.commit().unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[core]\n\tabbrev = 9\n\tx = y\n"
    );
    assert_eq!(c.get_i32("core.abbrev").unwrap(), 9);

    // Read-only configurations cannot be locked.
    let e = memory().lock().unwrap_err();
    assert_eq!(e.code(), ErrorCode::READ_ONLY);
    assert_eq!(e.message(), "cannot lock: the configuration is read-only");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "reaches C (the search paths)")]
fn find_files() {
    for (r, name) in [
        (Config::find_global(), ".gitconfig"),
        (Config::find_xdg(), "config"),
        (Config::find_system(), "gitconfig"),
    ] {
        match r {
            Ok(p) => {
                assert!(p.exists(), "{p:?}");
                assert!(p.ends_with(name), "{p:?}");
            }
            Err(e) => assert_eq!(e.code(), ErrorCode::NOT_FOUND, "{e:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Misbehaving backends (review api-r1, S-3, S-6c, S-6e, S-6g)
// ---------------------------------------------------------------------------

/// A backend that implements nothing: every method has a default.
struct Nothing;
impl ConfigBackend for Nothing {}

#[test]
fn backend_defaults() {
    let c = Config::new();
    c.add_backend(Nothing, ConfigLevel::App, false).unwrap();
    assert_eq!(c.get_bool("a.b").unwrap_err().code(), ErrorCode::NOT_FOUND);
    assert_eq!(c.entries(None).unwrap().count(), 0);
    let e = c.set_bool("a.b", true).unwrap_err();
    assert_eq!(e.message(), "this backend is read-only");
    assert!(c.lock().is_err());
}

struct Bad(&'static str);
impl ConfigBackend for Bad {
    fn get(&self, key: &[u8]) -> Result<Option<ConfigEntry>, Error> {
        match self.0 {
            "panic" => panic!("get panics"),
            "nul" => Ok(Some(ConfigEntry::new(
                b"a.b\0c".to_vec(),
                Some(b"1".to_vec()),
                ConfigLevel::Highest,
            ))),
            "positive" => Err(Error::new(ErrorCode::EXISTS, ErrorClass::CONFIG, "p").clone()),
            "iterover" => Err(Error::new(ErrorCode::ITER_OVER, ErrorClass::CONFIG, "it")),
            _ => Ok(Some(ConfigEntry::new(
                key.to_vec(),
                None,
                ConfigLevel::Local,
            ))),
        }
    }
    fn entries(&self) -> Result<Vec<ConfigEntry>, Error> {
        match self.0 {
            "panic" => panic!("entries panics"),
            _ => Ok(vec![
                ConfigEntry::new(b"a.b".to_vec(), Some(b"1".to_vec()), ConfigLevel::Global),
                ConfigEntry::new(b"a.b\0c".to_vec(), Some(b"2".to_vec()), ConfigLevel::Global),
            ]),
        }
    }
    fn set(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        panic!("set panics")
    }
}

fn bad(mode: &'static str) -> Config<'static> {
    let c = Config::new();
    c.add_backend(Bad(mode), ConfigLevel::Local, false).unwrap();
    c
}

/// S-6c: a panicking backend gives an error of class INTERNAL.
#[test]
fn panicking_backend_is_an_internal_error() {
    let c = bad("panic");
    for e in [
        c.get_bool("a.b").unwrap_err(),
        c.entries(None)
            .and_then(|i| i.collect::<Result<Vec<_>, _>>())
            .unwrap_err(),
        c.set_bool("a.b", true).unwrap_err(),
    ] {
        assert_eq!(e.code(), ErrorCode::GENERIC);
        assert_eq!(e.class(), ErrorClass::INTERNAL);
    }
    // Still usable.
    assert!(c.snapshot().is_err());
}

/// S-6e: a NUL in a name from a backend is refused, not cut.
#[test]
fn nul_in_backend_names_is_refused() {
    let c = bad("nul");
    let e = c.get_entry("a.b").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(
        e.message(),
        "the config entry name 'a.b' from the backend contains a NUL byte"
    );
    let e = c
        .entries(None)
        .and_then(|i| i.collect::<Result<Vec<_>, _>>())
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
}

/// S-6g: success-like and ITER_OVER codes from a backend are failures.
#[test]
fn backend_codes_are_sanitized() {
    let e = bad("iterover").get_bool("a.b").unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "it");
    let e = bad("positive").get_bool("a.b").unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
    // A bare name is true.
    assert!(bad("bare").get_bool("a.b").unwrap());
}

/// A backend that reads and writes its own configuration (re-entry).
struct Reenter(Arc<OnceLock<Config<'static>>>, AtomicUsize);
impl ConfigBackend for Reenter {
    fn get(&self, key: &[u8]) -> Result<Option<ConfigEntry>, Error> {
        if self.1.fetch_add(1, Ordering::SeqCst) == 0
            && let Some(c) = self.0.get()
        {
            let _ = c.entries(None).map(Iterator::count);
            let _ = c.add_memory(b"[r]\ne = 1\n", ConfigLevel::App, true);
        }
        Ok(Some(ConfigEntry::new(
            key.to_vec(),
            Some(b"v".to_vec()),
            ConfigLevel::Local,
        )))
    }
}

#[test]
#[cfg_attr(
    miri,
    ignore = "the backend holds its own Config: a cycle that leaks by design (S-6h)"
)]
fn reentrant_backend() {
    let me = Arc::new(OnceLock::new());
    let c = Config::new();
    let _ = me.set(c.clone());
    c.add_backend(
        Reenter(me.clone(), AtomicUsize::new(0)),
        ConfigLevel::Local,
        false,
    )
    .unwrap();
    assert_eq!(c.get_string("a.b").unwrap(), "v");
    assert_eq!(c.get_i32("r.e").unwrap(), 1);
}

struct PanicDrop;
impl Drop for PanicDrop {
    fn drop(&mut self) {
        panic!("backend Drop panics");
    }
}
struct DropPanics(#[allow(dead_code)] PanicDrop);
impl ConfigBackend for DropPanics {}

/// S-3: a backend whose `Drop` panics: no abort.
#[test]
#[cfg_attr(miri, ignore = "the panic payload is leaked by design (S-3)")]
fn backend_drop_panic() {
    let c = Config::new();
    c.add_backend(DropPanics(PanicDrop), ConfigLevel::App, false)
        .unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(c)));
    assert!(r.is_ok());
    // Refused (an existing level), so dropped at once.
    let c = Config::new();
    c.add_backend(Nothing, ConfigLevel::App, false).unwrap();
    let e = c
        .add_backend(DropPanics(PanicDrop), ConfigLevel::App, false)
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::EXISTS);
}
