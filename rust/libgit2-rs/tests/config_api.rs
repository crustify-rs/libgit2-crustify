//! The safe Rust API of the config modules, with no `unsafe`.
//!
//! The memory and Rust backends, lookups, parsing, snapshots and
//! concurrent readers run under Miri; file and pattern tests reach C
//! (`stat`, PCRE2) and are ignored there.

#![forbid(unsafe_code)]

use std::sync::Mutex;

use libgit2_rs::config::{
    ConfigMap, GIT_CONFIGMAP_STRING, GIT_CONFIGMAP_TRUE, config_parse_bool, config_parse_i32,
    config_parse_i64,
};
use libgit2_rs::util::test_support;
use libgit2_rs::{Config, ConfigBackend, ConfigEntry, ConfigLevel, Error, ErrorCode};

const TEXT: &[u8] = b"[core]\n\tbare = false\n\tabbrev = 12\n[remote \"Origin\"]\n\turl = a\n\turl = b\n[x]\n\tflag\n\tsize = 2k\n";

fn memory() -> Config<'static> {
    let c = Config::new();
    c.add_memory(TEXT, ConfigLevel::Local, false).unwrap();
    c
}

#[test]
fn memory_lookups() {
    let _g = test_support::init();
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
    let _g = test_support::init();
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
    let _g = test_support::init();
    assert!(config_parse_bool("yes").unwrap());
    assert!(!config_parse_bool("").unwrap());
    assert_eq!(config_parse_i32("-3k").unwrap(), -3072);
    assert_eq!(config_parse_i64("1g").unwrap(), 1 << 30);
    let e = config_parse_i32("99999999999").unwrap_err();
    assert_eq!(
        e.message(),
        "failed to parse '99999999999' as a 32-bit integer"
    );
    let c = memory();
    let maps = [
        ConfigMap {
            kind: GIT_CONFIGMAP_TRUE,
            str_match: None,
            map_value: 1,
        },
        ConfigMap {
            kind: GIT_CONFIGMAP_STRING,
            str_match: Some(b"a"),
            map_value: 7,
        },
    ];
    assert_eq!(c.get_mapped("x.flag", &maps).unwrap(), 1);
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
    let _g = test_support::init();
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
    let _g = test_support::init();
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
    let _g = test_support::init();
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
        .entries(Some("^core\\."))
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
