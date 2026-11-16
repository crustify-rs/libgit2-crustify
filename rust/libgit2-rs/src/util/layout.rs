//! Layout tests: check Rust mirrors of C types against the C compiler's
//! view of them (PORTING.md §4.1).
//!
//! For each mirrored or shared-prefix type, a module lists the type, its C
//! name and the C-visible fields, and [`c_layout_test!`](crate::c_layout_test)
//! makes a test that writes a C file of `_Static_assert`s — the C type's
//! `sizeof` and `_Alignof`, and each field's `offsetof` and `sizeof` — from
//! Rust's `size_of`, `align_of` and `offset_of!`, and compiles it with
//! `clang -fsyntax-only` with the include paths and definitions of the
//! libgit2 build in `LIBGIT2_BUILD_DIR` (read from its `build.ninja`). Any
//! mismatch fails the test with the compiler's message.
//!
//! One invocation per module, at the end of the module, adding a type in
//! one line:
//!
//! ```ignore
//! crate::c_layout_test! {
//!     headers: ["str.h"],
//!     types: [
//!         GitStr => "git_str" { ptr, asize, size },
//!         TextStats => "git_str_text_stats" { bom, nul, cr, lf, crlf, printable, nonprintable },
//!         // a Rust field with another name in C: `rust_name as "c_name"`
//!     ],
//!     // optional: raw C assertions, e.g. for bitfields
//!     asserts: ["sizeof(git_str_bom_t) == 4"],
//! }
//! ```
//!
//! Only the macros are in the library; the generator and the checker are
//! compiled for tests alone (`cfg(test)`), so the shipped library has no
//! process-spawning code.
//!
//! The tests need `LIBGIT2_BUILD_DIR` and clang, so they run with
//! `--features link-c` (and are ignored without it and under Miri).
//! `common.h` (of `src/libgit2`) is always included first.

#[cfg(test)]
use std::fmt::Write as _;

#[cfg(test)]
/// A field's place in a Rust type.
#[derive(Debug)]
pub struct FieldLayout {
    /// The field's name in C.
    pub c_name: &'static str,
    /// `offset_of!`.
    pub offset: usize,
    /// The field's `size_of`.
    pub size: usize,
}

#[cfg(test)]
/// A Rust type's layout, and the C type it mirrors.
#[derive(Debug)]
pub struct TypeLayout {
    /// The Rust type, for messages.
    pub rust: &'static str,
    /// The C type expression.
    pub c_type: &'static str,
    /// `size_of`.
    pub size: usize,
    /// `align_of`.
    pub align: usize,
    /// The C-visible fields.
    pub fields: Vec<FieldLayout>,
}

#[cfg(test)]
/// The size of the field a projection closure selects.
pub fn field_size<T, F>(_: fn(&T) -> &F) -> usize {
    size_of::<F>()
}

/// A [`TypeLayout`] for `rust => "c_type" { fields }`.
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! c_layout {
    ($rust:ty => $c:literal { $($field:ident $(as $cname:literal)?),* $(,)? }) => {
        $crate::util::layout::TypeLayout {
            rust: stringify!($rust),
            c_type: $c,
            size: ::core::mem::size_of::<$rust>(),
            align: ::core::mem::align_of::<$rust>(),
            fields: vec![$(
                $crate::util::layout::FieldLayout {
                    c_name: $crate::c_layout!(@name $field $($cname)?),
                    offset: ::core::mem::offset_of!($rust, $field),
                    size: $crate::util::layout::field_size(|s: &$rust| &s.$field),
                },
            )*],
        }
    };
    (@name $field:ident $cname:literal) => { $cname };
    (@name $field:ident) => { stringify!($field) };
}

/// Define this module's layout test (see the [module](crate::util::layout)).
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! c_layout_test {
    (
        headers: [$($h:literal),* $(,)?],
        types: [$($rust:ty => $c:literal { $($body:tt)* }),* $(,)?]
        $(, asserts: [$($a:literal),* $(,)?])? $(,)?
    ) => {
        #[cfg(test)]
        #[test]
        #[cfg_attr(any(miri, not(feature = "link-c")), ignore = "runs clang on LIBGIT2_BUILD_DIR: needs --features link-c, not Miri")]
        fn c_layout() {
            $crate::util::layout::check(
                &[$($h),*],
                &[$($crate::c_layout!($rust => $c { $($body)* })),*],
                &[$($($a),*)?],
            );
        }
    };
}

#[cfg(test)]
/// The C source that asserts `types`' layouts.
pub fn c_source(headers: &[&str], types: &[TypeLayout], asserts: &[&str]) -> String {
    let mut s = String::from("#include \"common.h\"\n#include <stddef.h>\n");
    for h in headers {
        let _ = writeln!(s, "#include \"{h}\"");
    }
    for t in types {
        let c = t.c_type;
        let _ = writeln!(
            s,
            "_Static_assert(sizeof({c}) == {}, \"{c}: sizeof is {} in Rust ({})\");",
            t.size, t.size, t.rust
        );
        let _ = writeln!(
            s,
            "_Static_assert(_Alignof({c}) == {}, \"{c}: _Alignof is {} in Rust\");",
            t.align, t.align
        );
        for f in &t.fields {
            let n = f.c_name;
            let _ = writeln!(
                s,
                "_Static_assert(offsetof({c}, {n}) == {}, \"{c}.{n}: offset is {} in Rust\");",
                f.offset, f.offset
            );
            let _ = writeln!(
                s,
                "_Static_assert(sizeof((({c} *)0)->{n}) == {}, \"{c}.{n}: size is {} in Rust\");",
                f.size, f.size
            );
        }
    }
    for a in asserts {
        let _ = writeln!(s, "_Static_assert({a}, \"{}\");", a.replace('"', "'"));
    }
    s
}

/// The `INCLUDES`, `DEFINES` and `-D` flags of `src/libgit2`'s objects in the build at
/// `dir`.
#[cfg(test)]
pub(crate) fn build_flags(dir: &std::path::Path) -> Vec<String> {
    let ninja = std::fs::read_to_string(dir.join("build.ninja"))
        .unwrap_or_else(|e| panic!("cannot read {}/build.ninja: {e}", dir.display()));
    let mut in_edge = false;
    let mut flags = Vec::new();
    for line in ninja.lines() {
        if line.starts_with("build src/libgit2/CMakeFiles/libgit2.dir/") && line.contains(".c.o:") {
            in_edge = true;
            continue;
        }
        if in_edge {
            let l = line.trim_start();
            if let Some(v) = l
                .strip_prefix("DEFINES = ")
                .or_else(|| l.strip_prefix("INCLUDES = "))
            {
                flags.extend(v.split_whitespace().map(String::from));
            } else if let Some(v) = l.strip_prefix("FLAGS = ") {
                flags.extend(
                    v.split_whitespace()
                        .filter(|f| f.starts_with("-D"))
                        .map(String::from),
                );
            } else if !line.starts_with(' ') {
                break;
            }
        }
    }
    assert!(
        !flags.is_empty(),
        "no compile flags for src/libgit2 found in {}/build.ninja",
        dir.display()
    );
    flags
}

#[cfg(test)]
/// Compile the assertions for `types` with clang against the build in
/// `LIBGIT2_BUILD_DIR`; panic with the compiler's output on a mismatch.
pub fn check(headers: &[&str], types: &[TypeLayout], asserts: &[&str]) {
    let dir = std::env::var_os("LIBGIT2_BUILD_DIR")
        .expect("LIBGIT2_BUILD_DIR must name a libgit2 CMake build directory");
    let dir = std::path::PathBuf::from(dir);
    let src = c_source(headers, types, asserts);
    let cc = std::env::var("CC").unwrap_or_else(|_| "clang".into());
    let mut cmd = std::process::Command::new(&cc);
    cmd.current_dir(&dir).args(build_flags(&dir)).args([
        "-std=gnu11",
        "-fsyntax-only",
        "-x",
        "c",
        "-",
    ]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {cc}: {e}"));
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(src.as_bytes()).expect("write C source");
    }
    let out = child.wait_with_output().expect("clang");
    assert!(
        out.status.success(),
        "layout mismatch:\n{}\n--- generated C ---\n{src}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(test)]
mod tests {
    #[repr(C)]
    struct Pair {
        a: u8,
        b: u32,
    }

    #[test]
    fn generates_assertions() {
        let t = crate::c_layout!(Pair => "struct pair" { a, b as "second" });
        assert_eq!(
            (t.size, t.align, t.fields[1].offset, t.fields[1].size),
            (8, 4, 4, 4)
        );
        let src = super::c_source(&["x.h"], &[t], &["1 == 1"]);
        assert!(src.contains("_Static_assert(offsetof(struct pair, second) == 4"));
        assert!(src.contains("#include \"x.h\""));
        assert!(src.contains("_Static_assert(1 == 1"));
    }

    #[test]
    #[cfg_attr(
        any(miri, not(feature = "link-c")),
        ignore = "runs clang on LIBGIT2_BUILD_DIR"
    )]
    fn detects_a_mismatch() {
        let t = crate::c_layout!(Pair => "git_str" { a as "ptr" });
        let r = std::panic::catch_unwind(|| super::check(&["str.h"], &[t], &[]));
        assert!(r.is_err());
    }
}
