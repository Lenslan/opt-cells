//! Arithmetic, relational and dynamic-index DSL features.
//!
//! Every positive case is checked exhaustively twice: the elaborated AIG against an
//! integer reference model, and the mapped netlist against the same model.

mod common;

use std::collections::HashMap;

use opt_cells::frontend::elaborate::elaborate;
use opt_cells::frontend::library::load_library;
use opt_cells::frontend::parser::parse_program;
use opt_cells::mapper::map_aig;
use opt_cells::{run_pipeline, RunInputs};

/// `(name, width)`; width 0 means a scalar signal.
type Port = (&'static str, u32);

fn lib_path() -> String {
    format!("{}/workspace/lib/basic.toml", env!("CARGO_MANIFEST_DIR"))
}

fn bit_names(ports: &[Port]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, w) in ports {
        if *w == 0 {
            out.push(name.to_string());
        } else {
            // `v@4` names a vector whose least significant bit has index 4.
            let (base, lo) = match name.split_once('@') {
                Some((base, lo)) => (base, lo.parse::<u32>().expect("low bound")),
                None => (*name, 0),
            };
            for i in lo..lo + *w {
                out.push(format!("{}__{}", base, i));
            }
        }
    }
    out
}

fn collect(values: &HashMap<String, bool>, port: &Port) -> u64 {
    let (name, w) = port;
    let get = |key: &str| -> u64 {
        *values
            .get(key)
            .unwrap_or_else(|| panic!("output bit '{key}' missing")) as u64
    };
    if *w == 0 {
        get(name)
    } else {
        (0..*w).map(|i| get(&format!("{}__{}", name, i)) << i).sum()
    }
}

fn check(src: &str, ins: &[Port], outs: &[Port], reference: impl Fn(&[u64]) -> Vec<u64>) {
    let prog = parse_program(src, "t.dsl").unwrap_or_else(|e| panic!("parse failed: {e}\n{src}"));
    let elab =
        elaborate(&prog, "t.dsl", src).unwrap_or_else(|e| panic!("elaborate failed: {e}\n{src}"));
    let lib = load_library(&lib_path()).expect("load library");
    let netlist = map_aig(&elab.aig, &lib).expect("map");

    let in_bits = bit_names(ins);
    assert!(in_bits.len() <= 12, "exhaustive simulation is bounded");
    for pattern in 0..(1u64 << in_bits.len()) {
        let mut assignment = HashMap::new();
        for (i, name) in in_bits.iter().enumerate() {
            assignment.insert(name.clone(), (pattern >> i) & 1 == 1);
        }
        let mut vals = Vec::new();
        let mut pos = 0;
        for (_, w) in ins {
            let nb = (*w).max(1);
            vals.push((pattern >> pos) & ((1u64 << nb) - 1));
            pos += nb;
        }
        let expected = reference(&vals);
        assert_eq!(expected.len(), outs.len());
        let aig_out = common::simulate_aig(&elab.aig, &assignment);
        let net_out = common::simulate_netlist(&elab.aig, &lib, &netlist, &assignment);
        for (port, exp) in outs.iter().zip(&expected) {
            assert_eq!(
                collect(&aig_out, port),
                *exp,
                "AIG output {} wrong for inputs {vals:?}\n{src}",
                port.0
            );
            assert_eq!(
                collect(&net_out, port),
                *exp,
                "mapped netlist output {} wrong for inputs {vals:?}\n{src}",
                port.0
            );
        }
    }
}

fn elab_err(src: &str) -> String {
    let prog = parse_program(src, "t.dsl").unwrap_or_else(|e| panic!("parse failed: {e}\n{src}"));
    match elaborate(&prog, "t.dsl", src) {
        Ok(_) => panic!("expected an elaboration error:\n{src}"),
        Err(e) => e.to_string(),
    }
}

const AB4: &[Port] = &[("a", 4), ("b", 4)];

// ---------------------------------------------------------------- arithmetic

#[test]
fn add_keeps_carry_when_target_is_wider() {
    check(
        "input a[3:0], b[3:0]; output s[4:0]; s = a + b;",
        AB4,
        &[("s", 5)],
        |v| vec![v[0] + v[1]],
    );
}

#[test]
fn add_wraps_at_target_width() {
    check(
        "input a[3:0], b[3:0]; output s[3:0]; s = a + b;",
        AB4,
        &[("s", 4)],
        |v| vec![(v[0] + v[1]) & 15],
    );
}

#[test]
fn sub_wraps_modulo_width() {
    check(
        "input a[3:0], b[3:0]; output d[3:0]; d = a - b;",
        AB4,
        &[("d", 4)],
        |v| vec![v[0].wrapping_sub(v[1]) & 15],
    );
}

#[test]
fn mul_full_product() {
    check(
        "input a[3:0], b[3:0]; output p[7:0]; p = a * b;",
        AB4,
        &[("p", 8)],
        |v| vec![v[0] * v[1]],
    );
}

#[test]
fn mul_truncates_to_target_width() {
    check(
        "input a[3:0], b[3:0]; output p[3:0]; p = a * b;",
        AB4,
        &[("p", 4)],
        |v| vec![(v[0] * v[1]) & 15],
    );
}

#[test]
fn add_constant_and_mixed_precedence() {
    check(
        "input a[2:0], b[2:0], c[2:0]; output y[6:0]; y = a + b * c - 1;",
        &[("a", 3), ("b", 3), ("c", 3)],
        &[("y", 7)],
        |v| vec![(v[0] + v[1] * v[2]).wrapping_sub(1) & 127],
    );
}

// ---------------------------------------------------------------- relational

#[test]
fn relational_operators_are_unsigned() {
    check(
        "input a[3:0], b[3:0]; output lt, le, gt, ge;
         lt = a < b; le = a <= b; gt = a > b; ge = a >= b;",
        AB4,
        &[("lt", 0), ("le", 0), ("gt", 0), ("ge", 0)],
        |v| {
            let (a, b) = (v[0], v[1]);
            vec![
                (a < b) as u64,
                (a <= b) as u64,
                (a > b) as u64,
                (a >= b) as u64,
            ]
        },
    );
}

#[test]
fn compare_against_unsized_constants() {
    check(
        "input a[3:0]; output y0, y1, y2; y0 = a < 10; y1 = a >= 3; y2 = 5 > a;",
        &[("a", 4)],
        &[("y0", 0), ("y1", 0), ("y2", 0)],
        |v| vec![(v[0] < 10) as u64, (v[0] >= 3) as u64, (5 > v[0]) as u64],
    );
}

#[test]
fn unsized_constant_is_not_truncated_to_operand_width() {
    check(
        "input a[2:0]; output y; y = a < 10;",
        &[("a", 3)],
        &[("y", 0)],
        |_| vec![1],
    );
}

#[test]
fn sized_sum_wraps_before_compare_but_unsized_sum_does_not() {
    check(
        "input a[3:0], b[3:0], c[3:0]; output w, n;
         w = (a + b) < c;
         n = (a + 1) < c;",
        &[("a", 4), ("b", 4), ("c", 4)],
        &[("w", 0), ("n", 0)],
        |v| {
            vec![
                (((v[0] + v[1]) & 15) < v[2]) as u64,
                ((v[0] + 1) < v[2]) as u64,
            ]
        },
    );
}

#[test]
fn relational_binds_tighter_than_equality_and_bitwise() {
    check(
        "input a[1:0], b[1:0], c, d; output y0, y1;
         y0 = a + 1 < b & c;
         y1 = a < b == d;",
        &[("a", 2), ("b", 2), ("c", 0), ("d", 0)],
        &[("y0", 0), ("y1", 0)],
        |v| {
            vec![
                ((v[0] + 1 < v[1]) as u64) & v[2],
                (((v[0] < v[1]) as u64) == v[3]) as u64,
            ]
        },
    );
}

// ---------------------------------------------------------------- indexing

#[test]
fn dynamic_index_selects_bit() {
    check(
        "input bitmap[7:0], idx[2:0]; output y; y = bitmap[idx];",
        &[("bitmap", 8), ("idx", 3)],
        &[("y", 0)],
        |v| vec![(v[0] >> v[1]) & 1],
    );
}

#[test]
fn dynamic_index_out_of_range_reads_zero() {
    check(
        "input bitmap[5:0], idx[3:0]; output y; y = bitmap[idx];",
        &[("bitmap", 6), ("idx", 4)],
        &[("y", 0)],
        |v| vec![if v[1] < 6 { (v[0] >> v[1]) & 1 } else { 0 }],
    );
}

#[test]
fn dynamic_index_respects_declared_low_bound() {
    // v[7:4]: index 4 is the least significant bit.
    check(
        "input v[7:4], idx[2:0]; output y; y = v[idx];",
        &[("v@4", 4), ("idx", 3)],
        &[("y", 0)],
        |v| {
            vec![if v[1] >= 4 {
                (v[0] >> (v[1] - 4)) & 1
            } else {
                0
            }]
        },
    );
}

#[test]
fn dynamic_index_accepts_arbitrary_expression() {
    check(
        "input bitmap[7:0], i[2:0]; output y; y = bitmap[i + 1] & !bitmap[i];",
        &[("bitmap", 8), ("i", 3)],
        &[("y", 0)],
        |v| {
            let hi = if v[1] + 1 < 8 {
                (v[0] >> (v[1] + 1)) & 1
            } else {
                0
            };
            vec![hi & !(v[0] >> v[1]) & 1]
        },
    );
}

// ---------------------------------------------------------------- vectors as values

#[test]
fn multi_bit_intermediate_can_be_sliced() {
    check(
        "input a[3:0], b[3:0]; output y, z; t = a + b; y = t[3]; z = t[2:0] == 3'd5;",
        AB4,
        &[("y", 0), ("z", 0)],
        |v| {
            let t = (v[0] + v[1]) & 15;
            vec![t >> 3, ((t & 7) == 5) as u64]
        },
    );
}

#[test]
fn slice_assignment_fills_parts_of_an_output() {
    check(
        "input a[3:0], b[3:0]; output y[3:0];
         y[1:0] = a[1:0] + b[1:0];
         y[3:2] = a[3:2] & b[3:2];",
        AB4,
        &[("y", 4)],
        |v| {
            let lo = ((v[0] & 3) + (v[1] & 3)) & 3;
            let hi = (v[0] >> 2) & (v[1] >> 2);
            vec![lo | (hi << 2)]
        },
    );
}

#[test]
fn bitwise_not_and_logical_not_differ_on_vectors() {
    check(
        "input a[3:0]; output n[3:0], z; n = ~a; z = !a;",
        &[("a", 4)],
        &[("n", 4), ("z", 0)],
        |v| vec![!v[0] & 15, (v[0] == 0) as u64],
    );
}

#[test]
fn multi_bit_mux_and_bitwise_ops() {
    check(
        "input s, a[2:0], b[2:0]; output y[2:0], x[2:0]; y = s ? a : b; x = (a ^ b) | 3'b100;",
        &[("s", 0), ("a", 3), ("b", 3)],
        &[("y", 3), ("x", 3)],
        |v| vec![if v[0] == 1 { v[1] } else { v[2] }, (v[1] ^ v[2]) | 4],
    );
}

#[test]
fn narrower_value_is_zero_extended_into_wider_output() {
    check(
        "input a[1:0]; output y[3:0]; y = a;",
        &[("a", 2)],
        &[("y", 4)],
        |v| vec![v[0]],
    );
}

#[test]
fn constant_arm_does_not_widen_scalar_intermediate() {
    // Pre-existing idiom: `t` must stay one bit wide.
    check(
        "input s, q, c, d; output y; t = s ? 1 : q; y = c ? d : t;",
        &[("s", 0), ("q", 0), ("c", 0), ("d", 0)],
        &[("y", 0)],
        |v| vec![if v[2] == 1 { v[3] } else { v[0] | v[1] }],
    );
}

// ---------------------------------------------------------------- errors

#[test]
fn relational_width_mismatch_is_rejected() {
    let msg = elab_err("input a[3:0], b[4:0]; output y; y = a < b;");
    assert!(msg.contains("width mismatch"), "{msg}");
}

#[test]
fn assigning_wider_value_to_narrower_target_is_rejected() {
    let msg = elab_err("input a[3:0]; output y; y = a;");
    assert!(msg.contains("width mismatch"), "{msg}");
}

#[test]
fn duplicate_output_assignment_is_rejected() {
    let msg = elab_err("input a, b; output y[1:0]; y = 2'b00; y[1] = a & b;");
    assert!(msg.contains("duplicate assignment"), "{msg}");
}

#[test]
fn assigning_to_an_input_is_rejected() {
    let msg = elab_err("input a, b; output y; a = b; y = a;");
    assert!(msg.contains("input"), "{msg}");
}

#[test]
fn dynamic_index_on_scalar_is_rejected() {
    let msg = elab_err("input a, i[1:0]; output y; y = a[i];");
    assert!(msg.contains("single-bit"), "{msg}");
}

#[test]
fn unsized_literal_wider_than_32_bits_is_a_parse_error() {
    assert!(parse_program("input a; output y; y = a & 4294967296;", "t.dsl").is_err());
}

// ---------------------------------------------------------------- end to end

#[test]
fn pipeline_reports_cells_and_tcl_for_arithmetic() {
    let (report, _) = run_pipeline(RunInputs {
        input_path: "inline.dsl".into(),
        input_text: "input cnt[3:0], bitmap[3:0]; output hit, nxt[3:0];
                     hit = bitmap[cnt[1:0]] & (cnt <= 9);
                     nxt = cnt + 1;"
            .into(),
        library_path: lib_path(),
        prefix: None,
    })
    .expect("pipeline ok");
    assert!(report.contains("-> hit"), "{report}");
    assert!(report.contains("nxt[3]"), "{report}");
    assert!(report.contains("create_cell"), "{report}");
    assert!(report.contains("connect_net"), "{report}");
}
