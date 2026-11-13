// `DiffParseOptions` holds only an `oid_type` libgit2 names (or 0), so no raw
// `git_diff_parse_options`, whose `oid_type` may be any value, converts into one.
use libgit2::sys::libgit2 as sys;
use libgit2::libgit2::src::libgit2::diff_parse::DiffParseOptions;

fn main() {
    let raw = sys::git_diff_parse_options {
        version: sys::GIT_DIFF_PARSE_OPTIONS_VERSION,
        oid_type: 7,
    };
    let _ = DiffParseOptions::from(raw);
}
