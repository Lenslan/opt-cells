use std::io::Write;

use opt_cells::{run_pipeline, RunInputs};

fn run_inline(input_text: &str, library_path: &str) -> String {
    run_pipeline(RunInputs {
        input_path: "inline.dsl".to_string(),
        input_text: input_text.to_string(),
        library_path: library_path.to_string(),
    })
    .expect("pipeline ok")
    .0
}

#[test]
fn ternary_expression_maps_to_mux_when_available() {
    let mut lib = tempfile::NamedTempFile::new().unwrap();
    write!(
        lib,
        r#"
[[cell]]
name = "MUX2"
inputs = ["I0", "I1", "S"]
output = "Z"
function = "S ? I1 : I0"
"#
    )
    .unwrap();

    let report = run_inline(
        "input sel, a, b; output y; y = sel ? a : b;",
        lib.path().to_str().unwrap(),
    );

    assert!(report.contains("Total cells used : 1"), "{report}");
    assert!(report.contains("MUX2"), "{report}");
}

#[test]
fn non_eco_intermediate_is_not_used_as_report_net_name() {
    let report = run_inline(
        "input a, b, c; output y; temp = a & b; y = temp | c;",
        "tests/fixtures/libs/basic.toml",
    );

    assert!(report.contains("Total cells used : 2"), "{report}");
    assert!(!report.contains("temp"), "{report}");
}

#[test]
fn eco_intermediate_replaces_internal_report_net_name() {
    let report = run_inline(
        "input a, b, c; output y; eco_ab = a & b; y = eco_ab | c;",
        "tests/fixtures/libs/basic.toml",
    );

    assert!(
        report.contains("  u1 : AND2  (a=a, b=b)  -> eco_ab"),
        "{report}"
    );
    assert!(report.contains("create_net      eco_ab"), "{report}");
    assert!(report.contains("connect_net     eco_ab"), "{report}");
}
