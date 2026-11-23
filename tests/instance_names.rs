//! Instances are named after the function code of their library cell, not its whole name:
//! `AO21D0BWP7T40P140` is instantiated as `eco_AO21_u0`. Everything else in the report
//! still names the library cell in full.

mod common;

use std::collections::HashSet;

use common::{tcl_commands, words};
use opt_cells::{run_pipeline, RunInputs};

/// Every cell is named `<function code>D0BWP7T40P140`.
const WORKSPACE_LIB: &str = "workspace/lib/basic.toml";
const WORKSPACE_LIB_SUFFIX: &str = "D0BWP7T40P140";
/// Plain names (`NAND2`, `AND2`, ...) next to `AN3D0BWP7T40P140` and `INR4D0BWP7T40P140`.
const MIXED_LIB: &str = "tests/fixtures/libs/basic.toml";

const AO21_DSL: &str = "input a, b, c; output y; y = (a & b) | c;";

fn run(input_text: &str, library_path: &str, prefix: Option<&str>) -> String {
    run_pipeline(RunInputs {
        input_path: "inline.dsl".to_string(),
        input_text: input_text.to_string(),
        library_path: library_path.to_string(),
        prefix: prefix.map(str::to_string),
    })
    .expect("pipeline ok")
    .0
}

#[test]
fn instance_is_named_after_the_function_code_of_its_cell() {
    let report = run(AO21_DSL, WORKSPACE_LIB, None);

    let expected = "\
create_cell  eco_AO21_u0  [get_lib_cells */AO21D0BWP7T40P140]
connect_net  a            [get_pin eco_AO21_u0/A1]
connect_net  b            [get_pin eco_AO21_u0/A2]
connect_net  c            [get_pin eco_AO21_u0/B]
connect_net  y            [get_pin eco_AO21_u0/Z]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

#[test]
fn instances_of_the_same_cell_are_told_apart_by_their_uid() {
    let report = run(
        "input a, b, c, d; output y, z; y = a & b; z = c & d;",
        WORKSPACE_LIB,
        None,
    );

    let expected = "\
create_cell  eco_AN2_u0  [get_lib_cells */AN2D0BWP7T40P140]
create_cell  eco_AN2_u1  [get_lib_cells */AN2D0BWP7T40P140]
connect_net  a           [get_pin eco_AN2_u0/A1]
connect_net  b           [get_pin eco_AN2_u0/A2]
connect_net  y           [get_pin eco_AN2_u0/Z]
connect_net  c           [get_pin eco_AN2_u1/A1]
connect_net  d           [get_pin eco_AN2_u1/A2]
connect_net  z           [get_pin eco_AN2_u1/Z]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

/// The netlist and usage sections say which library cell was picked, so they keep the name
/// that `get_lib_cells` resolves.
#[test]
fn netlist_and_usage_sections_keep_the_full_cell_name() {
    let report = run(AO21_DSL, WORKSPACE_LIB, None);

    assert!(
        report.contains("  u0 : AO21D0BWP7T40P140  (A1=a, A2=b, B=c)  -> y\n"),
        "{report}"
    );
    assert!(report.contains("  AO21D0BWP7T40P140 x 1\n"), "{report}");
}

#[test]
fn custom_prefix_goes_in_front_of_the_function_code() {
    let report = run(AO21_DSL, WORKSPACE_LIB, Some("test"));

    let expected = "\
create_cell  eco_test_AO21_u0  [get_lib_cells */AO21D0BWP7T40P140]
connect_net  a                 [get_pin eco_test_AO21_u0/A1]
connect_net  b                 [get_pin eco_test_AO21_u0/A2]
connect_net  c                 [get_pin eco_test_AO21_u0/B]
connect_net  y                 [get_pin eco_test_AO21_u0/Z]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

/// A name without a drive-strength marker has nothing to drop.
#[test]
fn cell_without_a_drive_strength_marker_keeps_its_whole_name() {
    let report = run(
        "input a, b, c, d, e; output y, z; y = !(a & b); z = c & d & e;",
        MIXED_LIB,
        None,
    );

    let expected = "\
create_cell  eco_NAND2_u0  [get_lib_cells */NAND2]
create_cell  eco_AN3_u1    [get_lib_cells */AN3D0BWP7T40P140]
connect_net  a             [get_pin eco_NAND2_u0/a]
connect_net  b             [get_pin eco_NAND2_u0/b]
connect_net  y             [get_pin eco_NAND2_u0/y]
connect_net  c             [get_pin eco_AN3_u1/A1]
connect_net  d             [get_pin eco_AN3_u1/A2]
connect_net  e             [get_pin eco_AN3_u1/A3]
connect_net  z             [get_pin eco_AN3_u1/Z]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

/// The netlist section lists each mapped cell as `u<uid> : <CELL>`. The Tcl section must
/// create exactly those instances, each named after the function code and built from the
/// full cell name, and connect pins on those instances only.
#[test]
fn tcl_creates_and_connects_one_short_named_instance_per_mapped_cell() {
    let report = run(
        "input a, b, c, d, e, f, g, h, i, s;
         output y0, y1, y2, y3, y4, y5, y6, y7, y8, y9;
         y0 = a & b;
         y1 = !(b & c);
         y2 = a ^ b ^ c;
         y3 = (d & e) | f;
         y4 = (g | h) & i;
         y5 = s ? h : i;
         y6 = (a & b) | (c & d);
         y7 = !((e | f) & (g | h));
         y8 = a & b & c & d;
         y9 = a | b | c | d;",
        WORKSPACE_LIB,
        None,
    );

    let expected_creates: Vec<(String, String)> = report
        .lines()
        .filter(|line| line.contains(" -> "))
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (uid, cell) = (fields[0], fields[2]);
            let function_code = cell
                .strip_suffix(WORKSPACE_LIB_SUFFIX)
                .expect("workspace library cell name");
            (format!("eco_{function_code}_{uid}"), format!("*/{cell}]"))
        })
        .collect();
    assert!(expected_creates.len() >= 10, "{report}");

    let tcl = tcl_commands(&report);
    let creates: Vec<(String, String)> = tcl
        .iter()
        .filter(|command| command[0] == "create_cell")
        .map(|command| (command[1].to_string(), command[3].to_string()))
        .collect();
    assert_eq!(creates, expected_creates, "{report}");

    let created: HashSet<&str> = creates.iter().map(|(inst, _)| inst.as_str()).collect();
    assert_eq!(created.len(), creates.len(), "{report}");
    let connected: HashSet<&str> = tcl
        .iter()
        .filter(|command| command[0] == "connect_net")
        .map(|command| command[3].split_once('/').expect("instance/pin").0)
        .collect();
    assert_eq!(connected, created, "{report}");
}
