//! The build script's reading of the `ffi_export` gateways (`build/gateways.rs`), which
//! decides what a `system` build refuses and what the binaries export dynamically.

#[path = "../../build/gateways.rs"]
mod gateways;

use gateways::gateways_of;

const FEATURE: &str = "#[cfg(feature = \"ffi-export-deps-llhttp-http-c\")]";
const EXPECT: &str = "#[expect(\n    unsafe_code,\n    reason = \"x\"\n)]";

fn features(text: &str) -> Vec<Option<String>> {
    gateways_of(text)
        .into_iter()
        .map(|(gate, _)| gate)
        .collect()
}

#[test]
fn a_gateways_feature_is_read_before_a_multi_line_attribute() {
    let text = format!("{FEATURE}\n{EXPECT}\nmod ffi_export {{\n}}\n");
    assert_eq!(features(&text), [Some("DEPS_LLHTTP_HTTP_C".to_string())]);
}

#[test]
fn a_gateways_feature_is_read_after_a_multi_line_attribute() {
    let text = format!("{EXPECT}\n{FEATURE}\nmod ffi_export {{\n}}\n");
    assert_eq!(features(&text), [Some("DEPS_LLHTTP_HTTP_C".to_string())]);
}

#[test]
fn a_gateway_without_a_feature_has_none() {
    let text = format!("{EXPECT}\nmod ffi_export {{\n}}\n");
    assert_eq!(features(&text), [None]);
}

#[test]
fn another_items_feature_does_not_reach_a_later_gateway() {
    let text = format!("{FEATURE}\nfn other() {{}}\n\n{EXPECT}\npub mod ffi_export {{\n}}\n");
    assert_eq!(features(&text), [None]);
}

#[test]
fn a_gateways_lines_end_with_its_closing_brace() {
    let text = "mod ffi_export {\n    fn a() {\n    }\n}\nfn after() {}\n";
    let gateways = gateways_of(text);
    assert_eq!(gateways.len(), 1);
    assert_eq!(gateways[0].1, ["fn a() {", "}", "}"]);
}
