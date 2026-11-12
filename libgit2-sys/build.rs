//! Build script of the `-sys` crate, as crustify's `specs/bindgen.md` describes it; every
//! `-sys` crate of the workspace would hold a copy differing only in the constants below.
//!
//! - Shipped, by default: the committed `src/bindings.rs`, against the library found through
//!   `LIBGIT2_DIR` (an install prefix) or `pkg-config`, which must be the version the bindings were
//!   generated from.
//! - Regenerate, under the `bindgen` feature: bindings generated into `OUT_DIR` from the headers of
//!   the library found.
//! - Campaign, when `CRUSTIFY_BUILD_VARIANT` names a variant of `crustify/artifacts/build.json`, or
//!   `CRUSTIFY_BUILD_DIR` a private build: the bindings are regenerated from `c/`, with the include
//!   dirs and definitions libgit2's own TUs compile with, so that `wrapper.h` may name its internal
//!   headers, and checked against the committed ones, which `CRUSTIFY_BINDINGS_UPDATE=1` overwrites
//!   instead. libgit2 builds with `-fvisibility=hidden`, so its shared library exports only the
//!   `GIT_EXTERN` API: a variant that records a `static` archive, the objects `libgit2.so` links
//!   (`crustify/scripts/static-archive.sh`), has it linked statically, unbundled, so that the Rust
//!   reaches libgit2's internal functions, with the libraries `libgit2.so` links, as its
//!   `build.ninja` link edge records them; only the side campaigns' installs among those take an
//!   rpath. A variant that records none has its `libgit2.so` linked, with an rpath to it. A private
//!   build follows the record of the variant `CRUSTIFY_BUILD_VARIANT` names.
//!
//! Dependents read `DEP_GIT2_LINKAGE` (`static` or `shared`), `DEP_GIT2_BUILD_DIR`,
//! `DEP_GIT2_DEP_LIB_DIRS` (the installs' lib dirs, for their rpath) and `DEP_GIT2_VARIANT`.

/// The link unit: its library and its `pkg-config` module.
const LIB: &str = "libgit2";
/// The version of the library the committed bindings were generated from.
const C_VERSION: &str = "1.9.0";
/// The preprocessor definitions the bindings and the shims are built with against an install;
/// a campaign build takes those of the library's own TUs.
const DEFINES: &[&str] = &["GIT_DEPRECATE_HARD"];

use std::path::{Path, PathBuf};
use std::{env, fs};

/// The headers and version of the library a build compiles and links against.
struct Library {
    include: Vec<PathBuf>,
    /// The preprocessor definitions the bindings and the C wrappers are built with.
    defines: Vec<String>,
    version: String,
    /// The compiler and instrumentation flags of a campaign variant, for the C wrappers.
    cc: Option<(String, Vec<String>)>,
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("bindgen");
    for f in [
        "wrapper.h",
        "allowlist.txt",
        "external.txt",
        "shims.h",
        "shims.c",
    ] {
        println!("cargo:rerun-if-changed={}", input.join(f).display());
    }
    for f in ["src/bindings.rs", "src/static_fns.c"] {
        println!("cargo:rerun-if-changed={}", manifest.join(f).display());
    }
    for v in [
        "CRUSTIFY_BUILD_VARIANT",
        "CRUSTIFY_BUILD_DIR",
        "CRUSTIFY_BINDINGS_UPDATE",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    println!("cargo::rustc-check-cfg=cfg(crustify_regenerated)");
    // Dependents' build scripts read it as `DEP_<LINKS>_BINDGEN_DIR`.
    println!("cargo:bindgen_dir={}", input.display());

    let set = |v: &str| env::var_os(v).is_some_and(|s| !s.is_empty());
    let campaign = set("CRUSTIFY_BUILD_VARIANT") || set("CRUSTIFY_BUILD_DIR");
    let lib = if campaign {
        campaign_library(&manifest)
    } else {
        installed_library()
    };
    let regenerate = cfg!(feature = "bindgen");
    if !regenerate && lib.version != C_VERSION {
        panic!(
            "{LIB} {} found, but the bindings were generated from {C_VERSION}: build {LIB} \
             {C_VERSION}, or enable the `bindgen` feature to regenerate them",
            lib.version
        );
    }

    let mut static_fns = manifest.join("src/static_fns.c");
    if regenerate {
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        let (bindings, fns) = generate(&input, &lib, &out);
        if campaign {
            check_or_update(&manifest.join("src/bindings.rs"), &bindings, &out);
            check_or_update(&static_fns, fns.as_deref().unwrap_or(""), &out);
            println!("cargo:rustc-env=C_VERSION={C_VERSION}");
        } else {
            fs::write(out.join("bindings.rs"), bindings).unwrap();
            static_fns = out.join("static_fns.c");
            fs::write(&static_fns, fns.unwrap_or_default()).unwrap();
            println!("cargo:rustc-cfg=crustify_regenerated");
            println!("cargo:rustc-env=C_VERSION={}", lib.version);
        }
    } else {
        println!("cargo:rustc-env=C_VERSION={C_VERSION}");
    }
    compile_wrappers(&input, &lib, &static_fns);
}

/// The library of a campaign build: the variant's, or the private build's.
///
/// The variant's `static` archive, when `build.json` records one, is linked statically, and
/// its shared library otherwise; a private build follows the record of the variant
/// `CRUSTIFY_BUILD_VARIANT` names, and must carry the same archive. Either way, the libraries
/// the shared library links, and the include dirs and definitions the library's own TUs
/// compile with, are read from the build's `build.ninja`.
fn campaign_library(manifest: &Path) -> Library {
    let root = manifest.parent().unwrap().to_path_buf();
    let variant = env::var("CRUSTIFY_BUILD_VARIANT").unwrap_or_default();
    let record = (!variant.is_empty()).then(|| {
        let path = root.join("crustify/artifacts/build.json");
        println!("cargo:rerun-if-changed={}", path.display());
        let spec: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("build.json"))
                .expect("valid build.json");
        let record = spec["variants"][&variant].clone();
        assert!(
            record.is_object(),
            "no variant {variant:?} in {}",
            path.display()
        );
        record
    });
    let build = match env::var_os("CRUSTIFY_BUILD_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => root.join(
            record
                .as_ref()
                .and_then(|r| r["dir"].as_str())
                .expect("a variant's `dir`"),
        ),
    };
    let ninja = Ninja::read(&build);
    let link = ninja.link_edge(&format!("{LIB}.so"));
    let so = build.join(&link.output);
    assert!(so.exists(), "{} is missing: build it first", so.display());
    println!("cargo:rerun-if-changed={}", so.display());

    let archive = record
        .as_ref()
        .and_then(|r| r["static"].as_str())
        .map(|rel| build.join(rel));
    if let Some(archive) = &archive {
        link_static(archive, &so);
    } else {
        println!("cargo:rustc-link-search=native={}", build.display());
        println!("cargo:rustc-link-lib=dylib={}", &LIB[3..]);
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", build.display());
    }
    // The libraries the shared library links, linked as it links them: the out-of-tree ones
    // from their side campaigns' installs of the same variant, by the absolute paths the
    // build recorded, so that every checkout, a worktree's included, links the same ones;
    // the system's by name. A statically linked library needs them itself, and a shared one
    // loads them through its own rpath; the installs' dirs go on the rpath either way, and to
    // dependents (`DEP_GIT2_DEP_LIB_DIRS`) for theirs, so that a binary loads the very
    // libraries the build was linked with.
    let libs = LinkLibraries::parse(&link.var("LINK_LIBRARIES"));
    for dir in &libs.search {
        println!("cargo:rustc-link-search=native={dir}");
    }
    if archive.is_some() {
        for (kind, name) in &libs.libs {
            println!("cargo:rustc-link-lib={kind}={name}");
        }
    }
    for dir in &libs.rpath {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
    println!("cargo:dep_lib_dirs={}", libs.rpath.join(":"));
    println!(
        "cargo:linkage={}",
        if archive.is_some() {
            "static"
        } else {
            "shared"
        }
    );
    println!("cargo:variant={variant}");
    println!("cargo:build_dir={}", build.display());

    // The bindings see the internal headers as the library's own TUs do, from this checkout's
    // `c/` in place of the source dir the build was configured from.
    let source = cache_var(&build.join("CMakeCache.txt"), "CMAKE_HOME_DIRECTORY:");
    let compile = ninja.compile_edge(link, &format!("CMakeFiles/{LIB}.dir/"));
    let local = |p: &str| match Path::new(p).strip_prefix(&source) {
        Ok(rest) => root.join("c").join(rest),
        Err(_) => PathBuf::from(p),
    };
    let mut include = vec![root.join("c/include")];
    for dir in include_dirs(&compile.var("INCLUDES")) {
        let dir = local(&dir);
        if !include.contains(&dir) {
            include.push(dir);
        }
    }
    let defines = format!("{} {}", compile.var("DEFINES"), compile.var("FLAGS"))
        .split_whitespace()
        .filter_map(|f| f.strip_prefix("-D"))
        .map(String::from)
        .collect();
    Library {
        include,
        defines,
        version: pc_version(&build.join(format!("{LIB}.pc"))),
        cc: Some(variant_cc(&build.join("CMakeCache.txt"))),
    }
}

/// Links a variant's static archive, which `crustify/scripts/static-archive.sh` made from the
/// objects its shared library links, as the last step of its build.
///
/// The archive is linked unbundled (`-bundle`): the rlib does not copy its objects, and each
/// final binary links it from the variant's build, after the rlibs. Rust calls into the C
/// resolve from it as the linker meets them; the C's calls into the Rust exports of a ported
/// module resolve too, although their rlib comes first, since rustc's `symbols.o`, which
/// leads every link, references each exported symbol of the upstream crates, so that their
/// objects are already linked when the archive is read. No group or repetition is needed,
/// with rust-lld, the default, or GNU ld.
fn link_static(archive: &Path, so: &Path) {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("lib"))
        .and_then(|n| n.strip_suffix(".a"))
        .unwrap_or_else(|| panic!("{}: not named lib<name>.a", archive.display()));
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let archived = mtime(archive).unwrap_or_else(|| {
        panic!(
            "{} is missing: run crustify/scripts/static-archive.sh {LIB} <archive> in {}, as \
             the last step of the variant's build command",
            archive.display(),
            so.parent().unwrap().display()
        )
    });
    assert!(
        mtime(so).is_none_or(|linked| linked <= archived),
        "{} is older than {}, which was relinked since: rerun the variant's build command, \
         whose last step archives its objects again",
        archive.display(),
        so.display()
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static:-bundle={name}");
}

/// The edges of a CMake build's `build.ninja` that the campaign link reads.
struct Ninja {
    edges: Vec<Edge>,
}

/// One `build` edge: its outputs, rule, explicit inputs and variables.
struct Edge {
    output: String,
    outputs: Vec<String>,
    rule: String,
    inputs: Vec<String>,
    vars: Vec<(String, String)>,
}

impl Edge {
    fn var(&self, name: &str) -> String {
        self.vars
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

impl Ninja {
    fn read(build: &Path) -> Self {
        let path = build.join("build.ninja");
        println!("cargo:rerun-if-changed={}", path.display());
        let text = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}: a campaign build is a CMake build with the Ninja generator",
                path.display()
            )
        });
        let text = text.replace("$\n", "");
        let mut edges: Vec<Edge> = Vec::new();
        let mut current: Option<Edge> = None;
        for line in text.lines() {
            if let Some(var) = line.strip_prefix("  ") {
                if let (Some(edge), Some((n, v))) = (current.as_mut(), var.split_once(" = ")) {
                    edge.vars.push((n.trim().to_string(), v.to_string()));
                }
                continue;
            }
            edges.extend(current.take());
            let Some(decl) = line.strip_prefix("build ") else {
                continue;
            };
            let words = ninja_words(decl);
            let Some(colon) = words.iter().position(|w| w == ":") else {
                continue;
            };
            let outputs: Vec<String> = words[..colon]
                .iter()
                .filter(|w| *w != "|")
                .cloned()
                .collect();
            let (Some(rule), false) = (words.get(colon + 1).cloned(), outputs.is_empty()) else {
                continue;
            };
            let inputs = words[colon + 2..]
                .iter()
                .take_while(|w| *w != "|" && *w != "||")
                .cloned()
                .collect();
            current = Some(Edge {
                output: outputs[0].clone(),
                outputs,
                rule,
                inputs,
                vars: Vec::new(),
            });
        }
        edges.extend(current);
        Self { edges }
    }

    /// The edge that links `name`, followed through CMake's symlink edges.
    fn link_edge(&self, name: &str) -> &Edge {
        let mut name = name.to_string();
        for _ in 0..4 {
            let edge = self
                .edges
                .iter()
                .find(|e| e.outputs.contains(&name))
                .unwrap_or_else(|| panic!("build.ninja has no edge building {name}"));
            if edge.rule.contains("SHARED_LIBRARY_LINKER") {
                return edge;
            }
            name = edge
                .inputs
                .first()
                .unwrap_or_else(|| panic!("{name} is built from nothing"))
                .clone();
        }
        panic!("build.ninja links no shared library {name}")
    }

    /// The compile edge of the first object `link` takes whose path holds `dir`.
    fn compile_edge(&self, link: &Edge, dir: &str) -> &Edge {
        link.inputs
            .iter()
            .filter(|i| i.contains(dir))
            .find_map(|o| self.edges.iter().find(|e| &e.output == o))
            .unwrap_or_else(|| panic!("{} links no object under {dir}", link.output))
    }
}

/// The words of a `build` line, with ninja's `$ `, `$:` and `$$` escapes resolved, and the
/// colon that ends the outputs left as a word of its own.
fn ninja_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = line.chars();
    let flush = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '$' => word.extend(chars.next()),
            ' ' => flush(&mut word, &mut words),
            ':' => {
                flush(&mut word, &mut words);
                words.push(":".to_string());
            }
            c => word.push(c),
        }
    }
    flush(&mut word, &mut words);
    words
}

/// The libraries of a link edge's `LINK_LIBRARIES`, as cargo directives take them.
struct LinkLibraries {
    search: Vec<String>,
    libs: Vec<(&'static str, String)>,
    rpath: Vec<String>,
}

impl LinkLibraries {
    fn parse(value: &str) -> Self {
        let mut this = Self {
            search: Vec::new(),
            libs: Vec::new(),
            rpath: Vec::new(),
        };
        let push = |v: &mut Vec<String>, s: &str| {
            if !s.is_empty() && !v.iter().any(|x| x == s) {
                v.push(s.to_string());
            }
        };
        for word in value.split_whitespace() {
            if let Some(dirs) = word.strip_prefix("-Wl,-rpath,") {
                for dir in dirs.split(':') {
                    push(&mut this.rpath, dir);
                }
            } else if let Some(name) = word.strip_prefix("-l") {
                this.libs.push(("dylib", name.to_string()));
            } else if word == "-pthread" {
                this.libs.push(("dylib", "pthread".to_string()));
            } else if word.starts_with('/') {
                let path = Path::new(word);
                let file = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or_default();
                let stem = file.strip_prefix("lib").unwrap_or(file);
                let (kind, name) = match stem.split_once(".so") {
                    Some((name, _)) => ("dylib", name),
                    None => ("static", stem.strip_suffix(".a").unwrap_or(stem)),
                };
                push(&mut this.search, path.parent().unwrap().to_str().unwrap());
                this.libs.push((kind, name.to_string()));
            } else {
                panic!("LINK_LIBRARIES: cannot link {word:?} from a build script");
            }
        }
        this
    }
}

/// The include dirs of an edge's `INCLUDES`, `-I` and `-isystem` alike, in order.
fn include_dirs(value: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut words = value.split_whitespace();
    while let Some(w) = words.next() {
        if let Some(d) = w.strip_prefix("-I").filter(|d| !d.is_empty()) {
            dirs.push(d.to_string());
        } else if w == "-I" || w == "-isystem" {
            dirs.extend(words.next().map(String::from));
        } else if let Some(d) = w.strip_prefix("-isystem") {
            dirs.push(d.to_string());
        }
    }
    dirs
}

/// A value of a CMake cache, by its `NAME:` prefix.
fn cache_var(cache: &Path, name: &str) -> String {
    let text = fs::read_to_string(cache).unwrap_or_else(|e| panic!("{}: {e}", cache.display()));
    text.lines()
        .find_map(|l| l.strip_prefix(name).and_then(|r| r.split_once('=')))
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| panic!("{} records no {name}", cache.display()))
}

/// The library as a dependent finds it: under `<LIB>_DIR`, or through `pkg-config`.
fn installed_library() -> Library {
    let dir_var = format!("{}_DIR", LIB.to_uppercase());
    println!("cargo:rerun-if-env-changed={dir_var}");
    if let Some(dir) = env::var_os(&dir_var).map(PathBuf::from) {
        let lib_dir = dir.join("lib");
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib={}", &LIB[3..]);
        return Library {
            include: vec![dir.join("include")],
            defines: DEFINES.iter().map(|d| (*d).to_string()).collect(),
            version: pc_version(&lib_dir.join(format!("pkgconfig/{LIB}.pc"))),
            cc: None,
        };
    }
    let found = pkg_config::Config::new()
        .probe(LIB)
        .unwrap_or_else(|e| panic!("{LIB} not found: set {dir_var} to its install prefix ({e})"));
    Library {
        include: found.include_paths,
        defines: DEFINES.iter().map(|d| (*d).to_string()).collect(),
        version: found.version,
        cc: None,
    }
}

/// The `Version` a `pkg-config` file records.
fn pc_version(pc: &Path) -> String {
    println!("cargo:rerun-if-changed={}", pc.display());
    fs::read_to_string(pc)
        .unwrap_or_else(|e| panic!("{}: {e}", pc.display()))
        .lines()
        .find_map(|l| l.strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| panic!("{} records no Version", pc.display()))
}

/// The compiler of a campaign variant, and the instrumentation flags of its C flags, as
/// its CMake cache records them.
fn variant_cc(cache: &Path) -> (String, Vec<String>) {
    let text = fs::read_to_string(cache).unwrap_or_else(|e| panic!("{}: {e}", cache.display()));
    let var = |name: &str| {
        text.lines()
            .find_map(|l| {
                l.strip_prefix(name)
                    .and_then(|r| r.split_once('='))
                    .map(|(_, v)| v)
            })
            .unwrap_or_default()
            .to_string()
    };
    let instrumentation = [
        "-fsanitize",
        "-fno-sanitize",
        "-fpass-plugin",
        "-fprofile-arcs",
        "-ftest-coverage",
        "--coverage",
    ];
    let flags = var("CMAKE_C_FLAGS:")
        .split_whitespace()
        .filter(|f| instrumentation.iter().any(|p| f.starts_with(p)))
        .map(String::from)
        .collect();
    (var("CMAKE_C_COMPILER:"), flags)
}

/// Compiles the macro shims and the wrappers of the `static inline` functions.
fn compile_wrappers(input: &Path, lib: &Library, static_fns: &Path) {
    let mut cc = cc::Build::new();
    if let Some((compiler, flags)) = &lib.cc {
        cc.compiler(compiler);
        for f in flags {
            cc.flag(f);
        }
    }
    for d in &lib.defines {
        match d.split_once('=') {
            Some((name, value)) => cc.define(name, value),
            None => cc.define(d, None),
        };
    }
    cc.file(input.join("shims.c"))
        .includes(&lib.include)
        .include(input)
        .warnings(true);
    if fs::metadata(static_fns).is_ok_and(|m| m.len() > 0) {
        cc.file(static_fns);
    }
    cc.compile(&format!("crustify_{LIB}_wrappers"));
}

/// Fails the build when `path` does not hold `content`, unless asked to write it; the
/// content generated is left in `out` for comparison.
fn check_or_update(path: &Path, content: &str, out: &Path) {
    if fs::read_to_string(path).unwrap_or_default() == content {
        return;
    }
    if env::var_os("CRUSTIFY_BINDINGS_UPDATE").is_some() {
        if content.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, content).unwrap();
        }
        return;
    }
    let generated = out.join(path.file_name().unwrap());
    fs::write(&generated, content).unwrap();
    panic!(
        "{} does not match its bindgen input, which generates {}: rebuild with \
         CRUSTIFY_BINDINGS_UPDATE=1 and commit it",
        path.display(),
        generated.display()
    );
}

#[cfg(not(feature = "bindgen"))]
fn generate(_: &Path, _: &Library, _: &Path) -> (String, Option<String>) {
    unreachable!("regeneration needs the `bindgen` feature")
}

/// Generates the bindings, and the C wrappers of the allowlisted `static inline` functions
/// when there are any, from the crate's `bindgen/` input.
#[cfg(feature = "bindgen")]
fn generate(input: &Path, lib: &Library, out: &Path) -> (String, Option<String>) {
    let lines = |name: &str| -> Vec<String> {
        fs::read_to_string(input.join(name))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split('#').next().unwrap().trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    let mut b = bindgen::Builder::default()
        .header(input.join("wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", input.display()))
        .clang_args(lib.include.iter().map(|i| format!("-I{}", i.display())))
        .clang_args(lib.defines.iter().map(|d| format!("-D{d}")))
        .allowlist_item("crustify_.*")
        // A formatter of its own, not the toolchain's rustfmt, so that every toolchain
        // generates the same bytes.
        .formatter(bindgen::Formatter::Prettyplease)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .derive_default(true)
        .layout_tests(true)
        .generate_comments(true)
        .wrap_static_fns(true)
        .wrap_static_fns_path(out.join("static_fns"))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    for line in lines("allowlist.txt") {
        let (kind, pat) = line
            .split_once(char::is_whitespace)
            .expect("`<kind> <regex>`");
        let pat = pat.trim();
        b = match kind {
            "function" => b.allowlist_function(pat),
            "type" => b.allowlist_type(pat),
            "var" => b.allowlist_var(pat),
            k => panic!("allowlist.txt: unknown kind {k:?}"),
        };
    }
    for pat in lines("external.txt") {
        b = b.blocklist_type(pat);
    }
    let path = out.join("static_fns.c");
    // bindgen writes the wrappers only when the allowlist reaches a static inline function.
    let _ = fs::remove_file(&path);
    let bindings = b.generate().expect("bindgen").to_string();
    // The wrappers include the header by its absolute path; the build finds it in `bindgen/`.
    let fns = fs::read_to_string(&path).ok().map(|c| {
        c.replace(
            &format!("\"{}\"", input.join("wrapper.h").display()),
            "\"wrapper.h\"",
        )
    });
    (bindings, fns)
}
