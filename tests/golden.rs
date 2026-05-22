mod common;

use opt_cells::{run_pipeline, RunInputs};

fn read_fixture(rel: &str) -> String {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn run(input_rel: &str, library_rel: &str) -> String {
    let input_path = format!("tests/fixtures/{}", input_rel);
    let library_path = format!("tests/fixtures/{}", library_rel);
    let input_text = read_fixture(input_rel);
    let (report, _) = run_pipeline(RunInputs {
        input_path,
        input_text,
        library_path,
    })
    .expect("pipeline ok");
    report
}

fn assert_golden(actual: &str, expected_rel: &str) {
    let path = format!(
        "{}/tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        expected_rel
    );
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; run with UPDATE_GOLDEN=1 to create",
            path
        )
    });
    if actual != expected {
        eprintln!("--- expected ---\n{}", expected);
        eprintln!("--- actual ---\n{}", actual);
        panic!("golden mismatch: {}", expected_rel);
    }
}

#[test]
fn nand_basic() {
    let s = run("inputs/nand.dsl", "libs/basic.toml");
    assert_golden(&s, "expected/nand_basic.txt");
}

#[test]
fn decode_and4() {
    let s = run("inputs/decode.dsl", "libs/with_and4.toml");
    assert_golden(&s, "expected/decode_and4.txt");
}

#[test]
fn nand_with_and2_inv_only() {
    let s = run("inputs/nand.dsl", "libs/and2_only.toml");
    assert_golden(&s, "expected/nand_and2_only.txt");
}

#[test]
fn shared_subexpression() {
    let s = run("inputs/shared.dsl", "libs/basic.toml");
    assert_golden(&s, "expected/shared_basic.txt");
}

#[test]
fn mux_basic() {
    let s = run("inputs/mux.dsl", "libs/basic.toml");
    assert_golden(&s, "expected/mux_basic.txt");
}
