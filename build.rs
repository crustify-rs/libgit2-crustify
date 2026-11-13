//! Builds or locates the C, compiles the C wrappers of the raw bindings, and regenerates the
//! bindings on request, in the four modes of crustify's `bindgen` skill (Build modes), for each
//! link unit of [`UNITS`]:
//!
//! - Bundled, by default: `c/` built statically into `OUT_DIR` by libgit2's own `CMake`, in the
//!   configuration of the campaign's `plain` variant ([`CMAKE_OPTIONS`]), linking the system's
//!   OpenSSL, PCRE2 and zlib, with no rpath; under the `git2` feature, the CLI's TUs are compiled
//!   and archived too, and its executable left unlinked ([`built_executable_objects`]).
//! - Shared, under the `shared` feature: `c/` built the same way as shared libraries, which the
//!   crate's binaries find beside them through an `$ORIGIN` rpath. No library holds an executable
//!   link unit's objects, so this mode and the next refuse the `git2` feature.
//! - System, under the `system` feature: an install, found through `<LIB>_DIR` (an install prefix),
//!   then `pkg-config`, which must be the version the bindings were generated from. An install
//!   lacks the internal headers and the functions its shared library does not export, so this mode
//!   fails once the bindings reach either, as it fails once a module is switched to Rust.
//! - Campaign, when `CRUSTIFY_BUILD_VARIANT` names a variant of `crustify/artifacts/build.json`, or
//!   `CRUSTIFY_BUILD_DIR` a private build: the variant's build. libgit2 builds with
//!   `-fvisibility=hidden`, so its shared library exports only the `GIT_EXTERN` API: a variant that
//!   records a `static` archive, the objects all its link units link (`crustify/scripts/
//!   static-archive.sh`), has it linked statically, once, unbundled, so that the Rust reaches
//!   libgit2's internal functions, with the libraries the link unit links, as its `build.ninja`
//!   link edge records them; only the side campaigns' installs among those take an rpath. A variant
//!   that records none has its shared library linked, with an rpath to it. A private build follows
//!   the record of the variant `CRUSTIFY_BUILD_VARIANT` names. The Rust library a variant's own C
//!   links, from the first module's switch on, is the crate built as a `staticlib` in campaign mode
//!   with `CRUSTIFY_STATICLIB` set (`crustify/scripts/rust-staticlib.sh`): it reads the variant's
//!   configured build for its headers, definitions and compiler, and links none of it, so that it
//!   carries no C of the target's; the variant's link edges list it among their libraries
//!   (`LIBGIT2_RUST_LIBRARY`), and every other campaign build leaves it out of the link, so that no
//!   binary holds the Rust twice.
//!
//! Every mode uses the committed `sys/<link-unit>/bindings.rs` unless the `bindgen` feature
//! regenerates the bindings into `OUT_DIR` from the headers of the C it links, with the include
//! dirs and definitions the link unit's own TUs compile with, so that `wrapper.h` may name its
//! internal headers. In campaign mode the regenerated bindings are compared with the committed
//! ones instead, a difference failing the build, unless `CRUSTIFY_BINDINGS_UPDATE` is set, which
//! writes them over the committed ones.
//!
//! Dependents read `DEP_GIT2_LINKAGE` (`static` or `shared`), `DEP_GIT2_BUILD_DIR`,
//! `DEP_GIT2_DEP_LIB_DIRS` (the side campaigns' lib dirs, for their rpath) and `DEP_GIT2_VARIANT`
//! in campaign mode, and `DEP_GIT2_SYS_DIR_<LINK_UNIT>` in every mode.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

/// A link unit: one library or executable of the C, bound by one public module of `src/sys.rs`.
struct LinkUnit {
    /// Its module in `src/sys.rs`, its directory under `sys/`, and its argument to
    /// `crustify/scripts/static-archive.sh`.
    name: &'static str,
    /// The Cargo feature that builds it, for a link unit the crate can leave out; its
    /// modules in `src/sys.rs` and `src/lib.rs` are gated on the same feature.
    feature: Option<&'static str>,
    /// The file a variant's build links for it, as its `build.ninja` names it: a shared
    /// library, or an executable.
    artifact: &'static str,
    /// The library an install, a shared build or the bundled one gives it, by its file stem,
    /// which is also its `pkg-config` module: none for an executable, whose objects only the
    /// bundled build and the variants' static archive hold.
    lib: Option<&'static str>,
    /// The `CMake` target whose objects its own TUs compile into, under
    /// `CMakeFiles/<target>.dir/`: their include dirs and definitions are those of the bindings.
    target: &'static str,
    /// The `pkg-config` file of the build that records its version.
    version_pc: &'static str,
    /// The version of the C the committed bindings were generated from.
    c_version: &'static str,
    /// The preprocessor definitions its bindings and wrappers are built with against an
    /// install; the other modes take those of its own TUs.
    defines: &'static [&'static str],
}

impl LinkUnit {
    /// Its library's file stem, which only a library link unit has.
    fn library(&self) -> &'static str {
        self.lib
            .unwrap_or_else(|| panic!("the `{}` link unit is an executable", self.name))
    }

    /// Its library's link name, `git2` for `libgit2`.
    fn link_name(&self) -> &'static str {
        let lib = self.library();
        lib.strip_prefix("lib").unwrap_or(lib)
    }

    /// Its directory of `sys/`.
    fn sys_dir(&self, root: &Path) -> PathBuf {
        root.join("sys").join(self.name)
    }

    /// Whether the build includes it: always, or when its feature is on.
    fn enabled(&self) -> bool {
        self.feature.is_none_or(|f| {
            env::var_os(format!(
                "CARGO_FEATURE_{}",
                f.to_uppercase().replace('-', "_")
            ))
            .is_some()
        })
    }
}

/// The link units of the C, one module of `src/sys.rs` each, in link order: a link unit's
/// bindings may re-export the types of one before it, as its `external.txt` lists them.
///
/// The `git2` link unit is the CLI executable, `src/cli`, whose objects the variants' static
/// archive holds beside libgit2's and the bundled build compiles under the `git2` feature.
/// Its `main.c.o` defines `main`, besides the globals `cli_cmds`, `cli_common_opts`,
/// `cli_opt__show_help` and `cli_opt__use_pager`, which `cmd.c` and every `cmd_*.c` reference.
/// An archive member is linked only when a symbol it defines is referenced, so a test binary
/// links the C's `main` as soon as it references a `main.c` symbol, directly or through a
/// `cmd_*.c` or `cmd.c` one: the link then fails with two definitions of `main`, the C's and
/// the test harness's (or a Rust binary's). The other TUs (`opt.c`, `opt_usage.c`,
/// `common.c`, `progress.c`, `unix/sighandler.c`) reference none, and link alone. The port of
/// `main.c` has to deal with it: its globals move to Rust exports first, so that the
/// `cmd_*.c` that remain no longer pull it, and `main` itself becomes the Rust `git2`
/// binary's, its C leaving with the switch; until then a test reaches `main.c` and the
/// `cmd_*.c` only through the C executable the variant built, as a child process.
const UNITS: &[LinkUnit] = &[
    LinkUnit {
        name: "libgit2",
        feature: None,
        artifact: "libgit2.so",
        lib: Some("libgit2"),
        target: "libgit2",
        version_pc: "libgit2",
        c_version: "1.9.0",
        defines: &["GIT_DEPRECATE_HARD"],
    },
    LinkUnit {
        name: "git2",
        feature: Some("git2"),
        artifact: "git2",
        lib: None,
        target: "git2_cli",
        version_pc: "libgit2",
        c_version: "1.9.0",
        defines: &["GIT_DEPRECATE_HARD"],
    },
];

/// The configure options of the bundled and shared builds: those of the `plain` variant in
/// `crustify/artifacts/build.json`, without its generator and compiler, for which `CMake`'s
/// defaults and the C wrappers' compiler (`CC`, or the system's) stand in, its side campaigns'
/// installs, for which the system's OpenSSL, PCRE2 and zlib do, and its tests, which a dependent
/// does not build. `BUILD_SHARED_LIBS` follows the mode, and `BUILD_CLI` the `git2` link unit.
const CMAKE_OPTIONS: &[&str] = &[
    "-DUSE_HTTPS=OpenSSL",
    "-DUSE_SHA1=builtin",
    "-DUSE_SHA256=HTTPS",
    "-DUSE_COMPRESSION=zlib",
    "-DUSE_REGEX=pcre2",
    "-DUSE_HTTP_PARSER=builtin",
    "-DUSE_SSH=OFF",
    "-DUSE_AUTH_NTLM=builtin",
    "-DUSE_AUTH_NEGOTIATE=OFF",
    "-DUSE_I18N=OFF",
    "-DUSE_THREADS=ON",
    "-DDEPRECATE_HARD=ON",
    "-DBUILD_TESTS=OFF",
    "-DBUILD_EXAMPLES=OFF",
    "-DBUILD_FUZZERS=OFF",
    "-DCMAKE_BUILD_TYPE=RelWithDebInfo",
    // The static library is linked into position-independent Rust executables.
    "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
    // The include dirs and definitions of the link units' TUs, whatever the generator.
    "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
];

/// The files of `c/` the bundled and shared builds read.
const C_INPUTS: &[&str] = &["CMakeLists.txt", "cmake", "include", "src", "deps"];

/// The input files of a link unit's `sys/` directory.
const SYS_INPUTS: &[&str] = &[
    "wrapper.h",
    "allowlist.txt",
    "external.txt",
    "shims.h",
    "shims.c",
    "bindings.rs",
    "static_fns.c",
];

/// How the build gets the C.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bundled,
    Shared,
    System,
    Campaign,
}

/// The headers and version of a link unit's C, as a build compiles and links against it.
struct Library {
    include: Vec<PathBuf>,
    /// The preprocessor definitions the bindings and the C wrappers are built with.
    defines: Vec<String>,
    version: String,
    /// The compiler and instrumentation flags of a campaign variant, for the C wrappers.
    cc: Option<(String, Vec<String>)>,
}

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // `cargo bsan` compiles with `--cfg bsan`; tests use it to skip paths it cannot run.
    println!("cargo::rustc-check-cfg=cfg(bsan)");
    println!("cargo::rustc-check-cfg=cfg(crustify_regenerated)");
    for v in [
        "CRUSTIFY_BUILD_VARIANT",
        "CRUSTIFY_BUILD_DIR",
        "CRUSTIFY_BINDINGS_UPDATE",
        "CRUSTIFY_STATICLIB",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }

    let mode = mode();
    let regenerate = cfg!(feature = "bindgen");
    if mode == Mode::Campaign && regenerate {
        pin_llvm(&root);
    }
    let units: Vec<&LinkUnit> = UNITS.iter().filter(|u| u.enabled()).collect();
    if matches!(mode, Mode::System | Mode::Shared) {
        // An install, or a shared build, has a library for each library link unit, and none
        // for an executable's objects.
        let executables: Vec<&str> = units
            .iter()
            .filter(|u| u.lib.is_none())
            .map(|u| u.name)
            .collect();
        let feature = if mode == Mode::System {
            "system"
        } else {
            "shared"
        };
        assert!(
            executables.is_empty(),
            "the `{feature}` feature links libraries, and no library holds the objects of the \
             executable link units {executables:?}: build the bundled C, or leave out their \
             features (`--no-default-features --features libgit2,{feature}`)"
        );
    }
    let libraries: Vec<Library> = match mode {
        Mode::Campaign => campaign_libraries(&root, &units),
        Mode::System => units
            .iter()
            .map(|unit| installed_library(unit, &root))
            .collect(),
        Mode::Bundled | Mode::Shared => {
            let shared = mode == Mode::Shared;
            let cli = units.iter().any(|u| u.lib.is_none());
            let build = build_c(&root, shared, cli);
            units
                .iter()
                .map(|unit| match unit.lib {
                    Some(_) => built_library(unit, &root, &build, shared, regenerate),
                    None => built_executable_objects(unit, &root, &build),
                })
                .collect()
        }
    };
    for (unit, lib) in units.iter().zip(&libraries) {
        bindings(unit, &root, lib, mode, regenerate);
    }
    if regenerate && mode != Mode::Campaign {
        println!("cargo:rustc-cfg=crustify_regenerated");
    }
}

/// Has bindgen generate with the LLVM `Cargo.toml` pins (`bindgen-llvm`, at
/// `bindgen-llvm-version`), so that every campaign build regenerates the committed bytes.
///
/// bindgen loads libclang at run time through clang-sys, which takes the library
/// `LIBCLANG_PATH` names, and asks the clang `CLANG_PATH` names for the system include dirs;
/// unset, they fall back to a search whose result depends on the host. A driver may set its
/// own: `cargo bsan` points both at `BorrowSanitizer`'s LLVM, whose libclang keeps the doc
/// comments of macro constants that the pinned one drops. Both are set here, in this
/// process, to the pinned LLVM's, before bindgen reads them.
fn pin_llvm(root: &Path) {
    let manifest = root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    // A key of the [workspace.metadata.crustify] table, whose values are plain strings.
    let value = |key: &str| {
        let mut table = false;
        text.lines()
            .find_map(|l| {
                let l = l.trim();
                if l.starts_with('[') {
                    table = l == "[workspace.metadata.crustify]";
                    return None;
                }
                let (k, v) = l.split_once('=')?;
                (table && k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
            })
            .unwrap_or_else(|| {
                panic!(
                    "{} pins no `{key}` in [workspace.metadata.crustify]",
                    manifest.display()
                )
            })
    };
    let prefix = PathBuf::from(value("bindgen-llvm"));
    let version = value("bindgen-llvm-version");
    let libclang = prefix.join("lib/libclang.so");
    let clang = prefix.join("bin/clang");
    let reported = Command::new(&clang)
        .arg("--version")
        .output()
        .ok()
        .map_or_default(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    assert!(
        libclang.exists() && reported.contains(&format!("clang version {version} ")),
        "the raw bindings are generated with LLVM {version}, which Cargo.toml pins at {}: \
         {} or {} is missing, or not that version ({:?}); install it, or regenerate the \
         bindings with another LLVM and pin that one",
        prefix.display(),
        libclang.display(),
        clang.display(),
        reported.lines().next().unwrap_or_default()
    );
    // SAFETY: the build script has started no thread: nothing reads or writes the
    // environment concurrently.
    unsafe { env::set_var("LIBCLANG_PATH", &libclang) };
    // SAFETY: as above, the build script is still single-threaded.
    unsafe { env::set_var("CLANG_PATH", &clang) };
    println!("cargo:rerun-if-changed={}", manifest.display());
}

/// The build mode: campaign mode when the environment names a campaign build, else the one
/// the features select.
fn mode() -> Mode {
    let set = |v: &str| env::var_os(v).is_some_and(|s| !s.is_empty());
    let shared = cfg!(feature = "shared");
    let system = cfg!(feature = "system");
    if set("CRUSTIFY_BUILD_VARIANT") || set("CRUSTIFY_BUILD_DIR") {
        if shared || system {
            println!(
                "cargo:warning=campaign mode ignores the `shared` and `system` features: set \
                 CRUSTIFY_BUILD_VARIANT empty to build as a dependent would"
            );
        }
        return Mode::Campaign;
    }
    match (shared, system) {
        (false, false) => Mode::Bundled,
        (true, false) => Mode::Shared,
        (false, true) => Mode::System,
        (true, true) => panic!("the `shared` and `system` features exclude each other"),
    }
}

/// Generates, checks or picks a link unit's bindings, and compiles its C wrappers.
fn bindings(unit: &LinkUnit, root: &Path, lib: &Library, mode: Mode, regenerate: bool) {
    let input = unit.sys_dir(root);
    for f in SYS_INPUTS {
        println!("cargo:rerun-if-changed={}", input.join(f).display());
    }
    // Dependents' build scripts read it as `DEP_GIT2_SYS_DIR_<LINK_UNIT>`.
    println!("cargo:sys_dir_{}={}", unit.name, input.display());
    assert!(
        regenerate || lib.version == unit.c_version,
        "{} {} found, but the bindings were generated from {}: build {} {}, or enable the \
         `bindgen` feature to regenerate them",
        unit.name,
        lib.version,
        unit.c_version,
        unit.name,
        unit.c_version
    );

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join(unit.name);
    fs::create_dir_all(&out).unwrap();
    let mut bindings = input.join("bindings.rs");
    let mut static_fns = input.join("static_fns.c");
    let mut version = unit.c_version;
    if regenerate {
        let (generated, fns) = generate(unit, &input, lib, &out);
        if mode == Mode::Campaign {
            check_or_update(&bindings, &generated, &out);
            check_or_update(&static_fns, fns.as_deref().unwrap_or(""), &out);
        } else {
            bindings = out.join("bindings.rs");
            fs::write(&bindings, generated).unwrap();
            static_fns = out.join("static_fns.c");
            fs::write(&static_fns, fns.unwrap_or_default()).unwrap();
            version = &lib.version;
        }
    }
    write_doctest_bindings(&bindings, &out.join("bindings.doctest.rs"));
    println!(
        "cargo:rustc-env=CRUSTIFY_C_VERSION_{}={version}",
        unit.name.to_uppercase()
    );
    compile_wrappers(unit, &input, lib, &static_fns);
}

/// Writes the bindings rustdoc collects doctests from: the bindings with their doc comments
/// made plain comments, since the C comments bindgen keeps hold C, which rustdoc would compile
/// as Rust. Each comment's opening marker starts its line in bindgen's output, and a block
/// comment's nesting is kept, `/*` opening one as `/**` does.
fn write_doctest_bindings(bindings: &Path, doctest: &Path) {
    let text =
        fs::read_to_string(bindings).unwrap_or_else(|e| panic!("{}: {e}", bindings.display()));
    let mut plain = String::with_capacity(text.len());
    for line in text.lines() {
        let body = line.trim_start();
        plain.push_str(&line[..line.len() - body.len()]);
        if let Some(rest) = body.strip_prefix("///") {
            plain.push_str("// ");
            plain.push_str(rest);
        } else if let Some(rest) = body.strip_prefix("/**") {
            plain.push_str("/* ");
            plain.push_str(rest);
        } else {
            plain.push_str(body);
        }
        plain.push('\n');
    }
    fs::write(doctest, plain).unwrap();
}

// Bundled and shared modes.

/// Builds `c/` into `OUT_DIR` with its own `CMake`, static or shared, and returns the build
/// directory.
///
/// Only the library is built (`libgit2package`, `CMake`'s target of `libgit2.a` or
/// `libgit2.so`). With `cli`, the CLI is configured too, so that `compile_commands.json`
/// records how its TUs compile, for [`built_executable_objects`], but its executable, which
/// links the C's `main`, is not built.
fn build_c(root: &Path, shared: bool, cli: bool) -> PathBuf {
    let c = root.join("c");
    for f in C_INPUTS {
        println!("cargo:rerun-if-changed={}", c.join(f).display());
    }
    for v in ["CMAKE", "CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH"] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    let cmake = env::var_os("CMAKE").unwrap_or_else(|| "cmake".into());
    let build = PathBuf::from(env::var("OUT_DIR").unwrap()).join("c");
    // A build configured from another source dir, the same crate's at another path (its
    // packaged copy, say), cannot be reconfigured: it is started afresh.
    let cache = build.join("CMakeCache.txt");
    if cache.exists() && cache_var(&cache, "CMAKE_HOME_DIRECTORY:") != c.display().to_string() {
        fs::remove_dir_all(&build).unwrap();
    }
    fs::create_dir_all(&build).unwrap();
    // The compiler the C wrappers are compiled with, so that both halves agree.
    let compiler = cc::Build::new().get_compiler();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&c)
        .arg("-B")
        .arg(&build)
        .args(CMAKE_OPTIONS)
        .arg(format!(
            "-DBUILD_SHARED_LIBS={}",
            if shared { "ON" } else { "OFF" }
        ))
        .arg(format!("-DBUILD_CLI={}", if cli { "ON" } else { "OFF" }))
        .arg(format!("-DCMAKE_C_COMPILER={}", compiler.path().display()))
        .current_dir(&build);
    run(&mut configure);
    let mut compile = Command::new(&cmake);
    compile
        .arg("--build")
        .arg(&build)
        .args(["--target", "libgit2package"])
        .current_dir(&build);
    // CMake reads CMAKE_BUILD_PARALLEL_LEVEL itself; otherwise the build takes cargo's jobs.
    if env::var_os("CMAKE_BUILD_PARALLEL_LEVEL").is_none()
        && let Ok(jobs) = env::var("NUM_JOBS")
    {
        compile.arg("--parallel").arg(jobs);
    }
    run(&mut compile);
    build
}

/// Runs a build command, failing the build with its command line when it fails.
fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    assert!(status.success(), "{command:?} failed: {status}");
}

/// A link unit's library as the bundled or shared build of `c/` made it, linked.
fn built_library(
    unit: &LinkUnit,
    root: &Path,
    build: &Path,
    shared: bool,
    regenerate: bool,
) -> Library {
    let pc = build.join(format!("{}.pc", unit.version_pc));
    println!("cargo:rustc-link-search=native={}", build.display());
    if shared {
        let so = build.join(unit.artifact);
        refuse_unexported(unit, root, &so, regenerate, "shared");
        println!("cargo:rustc-link-lib=dylib={}", unit.link_name());
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        ship_beside_binaries(&so);
        export_dynamic_symbols(root);
    } else {
        println!("cargo:rustc-link-lib=static={}", unit.link_name());
        link_private_dependencies(unit, &pc);
    }
    let (include, defines) = compile_commands_flags(build, unit);
    let mut dirs = vec![root.join("c/include")];
    for dir in include {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    Library {
        include: dirs,
        defines,
        version: pc_field(&pc, "Version"),
        cc: None,
    }
}

/// A link unit's executable as the bundled build makes it: not linked, since it links the C's
/// `main`, but its own TUs compiled, each by the command `CMake`'s `compile_commands.json`
/// records for it, run as the build system would run it, and their objects archived into
/// `libcrustify_<link-unit>.a`, linked statically, as the variants' static archive holds them.
/// The objects of the libraries the executable links besides are the library link units'.
/// `main.c.o` is archived too, and is linked only when referenced, as [`UNITS`] describes.
fn built_executable_objects(unit: &LinkUnit, root: &Path, build: &Path) -> Library {
    let commands = unit_compile_commands(build, unit);
    let mut objects = Vec::new();
    for c in &commands {
        let object = c.directory.join(&c.output);
        fs::create_dir_all(object.parent().unwrap()).unwrap();
        run(Command::new("sh")
            .arg("-c")
            .arg(&c.command)
            .current_dir(&c.directory));
        objects.push(object);
    }
    let archive = build.join(format!("libcrustify_{}.a", unit.name));
    fs::remove_file(&archive).ok();
    // The archiver cc would use, run as a plain command, whose failure names no environment.
    let ar = cc::Build::new().get_archiver().get_program().to_owned();
    run(Command::new(ar).arg("crs").arg(&archive).args(&objects));
    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=static=crustify_{}", unit.name);
    let mut include = vec![root.join("c/include")];
    for dir in include_dirs(&commands[0].command)
        .into_iter()
        .map(PathBuf::from)
    {
        if !include.contains(&dir) {
            include.push(dir);
        }
    }
    Library {
        include,
        defines: define_flags(&commands[0].command),
        version: pc_field(&build.join(format!("{}.pc", unit.version_pc)), "Version"),
        cc: None,
    }
}

/// One TU's entry of a `CMake` build's `compile_commands.json`.
struct CompileCommand {
    directory: PathBuf,
    command: String,
    /// The object it writes, relative to `directory`.
    output: String,
}

/// The entries of a link unit's own TUs, those compiled under `CMakeFiles/<target>.dir/`, in
/// the order `compile_commands.json` records them.
fn unit_compile_commands(build: &Path, unit: &LinkUnit) -> Vec<CompileCommand> {
    let path = build.join("compile_commands.json");
    let commands: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("{}: {e}", path.display());
        }))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let dir = format!("CMakeFiles/{}.dir/", unit.target);
    let entries: Vec<CompileCommand> = commands
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let command = c["command"].as_str()?.to_string();
            // The entry's `output` is relative to the build's top dir, its command's `-o`
            // to the entry's `directory`, where the command runs.
            let mut words = command.split_whitespace();
            words.find(|w| *w == "-o")?;
            let output = words.next()?.to_string();
            output.contains(&dir).then(|| CompileCommand {
                directory: PathBuf::from(c["directory"].as_str().unwrap_or_default()),
                command,
                output,
            })
        })
        .collect();
    assert!(
        !entries.is_empty(),
        "{} compiles no object under {dir}",
        path.display()
    );
    entries
}

/// The include dirs and definitions of the first of a link unit's TUs that a `CMake` build's
/// `compile_commands.json` records.
fn compile_commands_flags(build: &Path, unit: &LinkUnit) -> (Vec<PathBuf>, Vec<String>) {
    let commands = unit_compile_commands(build, unit);
    let command = &commands[0].command;
    let include = include_dirs(command)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    (include, define_flags(command))
}

/// Links what a static library's `pkg-config` file records as its private dependencies: its
/// `Libs.private` libraries, and the `Requires.private` modules `pkg-config` resolves.
fn link_private_dependencies(unit: &LinkUnit, pc: &Path) {
    for flag in pc_field(pc, "Libs.private").split_whitespace() {
        if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib=dylib={lib}");
        } else if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        }
    }
    let requires = pc_field(pc, "Requires.private").replace(',', " ");
    let mut words = requires.split_whitespace();
    while let Some(module) = words.next() {
        if ["=", "!=", "<", "<=", ">", ">="].contains(&module) {
            // A version constraint: its operator, then its version.
            words.next();
            continue;
        }
        pkg_config::Config::new()
            .probe(module)
            .unwrap_or_else(|e| panic!("{module}, which {} links, is not found: {e}", unit.name));
    }
}

/// Puts a shared build's library beside the binaries cargo builds, where their `$ORIGIN`
/// rpath finds it: the target directory's profile directory, its `deps/` (tests) and its
/// `examples/`. The library is written under its soname, through a rename, so that a binary
/// running from the old copy keeps its mapping.
fn ship_beside_binaries(so: &Path) {
    let soname = fs::read_link(so)
        .unwrap_or_else(|e| panic!("{}: {e}", so.display()))
        .file_name()
        .unwrap()
        .to_owned();
    let build = so.parent().unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out.
    let Some(profile) = out.ancestors().nth(3) else {
        return;
    };
    for dir in [
        profile.to_path_buf(),
        profile.join("deps"),
        profile.join("examples"),
    ] {
        if !dir.is_dir() {
            continue;
        }
        let tmp = dir.join(format!(".{}.tmp", soname.to_string_lossy()));
        fs::copy(build.join(&soname), &tmp).unwrap();
        fs::rename(&tmp, dir.join(&soname)).unwrap();
    }
}

// System mode.

/// A link unit's library as an installed system finds it: under `<LIB>_DIR`, or through
/// `pkg-config`.
fn installed_library(unit: &LinkUnit, root: &Path) -> Library {
    let lib = unit.library();
    // An install holds the C of every module; one switched to Rust would be linked twice, as
    // that C and as the Rust that replaces it.
    let (switched, enabled) = gateways(root);
    assert!(
        switched.is_empty() && enabled.is_empty(),
        "the `system` feature links an install of {lib}, which holds the C of the modules \
         switched to Rust ({switched:?}) and of those whose exports are enabled ({enabled:?}): \
         build the bundled C instead"
    );
    // An install carries no internal header.
    let internal = internal_headers(&unit.sys_dir(root).join("wrapper.h"));
    assert!(
        internal.is_empty(),
        "the `system` feature links an install of {lib}, which lacks the internal headers the \
         bindings reach ({}): build the bundled C instead",
        internal.join(", ")
    );
    let regenerate = cfg!(feature = "bindgen");
    let dir_var = format!("{}_DIR", lib.to_uppercase());
    println!("cargo:rerun-if-env-changed={dir_var}");
    if let Some(dir) = env::var_os(&dir_var).map(PathBuf::from) {
        let lib_dir = dir.join("lib");
        refuse_unexported(
            unit,
            root,
            &lib_dir.join(unit.artifact),
            regenerate,
            "system",
        );
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib={}", unit.link_name());
        return Library {
            include: vec![dir.join("include")],
            defines: unit.defines.iter().map(|d| (*d).to_string()).collect(),
            version: pc_field(&lib_dir.join(format!("pkgconfig/{lib}.pc")), "Version"),
            cc: None,
        };
    }
    let found = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe(lib)
        .unwrap_or_else(|e| panic!("{lib} not found: set {dir_var} to its install prefix ({e})"));
    let system_dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/usr/lib",
    ];
    if let Some(so) = found
        .link_paths
        .iter()
        .map(PathBuf::as_path)
        .chain(system_dirs.iter().map(Path::new))
        .map(|d| d.join(unit.artifact))
        .find(|so| so.exists())
    {
        refuse_unexported(unit, root, &so, regenerate, "system");
    }
    pkg_config::Config::new().probe(lib).unwrap();
    Library {
        include: found.include_paths,
        defines: unit.defines.iter().map(|d| (*d).to_string()).collect(),
        version: found.version,
        cc: None,
    }
}

/// The internal headers `wrapper.h` includes: every `#include "…"` but the shims'.
fn internal_headers(wrapper: &Path) -> Vec<String> {
    fs::read_to_string(wrapper)
        .unwrap_or_else(|e| panic!("{}: {e}", wrapper.display()))
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#include \""))
        .filter_map(|l| l.split_once('"').map(|(h, _)| h.to_string()))
        .filter(|h| h != "shims.h")
        .collect()
}

/// Fails the build when a shared library does not export each function and global the
/// link unit's bindings declare, as a `mode` build links it: the shims and the wrappers of
/// the `static inline` functions, which the build compiles itself, aside.
fn refuse_unexported(unit: &LinkUnit, root: &Path, so: &Path, regenerate: bool, mode: &str) {
    let bindings = unit.sys_dir(root).join("bindings.rs");
    let exported = dynamic_symbols(so);
    let missing: Vec<String> = extern_items(&bindings)
        .into_iter()
        .filter(|name| !exported.contains(name))
        .collect();
    let regenerated = if regenerate {
        ", even when regenerated from its headers"
    } else {
        ""
    };
    assert!(
        missing.is_empty(),
        "the `{mode}` mode links {}, which does not export {} of the items the bindings \
         declare{regenerated} ({}{}): build the bundled C instead",
        so.display(),
        missing.len(),
        missing
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        if missing.len() > 8 { ", …" } else { "" }
    );
}

/// The functions and globals the `extern` blocks of generated bindings declare, but those
/// bound under another symbol (`#[link_name]`) and the `crustify_` shims.
fn extern_items(bindings: &Path) -> Vec<String> {
    let text =
        fs::read_to_string(bindings).unwrap_or_else(|e| panic!("{}: {e}", bindings.display()));
    let mut items = Vec::new();
    let mut renamed = false;
    let mut in_extern = false;
    for line in text.lines() {
        let t = line.trim();
        if line.ends_with("extern \"C\" {") && !line.starts_with(' ') {
            in_extern = true;
            continue;
        } else if line == "}" {
            in_extern = false;
            continue;
        } else if !in_extern {
            continue;
        }
        if t.starts_with("#[link_name") {
            renamed = true;
            continue;
        }
        let decl = t
            .strip_prefix("pub fn ")
            .or_else(|| t.strip_prefix("pub static mut "))
            .or_else(|| t.strip_prefix("pub static "));
        if let Some(decl) = decl {
            let name: String = decl
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !renamed && !name.starts_with("crustify_") {
                items.push(name);
            }
        }
        if !t.starts_with("#[") {
            renamed = false;
        }
    }
    items
}

/// The defined, default-visibility global and weak symbols of an ELF64 shared library's
/// dynamic symbol table.
fn dynamic_symbols(so: &Path) -> BTreeSet<String> {
    let data = fs::read(so).unwrap_or_else(|e| panic!("{}: {e}", so.display()));
    let bad = || -> ! { panic!("{}: not a little-endian ELF64 file", so.display()) };
    if data.get(..6) != Some(b"\x7fELF\x02\x01".as_slice()) {
        bad();
    }
    let int = |at: usize, len: usize| -> usize {
        let bytes = data.get(at..at + len).unwrap_or_else(|| bad());
        let mut word = [0u8; 8];
        word[..len].copy_from_slice(bytes);
        usize::try_from(u64::from_le_bytes(word)).unwrap_or_else(|_| bad())
    };
    let (shoff, shentsize, shnum) = (int(0x28, 8), int(0x3A, 2), int(0x3C, 2));
    let section = |i: usize| shoff + i * shentsize;
    let mut symbols = BTreeSet::new();
    for i in 0..shnum {
        // SHT_DYNSYM, whose sh_link names its string table.
        if int(section(i) + 4, 4) != 11 {
            continue;
        }
        let (offset, size, entsize) = (
            int(section(i) + 0x18, 8),
            int(section(i) + 0x20, 8),
            int(section(i) + 0x38, 8),
        );
        let strtab = int(section(int(section(i) + 0x28, 4)) + 0x18, 8);
        for sym in (offset..offset + size).step_by(entsize.max(1)).skip(1) {
            let (info, other, shndx) = (int(sym + 4, 1), int(sym + 5, 1), int(sym + 6, 2));
            // Defined (st_shndx not SHN_UNDEF), global or weak, default visibility.
            if shndx == 0 || !matches!(info >> 4, 1 | 2) || other & 3 != 0 {
                continue;
            }
            let name = strtab + int(sym, 4);
            let end = data[name..]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or_else(|| bad());
            symbols.insert(String::from_utf8_lossy(&data[name..name + end]).into_owned());
        }
    }
    symbols
}

// Campaign mode.

/// Each link unit's library in a campaign build, the variant's or the private build's,
/// linked.
///
/// The variant's `static` archive, when `build.json` records one, is linked statically, once,
/// and each link unit's shared library otherwise; a private build follows the record of the
/// variant `CRUSTIFY_BUILD_VARIANT` names, and must carry the same archive. Either way, the
/// libraries each link unit links, and the include dirs and definitions its own TUs compile
/// with, are read from the build's `build.ninja`, but for the Rust library the build's C links
/// (its `LIBGIT2_RUST_LIBRARY`), which is this crate.
///
/// With `CRUSTIFY_STATICLIB` set, the crate is that Rust library, built before the build's C
/// is linked: only the build's configuration is read, and nothing of it is linked.
fn campaign_libraries(root: &Path, units: &[&LinkUnit]) -> Vec<Library> {
    let staticlib = env::var_os("CRUSTIFY_STATICLIB").is_some_and(|v| !v.is_empty());
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
    let archive = record
        .as_ref()
        .and_then(|r| r["static"].as_str())
        .map(|rel| (rel, build.join(rel)));
    let ninja = Ninja::read(&build);
    if staticlib {
        return staticlib_libraries(root, units, &build, &ninja, &variant);
    }
    let rust = cache_entry(&build.join("CMakeCache.txt"), "LIBGIT2_RUST_LIBRARY:")
        .map(|lib| build.join(lib));
    let mut libraries = Vec::new();
    let mut rpath = Vec::new();
    let mut linked = Vec::new();
    let mut archived = false;
    for unit in units {
        let link = ninja.link_edge(unit.artifact);
        let artifact = build.join(&link.output);
        assert!(
            artifact.exists(),
            "{} is missing: build it first",
            artifact.display()
        );
        println!("cargo:rerun-if-changed={}", artifact.display());
        if let Some((rel, archive)) = &archive {
            // One archive holds every link unit's objects: it is linked once.
            if !archived {
                link_static(rel, archive, &build);
                archived = true;
            }
            check_archived(archive, &artifact);
        } else {
            assert!(
                unit.lib.is_some(),
                "the `{}` link unit is an executable, whose objects only a variant's static \
                 archive holds: record a `static` archive for the variant in build.json",
                unit.name
            );
            println!("cargo:rustc-link-search=native={}", build.display());
            println!("cargo:rustc-link-lib=dylib={}", unit.link_name());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", build.display());
        }
        // The libraries the link unit links, linked as it links them: the out-of-tree ones
        // from their side campaigns' installs of the same variant, by the absolute paths the
        // build recorded, so that every checkout, a worktree's included, links the same ones;
        // the system's by name. A statically linked library needs them itself, and a shared
        // one loads them through its own rpath; the installs' dirs go on the rpath either way,
        // and to dependents (`DEP_GIT2_DEP_LIB_DIRS`) for theirs, so that a binary loads the
        // very libraries the build was linked with.
        let libs = LinkLibraries::parse(&link.var("LINK_LIBRARIES"), &build, rust.as_deref());
        for dir in &libs.search {
            println!("cargo:rustc-link-search=native={dir}");
        }
        if archive.is_some() {
            for lib in libs.libs {
                if !linked.contains(&lib) {
                    println!("cargo:rustc-link-lib={}={}", lib.0, lib.1);
                    linked.push(lib);
                }
            }
        }
        for dir in libs.rpath {
            if !rpath.contains(&dir) {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
                rpath.push(dir);
            }
        }
        libraries.push(campaign_headers(unit, root, &build, &ninja, link));
    }
    if archive.is_none() {
        // The remaining C's shared libraries resolve the ported modules against the binary.
        export_dynamic_symbols(root);
    }
    println!("cargo:dep_lib_dirs={}", rpath.join(":"));
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
    libraries
}

/// Each link unit's headers, definitions and version in a campaign build, read from its
/// configured build, for the Rust library that build's own C links: nothing of the build is
/// linked, and it need not be built yet.
fn staticlib_libraries(
    root: &Path,
    units: &[&LinkUnit],
    build: &Path,
    ninja: &Ninja,
    variant: &str,
) -> Vec<Library> {
    let libraries = units
        .iter()
        .map(|unit| campaign_headers(unit, root, build, ninja, ninja.link_edge(unit.artifact)))
        .collect();
    println!("cargo:variant={variant}");
    println!("cargo:build_dir={}", build.display());
    libraries
}

/// A link unit's headers, definitions and version in a campaign build: the bindings see the
/// internal headers as the link unit's own TUs do, from this checkout's `c/` in place of the
/// source dir the build was configured from.
fn campaign_headers(
    unit: &LinkUnit,
    root: &Path,
    build: &Path,
    ninja: &Ninja,
    link: &Edge,
) -> Library {
    let source = cache_var(&build.join("CMakeCache.txt"), "CMAKE_HOME_DIRECTORY:");
    let compile = ninja.compile_edge(link, &format!("CMakeFiles/{}.dir/", unit.target));
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
    Library {
        include,
        defines: define_flags(&format!(
            "{} {}",
            compile.var("DEFINES"),
            compile.var("FLAGS")
        )),
        version: pc_field(&build.join(format!("{}.pc", unit.version_pc)), "Version"),
        cc: Some(variant_cc(&build.join("CMakeCache.txt"))),
    }
}

/// Links a variant's static archive, which `crustify/scripts/static-archive.sh` made from the
/// objects its link units link, as the last step of its build.
///
/// The archive is linked unbundled (`-bundle`): the rlib does not copy its objects, and each
/// final binary links it from the variant's build, after the rlibs. Rust calls into the C
/// resolve from it as the linker meets them; the C's calls into the Rust exports of a ported
/// module resolve too, although their rlib comes first, since rustc's `symbols.o`, which
/// leads every link, references each exported symbol of the upstream crates, so that their
/// objects are already linked when the archive is read. No group or repetition is needed,
/// with rust-lld, the default, or GNU ld.
fn link_static(rel: &str, archive: &Path, build: &Path) {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("lib"))
        .and_then(|n| n.strip_suffix(".a"))
        .unwrap_or_else(|| panic!("{}: not named lib<name>.a", archive.display()));
    if !archive.exists() {
        let units: Vec<&str> = UNITS.iter().map(|u| u.name).collect();
        panic!(
            "{} is missing: run crustify/scripts/static-archive.sh {rel} {} in {}, as the last \
             step of the variant's build command",
            archive.display(),
            units.join(" "),
            build.display()
        );
    }
    println!("cargo:rerun-if-changed={}", archive.display());
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static:-bundle={name}");
}

/// Fails the build when a link unit's artifact was relinked since the archive was made.
fn check_archived(archive: &Path, artifact: &Path) {
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let archived = mtime(archive);
    assert!(
        mtime(artifact).is_none_or(|linked| archived.is_some_and(|a| linked <= a)),
        "{} is older than {}, which was relinked since: rerun the variant's build command, \
         whose last step archives its objects again",
        archive.display(),
        artifact.display()
    );
}

/// The edges of a `CMake` build's `build.ninja` that the campaign link reads.
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
            .map_or_default(|(_, v)| v.clone())
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

    /// The edge that links `name`, followed through `CMake`'s symlink edges.
    fn link_edge(&self, name: &str) -> &Edge {
        let mut name = name.to_string();
        for _ in 0..4 {
            let edge = self
                .edges
                .iter()
                .find(|e| e.outputs.contains(&name))
                .unwrap_or_else(|| panic!("build.ninja has no edge building {name}"));
            // A shared library's or an executable's link rule.
            if edge.rule.contains("_LINKER") {
                return edge;
            }
            name = edge
                .inputs
                .first()
                .unwrap_or_else(|| panic!("{name} is built from nothing"))
                .clone();
        }
        panic!("build.ninja links no {name}")
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
    /// The libraries of `value`, but `rust`, the crate's own static library, which the
    /// build's C links in its place, and which `value` may name relative to `build`, and
    /// either may reach through a link.
    fn parse(value: &str, build: &Path, rust: Option<&Path>) -> Self {
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
            if rust.is_some_and(|r| same_file(&build.join(word), r)) {
                continue;
            }
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

/// Whether two paths name the same file, through any links, or are the same path.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// A value of a `CMake` cache, by its `NAME:` prefix.
fn cache_var(cache: &Path, name: &str) -> String {
    cache_entry(cache, name).unwrap_or_else(|| panic!("{} records no {name}", cache.display()))
}

/// A value of a `CMake` cache, by its `NAME:` prefix, when it records one that is not empty.
fn cache_entry(cache: &Path, name: &str) -> Option<String> {
    let text = fs::read_to_string(cache).unwrap_or_else(|e| panic!("{}: {e}", cache.display()));
    text.lines()
        .find_map(|l| l.strip_prefix(name).and_then(|r| r.split_once('=')))
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

/// The compiler of a campaign variant, and the instrumentation flags of its C flags, as its
/// `CMake` cache records them.
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

// Every mode.

/// The include dirs of a compile command, `-I` and `-isystem` alike, in order.
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

/// The preprocessor definitions of a compile command's `-D` flags.
fn define_flags(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter_map(|f| f.strip_prefix("-D"))
        .map(String::from)
        .collect()
}

/// A field of a `pkg-config` file, empty when the file does not record it.
fn pc_field(pc: &Path, field: &str) -> String {
    println!("cargo:rerun-if-changed={}", pc.display());
    fs::read_to_string(pc)
        .unwrap_or_else(|e| panic!("{}: {e}", pc.display()))
        .lines()
        .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))
        .map_or_default(|v| v.trim().to_string())
}

/// Compiles a link unit's macro shims and the wrappers of its `static inline` functions.
fn compile_wrappers(unit: &LinkUnit, input: &Path, lib: &Library, static_fns: &Path) {
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
    cc.compile(&format!("crustify_{}_wrappers", unit.name));
}

/// Fails the build when `path` does not hold `content`, unless asked to write it; the content
/// generated is left in `out` for comparison.
fn check_or_update(path: &Path, content: &str, out: &Path) {
    if fs::read_to_string(path).unwrap_or_default() == content {
        return;
    }
    if env::var_os("CRUSTIFY_BINDINGS_UPDATE").is_some() {
        // Empty or not: the layout holds each generated file.
        fs::write(path, content).unwrap();
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

/// The `ffi_export` gateways of `src/`: the files of those that compile unconditionally,
/// the modules switched to Rust, and the export features of those enabled.
///
/// A gateway is a `mod ffi_export`, under `#[cfg(feature = "ffi-export-<tu>")]` until its
/// module switches.
fn gateways(root: &Path) -> (Vec<String>, Vec<String>) {
    let enabled = enabled_export_features();
    let mut switched = Vec::new();
    let mut on = Vec::new();
    for (file, gate, _) in gateway_lines(root) {
        match gate {
            None => switched.push(file.display().to_string()),
            Some(g) if enabled.contains(&g) => on.push(g),
            Some(_) => {}
        }
    }
    (switched, on)
}

/// The `ffi-export-*` features enabled, as their `CARGO_FEATURE_` suffixes.
fn enabled_export_features() -> Vec<String> {
    env::vars()
        .filter_map(|(k, _)| {
            k.strip_prefix("CARGO_FEATURE_FFI_EXPORT_")
                .map(String::from)
        })
        .collect()
}

/// Each `ffi_export` gateway of `src/`: its file, its export feature (as a `CARGO_FEATURE_`
/// suffix) when it has one, and its lines.
fn gateway_lines(root: &Path) -> Vec<(PathBuf, Option<String>, Vec<String>)> {
    let src = root.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let braces =
        |s: &str| s.matches('{').count().cast_signed() - s.matches('}').count().cast_signed();
    let mut gateways = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let mut gate: Option<String> = None;
        let mut open: Option<(Option<String>, isize, Vec<String>)> = None;
        for line in text.lines() {
            let t = line.trim();
            if let Some((_, depth, lines)) = open.as_mut() {
                lines.push(t.to_string());
                *depth += braces(t);
                if *depth <= 0 {
                    let (g, _, lines) = open.take().unwrap();
                    gateways.push((file.clone(), g, lines));
                }
            } else if let Some(feature) = t
                .strip_prefix("#[cfg(feature = \"ffi-export-")
                .and_then(|r| r.strip_suffix("\")]"))
            {
                gate = Some(feature.to_uppercase().replace('-', "_"));
            } else if t.starts_with("mod ffi_export") || t.starts_with("pub mod ffi_export") {
                open = Some((gate.take(), braces(t), Vec::new()));
            } else if !t.starts_with("#[") && !t.starts_with("//") && !t.is_empty() {
                gate = None;
            }
        }
    }
    gateways
}

/// Has the binaries export, under their C names, what the `ffi_export` gateways compiled in
/// define, so that the remaining C's shared libraries resolve their references to the ported
/// modules against them, as the conventions' Exports section describes. An export is an item
/// under `#[unsafe(no_mangle)]` on a line of its own.
fn export_dynamic_symbols(root: &Path) {
    let enabled = enabled_export_features();
    for (_, gate, lines) in gateway_lines(root) {
        if gate.is_some_and(|g| !enabled.contains(&g)) {
            continue;
        }
        let mut no_mangle = false;
        for t in lines {
            if t == "#[unsafe(no_mangle)]" {
                no_mangle = true;
            } else if no_mangle && !t.starts_with("#[") && !t.starts_with("//") {
                if let Some(name) = exported_name(&t) {
                    println!("cargo:rustc-link-arg=-Wl,--export-dynamic-symbol={name}");
                }
                no_mangle = false;
            }
        }
    }
}

/// The name an exported function or static declares on its line.
fn exported_name(line: &str) -> Option<&str> {
    let rest = if let Some(i) = line.find("fn ") {
        &line[i + 3..]
    } else {
        let rest = &line[line.find("static ")? + 7..];
        rest.strip_prefix("mut ").unwrap_or(rest)
    };
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// The `.rs` files under `dir`.
fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

#[cfg(not(feature = "bindgen"))]
fn generate(_: &LinkUnit, _: &Path, _: &Library, _: &Path) -> (String, Option<String>) {
    unreachable!("regeneration needs the `bindgen` feature")
}

/// Generates a link unit's bindings, and the C wrappers of its allowlisted `static inline`
/// functions when there are any, from its `sys/<link-unit>/` input.
#[cfg(feature = "bindgen")]
fn generate(_: &LinkUnit, input: &Path, lib: &Library, out: &Path) -> (String, Option<String>) {
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
    fs::remove_file(&path).ok();
    let bindings = b.generate().expect("bindgen").to_string();
    // The wrappers include the header by its absolute path; the build finds it in the link
    // unit's `sys/` directory.
    let fns = fs::read_to_string(&path).ok().map(|c| {
        c.replace(
            &format!("\"{}\"", input.join("wrapper.h").display()),
            "\"wrapper.h\"",
        )
    });
    (bindings, fns)
}
