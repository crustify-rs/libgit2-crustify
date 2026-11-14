//! The `ffi_export` gateways of one source file, as the build script reads them. Kept apart
//! from `build.rs` so that `tests/unit_pub/build_script.rs` checks it.

/// Each `ffi_export` gateway of `text`: its export feature (as a `CARGO_FEATURE_` suffix)
/// when it has one, and its lines.
///
/// A gateway's feature is the `#[cfg(feature = "ffi-export-…")]` among the attributes of its
/// `mod`, wherever it stands among them: an attribute that spans lines, such as the region's
/// `#[expect(…)]`, neither sets nor resets it.
pub(crate) fn gateways_of(text: &str) -> Vec<(Option<String>, Vec<String>)> {
    let count = |s: &str, open: char, close: char| {
        s.matches(open).count().cast_signed() - s.matches(close).count().cast_signed()
    };
    let mut gateways = Vec::new();
    let mut gate: Option<String> = None;
    let mut open: Option<(Option<String>, isize, Vec<String>)> = None;
    // The brackets a multi-line attribute still has open.
    let mut attribute: isize = 0;
    for line in text.lines() {
        let t = line.trim();
        if let Some((_, depth, lines)) = open.as_mut() {
            lines.push(t.to_string());
            *depth += count(t, '{', '}');
            if *depth <= 0 {
                let (g, _, lines) = open.take().unwrap();
                gateways.push((g, lines));
            }
        } else if attribute > 0 {
            attribute += count(t, '[', ']');
        } else if let Some(feature) = t
            .strip_prefix("#[cfg(feature = \"ffi-export-")
            .and_then(|r| r.strip_suffix("\")]"))
        {
            gate = Some(feature.to_uppercase().replace('-', "_"));
        } else if t.starts_with("mod ffi_export") || t.starts_with("pub mod ffi_export") {
            open = Some((gate.take(), count(t, '{', '}'), Vec::new()));
        } else if t.starts_with("#[") {
            attribute = count(t, '[', ']');
        } else if !t.starts_with("//") && !t.is_empty() {
            gate = None;
        }
    }
    gateways
}
