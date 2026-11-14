//! Compiles C's `opt.c` under other names, `c_cli_opt_*`, beside the safe crate's exports of
//! the same functions, which the export feature links in place of the variant's `opt.c`, so
//! that one binary runs both.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../../../../../../..");
    let root = root.canonicalize().expect("the repository root");
    let mut build = cc::Build::new();
    for dir in [
        "c/src/util",
        "c/src/cli",
        "c/include",
        "crustify/builds/plain/gen_headers",
    ] {
        build.include(root.join(dir));
    }
    for name in [
        "cli_opt_parse",
        "cli_opt_foreach",
        "cli_opt_parser_init",
        "cli_opt_parser_next",
        "cli_opt_status_fprint",
    ] {
        build.define(name, Some(format!("c_{name}").as_str()));
    }
    build
        .define("_GNU_SOURCE", None)
        .define("GIT_DEPRECATE_HARD", None)
        .define("NDEBUG", None)
        .file(root.join("c/src/cli/opt.c"))
        .compile("c_opt");
    println!(
        "cargo:rerun-if-changed={}",
        root.join("c/src/cli/opt.c").display()
    );
}
