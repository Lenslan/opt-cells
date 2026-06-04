mod common;

use opt_cells::{run_pipeline, RunInputs};

fn run(input_rel: &str, lib_rel: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let input_path = format!("tests/fixtures/{}", input_rel);
    let library_path = format!("tests/fixtures/{}", lib_rel);
    let input_text =
        std::fs::read_to_string(format!("{}/tests/fixtures/{}", manifest, input_rel)).unwrap();
    run_pipeline(RunInputs {
        input_path,
        input_text,
        library_path,
    })
    .expect("pipeline ok")
    .0
}

#[test]
fn nand_uses_native_nand2_not_free_inverters() {
    let s = run("inputs/nand.dsl", "libs/basic.toml");
    assert!(
        s.contains("NAND2"),
        "should use the native NAND2 cell:\n{}",
        s
    );
    assert!(
        !s.contains("OR2"),
        "must not emulate with OR2 + inverters:\n{}",
        s
    );
    assert!(
        !s.contains("=!"),
        "no free pin inversion (pin=!signal) allowed:\n{}",
        s
    );
    assert!(
        s.contains("Total cells used : 1"),
        "still a single cell:\n{}",
        s
    );
}

#[test]
fn decode_counts_the_inverter() {
    let s = run("inputs/decode.dsl", "libs/with_and4.toml");
    assert!(
        s.contains("Total cells used : 7"),
        "decode mapping should count all required inverters:\n{}",
        s
    );
    assert!(s.contains("AND4"), "{}", s);
    assert!(
        s.contains("INV"),
        "the state[3] inversion must be a real INV cell:\n{}",
        s
    );
    assert!(!s.contains("=!"), "no free pin inversion:\n{}", s);
}

#[test]
fn mux_counts_the_inverter() {
    let s = run("inputs/mux.dsl", "libs/basic.toml");
    assert!(
        s.contains("Total cells used : 4"),
        "adds a real INV for !s:\n{}",
        s
    );
    assert!(s.contains("INV"), "{}", s);
    assert!(!s.contains("=!"), "no free pin inversion:\n{}", s);
}

#[test]
fn builtin_inverted_input_is_free() {
    let s = run("inputs/andb.dsl", "libs/with_andb.toml");
    assert!(s.contains("AND2B1"), "{}", s);
    assert!(
        s.contains("Total cells used : 1"),
        "built-in inversion is free:\n{}",
        s
    );
    assert!(!s.contains("INV"), "no separate inverter needed:\n{}", s);
    assert!(!s.contains("=!"), "{}", s);
}

#[test]
fn state_variants_use_builtin_inverted_pins_without_extra_inverters() {
    for input in [
        "inputs/STATE_1.dsl",
        "inputs/STATE_2.dsl",
        "inputs/STATE_3.dsl",
    ] {
        let s = run(input, "libs/state_test.toml");
        assert!(
            s.contains("Total cells used : 2"),
            "{} should map to AN3 + INR4 only:\n{}",
            input,
            s
        );
        assert!(s.contains("AN3D0BWP7T40P140"), "{}", s);
        assert!(s.contains("INR4D0BWP7T40P140"), "{}", s);
        assert!(
            !s.contains(" : INV"),
            "{} should not need extra input inverters:\n{}",
            input,
            s
        );
    }
}

#[test]
fn state_1_report_contains_tcl_script() {
    let s = run("inputs/STATE_1.dsl", "libs/state_test.toml");
    assert!(s.contains("  Tcl script"), "{}", s);
    assert!(s.contains("create_cell"), "{}", s);
    assert!(s.contains("eco_AN3D0BWP7T40P140_u0"), "{}", s);
    assert!(s.contains("[get_lib_cells */AN3D0BWP7T40P140]"), "{}", s);
    assert!(s.contains("eco_INR4D0BWP7T40P140_u1"), "{}", s);
    assert!(s.contains("[get_lib_cells */INR4D0BWP7T40P140]"), "{}", s);
    assert!(s.contains("create_net      eco_n9_u1"), "{}", s);
    assert!(s.contains("[get_pin eco_AN3D0BWP7T40P140_u0/Z]"), "{}", s);
    assert!(
        !s.contains("[get_pin eco_AN3D0BWP7T40P140/A1]"),
        "pin connections must use the created instance name:\n{}",
        s
    );
}

#[test]
fn no_inv_cell_and_inversion_needed_is_an_error() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let input_text =
        std::fs::read_to_string(format!("{}/tests/fixtures/inputs/nand.dsl", manifest)).unwrap();
    let res = run_pipeline(RunInputs {
        input_path: "tests/fixtures/inputs/nand.dsl".to_string(),
        input_text,
        library_path: "tests/fixtures/libs/no_inv.toml".to_string(),
    });
    assert!(
        res.is_err(),
        "missing INV + required inversion should be a mapping error"
    );
}
