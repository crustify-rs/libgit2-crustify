//! Embeds the rpaths a campaign build of `libgit2-sys` needs in this package's tests and
//! examples: `rustc-link-arg` from a dependency's build script does not reach dependents, and
//! cargo puts only search paths inside the target directory on the loader path. A variant
//! linked statically (`DEP_GIT2_LINKAGE=static`) needs only the side campaigns' installs it
//! links (`DEP_GIT2_DEP_LIB_DIRS`); one linked shared also needs its build dir, which holds
//! `libgit2.so`.

fn main() {
    // `cargo bsan` compiles with `--cfg bsan`; tests use it to skip paths it cannot run.
    println!("cargo:rustc-check-cfg=cfg(bsan)");
    for v in [
        "DEP_GIT2_LINKAGE",
        "DEP_GIT2_BUILD_DIR",
        "DEP_GIT2_DEP_LIB_DIRS",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    // A campaign build of libgit2-sys reports its build; a dependent's links an install.
    if std::env::var("DEP_GIT2_LINKAGE").as_deref() == Ok("shared")
        && let Ok(dir) = std::env::var("DEP_GIT2_BUILD_DIR")
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
    if let Ok(dirs) = std::env::var("DEP_GIT2_DEP_LIB_DIRS") {
        for dir in dirs.split(':').filter(|d| !d.is_empty()) {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
    }
}
