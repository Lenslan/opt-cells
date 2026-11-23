//! `--prefix <TAG>`: every cell and net the ECO script creates moves from the `eco_`
//! namespace into `eco_<TAG>_`. Ports and constants keep their names.

mod common;

use std::process::{Command, Output};

use common::{tcl_commands, words};
use opt_cells::error::OptCellsError;
use opt_cells::{run_pipeline, RunInputs};

const BASIC_LIB: &str = "tests/fixtures/libs/basic.toml";
const WORKSPACE_LIB: &str = "workspace/lib/basic.toml";

fn run(
    input_text: &str,
    library_path: &str,
    prefix: Option<&str>,
) -> Result<String, OptCellsError> {
    run_pipeline(RunInputs {
        input_path: "inline.dsl".to_string(),
        input_text: input_text.to_string(),
        library_path: library_path.to_string(),
        prefix: prefix.map(str::to_string),
    })
    .map(|(report, _)| report)
}

#[test]
fn prefix_test_names_every_created_cell_and_net_eco_test() {
    let report = run(
        "input a, b, c, d; output y; eco_ab = a & b; t = eco_ab | c; y = t & d;",
        BASIC_LIB,
        Some("test"),
    )
    .unwrap();

    let expected = "\
create_cell  eco_test_AND2_u0  [get_lib_cells */AND2]
create_cell  eco_test_OR2_u1   [get_lib_cells */OR2]
create_cell  eco_test_AND2_u2  [get_lib_cells */AND2]
create_net   eco_test_n6_u1
create_net   eco_test_ab
connect_net  d                 [get_pin eco_test_AND2_u0/a]
connect_net  eco_test_n6_u1    [get_pin eco_test_AND2_u0/b]
connect_net  y                 [get_pin eco_test_AND2_u0/y]
connect_net  c                 [get_pin eco_test_OR2_u1/a]
connect_net  eco_test_ab       [get_pin eco_test_OR2_u1/b]
connect_net  eco_test_n6_u1    [get_pin eco_test_OR2_u1/y]
connect_net  a                 [get_pin eco_test_AND2_u2/a]
connect_net  b                 [get_pin eco_test_AND2_u2/b]
connect_net  eco_test_ab       [get_pin eco_test_AND2_u2/y]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

#[test]
fn netlist_section_shows_aliases_under_the_custom_prefix() {
    let report = run(
        "input a, b, c, d; output y; eco_ab = a & b; t = eco_ab | c; y = t & d;",
        BASIC_LIB,
        Some("test"),
    )
    .unwrap();

    let netlist: Vec<&str> = report
        .lines()
        .filter(|line| line.contains(" -> "))
        .collect();
    assert_eq!(
        netlist,
        [
            "  u0 : AND2  (a=d, b=n6_u1)  -> y",
            "  u1 : OR2  (a=c, b=eco_test_ab)  -> n6",
            "  u2 : AND2  (a=a, b=b)  -> eco_test_ab",
        ],
        "{report}"
    );
}

#[test]
fn each_bit_of_a_vector_alias_is_prefixed() {
    let report = run(
        "input a[1:0], b[1:0]; output y; eco_s = a ^ b; y = eco_s[0] & eco_s[1];",
        WORKSPACE_LIB,
        Some("test"),
    )
    .unwrap();

    let tcl = tcl_commands(&report);
    assert!(
        tcl.contains(&vec!["create_net", "eco_test_s[0]"]),
        "{report}"
    );
    assert!(
        tcl.contains(&vec!["create_net", "eco_test_s[1]"]),
        "{report}"
    );
    assert!(
        report.contains("(A1=eco_test_s[0], A2=eco_test_s[1])  -> y"),
        "{report}"
    );
}

#[test]
fn ports_that_start_with_eco_keep_their_names() {
    let report = run(
        "input eco_in, b, c; output eco_out; eco_mid = eco_in & b; eco_out = eco_mid | c;",
        BASIC_LIB,
        Some("test"),
    )
    .unwrap();

    let expected = "\
create_cell  eco_test_OR2_u0   [get_lib_cells */OR2]
create_cell  eco_test_AND2_u1  [get_lib_cells */AND2]
create_net   eco_test_mid
connect_net  c                 [get_pin eco_test_OR2_u0/a]
connect_net  eco_test_mid      [get_pin eco_test_OR2_u0/b]
connect_net  eco_out           [get_pin eco_test_OR2_u0/y]
connect_net  eco_in            [get_pin eco_test_AND2_u1/a]
connect_net  b                 [get_pin eco_test_AND2_u1/b]
connect_net  eco_test_mid      [get_pin eco_test_AND2_u1/y]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
    assert!(
        report.contains("  u0 : OR2  (a=c, b=eco_test_mid)  -> eco_out\n"),
        "{report}"
    );
    assert!(
        report.contains("  u1 : AND2  (a=eco_in, b=b)  -> eco_test_mid\n"),
        "{report}"
    );
}

/// Ports are the script's interface to nets that already exist, possibly made by another
/// ECO step under another prefix, so an `eco_` in a declared name is not a namespace marker.
#[test]
fn vector_ports_that_start_with_eco_keep_their_names() {
    let report = run(
        "input eco_clr, d[1:0]; output eco_q[1:0]; eco_q = eco_clr ? 0 : d;",
        WORKSPACE_LIB,
        Some("test"),
    )
    .unwrap();

    let expected = "\
create_cell  eco_test_INR2_u0  [get_lib_cells */INR2D0BWP7T40P140]
create_cell  eco_test_INR2_u1  [get_lib_cells */INR2D0BWP7T40P140]
connect_net  d[0]              [get_pin eco_test_INR2_u0/A1]
connect_net  eco_clr           [get_pin eco_test_INR2_u0/B1]
connect_net  eco_q[0]          [get_pin eco_test_INR2_u0/ZN]
connect_net  d[1]              [get_pin eco_test_INR2_u1/A1]
connect_net  eco_clr           [get_pin eco_test_INR2_u1/B1]
connect_net  eco_q[1]          [get_pin eco_test_INR2_u1/ZN]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
    assert!(
        report.contains("(A1=d[1], B1=eco_clr)  -> eco_q[1]\n"),
        "{report}"
    );
}

/// The report for a custom prefix must be the default report with every created name
/// moved from `eco_` to `eco_<TAG>_` and nothing else changed. Only meaningful for
/// inputs whose ports do not contain `eco_`.
fn assert_only_created_names_move(case: &str, input_text: &str, library_path: &str) {
    let default = run(input_text, library_path, None).unwrap();
    let tagged = run(input_text, library_path, Some("Fix_2")).unwrap();

    assert!(
        default.contains("eco_"),
        "{case}: creates nothing to rename\n{default}"
    );
    assert!(
        !tagged.replace("eco_Fix_2_", "").contains("eco_"),
        "{case}: a name was left in the bare eco_ namespace\n{tagged}"
    );
    assert_eq!(
        words(&tagged.replace("eco_Fix_2_", "eco_")),
        words(&default),
        "{case}: the prefix changed more than names\n{tagged}"
    );
}

#[test]
fn fixture_reports_differ_from_the_default_only_in_created_names() {
    for (input, library) in [
        ("nand.dsl", "basic.toml"),
        ("nand.dsl", "and2_only.toml"),
        ("decode.dsl", "with_and4.toml"),
        ("shared.dsl", "basic.toml"),
        ("mux.dsl", "basic.toml"),
        ("andb.dsl", "with_andb.toml"),
        ("STATE_1.dsl", "state_test.toml"),
        ("STATE_2.dsl", "state_test.toml"),
        ("STATE_3.dsl", "state_test.toml"),
    ] {
        let input_text = std::fs::read_to_string(format!("tests/fixtures/inputs/{input}")).unwrap();
        assert_only_created_names_move(
            &format!("{input} x {library}"),
            &input_text,
            &format!("tests/fixtures/libs/{library}"),
        );
    }
}

#[test]
fn larger_netlists_differ_from_the_default_only_in_created_names() {
    for (case, input_text, library) in [
        (
            "multi-output logic",
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
        ),
        (
            "arithmetic with vector ports",
            "input cnt[3:0], bitmap[3:0]; output hit, nxt[3:0];
             hit = bitmap[cnt[1:0]] & (cnt <= 9);
             nxt = cnt + 1;",
            WORKSPACE_LIB,
        ),
        (
            "vector alias over 2-input cells",
            "input a[1:0], b[1:0]; output y; eco_s = a ^ b; y = eco_s[0] & eco_s[1];",
            BASIC_LIB,
        ),
        (
            "alias on an inverted input",
            "input a, b; output y, z; eco_n = !a; y = eco_n & b; z = eco_n | b;",
            BASIC_LIB,
        ),
    ] {
        assert_only_created_names_move(case, input_text, library);
    }
}

#[test]
fn invalid_prefix_is_an_error_instead_of_a_report() {
    for bad in ["", "a b", "a/b", "x[0]"] {
        match run("input a, b; output y; y = !(a & b);", BASIC_LIB, Some(bad)) {
            Err(OptCellsError::InvalidPrefix(got)) => assert_eq!(got, bad),
            other => panic!("prefix {bad:?} must be rejected, got {other:?}"),
        }
    }
}

// ------------------------------------------------------------------- real binary

const NAND_DSL: &str = "tests/fixtures/inputs/nand.dsl";

fn opt_cells(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_opt-cells"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("run opt-cells")
}

fn stdout_of(out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn cli_prefix_flag_renames_created_names() {
    let report = stdout_of(&opt_cells(&[NAND_DSL, "-l", BASIC_LIB, "--prefix", "test"]));

    let expected = "\
create_cell  eco_test_NAND2_u0  [get_lib_cells */NAND2]
connect_net  a                  [get_pin eco_test_NAND2_u0/a]
connect_net  b                  [get_pin eco_test_NAND2_u0/b]
connect_net  y                  [get_pin eco_test_NAND2_u0/y]
";
    assert_eq!(tcl_commands(&report), words(expected), "{report}");
}

#[test]
fn cli_short_flag_is_the_same_option() {
    let long = stdout_of(&opt_cells(&[NAND_DSL, "-l", BASIC_LIB, "--prefix", "test"]));
    let short = stdout_of(&opt_cells(&[NAND_DSL, "-l", BASIC_LIB, "-p", "test"]));

    assert_eq!(short, long);
}

#[test]
fn cli_quiet_summary_is_unaffected_by_the_prefix() {
    let summary = stdout_of(&opt_cells(&[
        NAND_DSL, "-l", BASIC_LIB, "--quiet", "--prefix", "test",
    ]));

    assert_eq!(summary, "total cells: 1\n");
}

#[test]
fn cli_rejects_an_invalid_prefix_with_exit_code_1() {
    let out = opt_cells(&[NAND_DSL, "-l", BASIC_LIB, "--prefix", "a/b"]);

    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid prefix 'a/b'"), "{stderr}");
}
