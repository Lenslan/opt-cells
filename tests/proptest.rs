use proptest::prelude::*;

use opt_cells::aig::Aig;
use opt_cells::match_npn::npn_canonical;

mod common;

proptest! {
    #[test]
    fn npn_canonical_is_idempotent(tt in any::<u64>(), k in 2u32..=4) {
        let info1 = npn_canonical(tt, k);
        let info2 = npn_canonical(info1.canonical_tt, k);
        prop_assert_eq!(info1.canonical_tt, info2.canonical_tt);
    }

    #[test]
    fn random_aig_maps_to_functionally_equivalent_netlist(
        seed in any::<u64>(),
    ) {
        // Build a small random AIG with up to 3 inputs and 3-5 AND2 nodes.
        let mut rng_state = seed | 1;
        fn next(rs: &mut u64) -> u64 { *rs = rs.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); *rs }

        let mut aig = Aig::new();
        let mut nodes: Vec<opt_cells::aig::Edge> = Vec::new();
        let pi_names = ["a".to_string(), "b".to_string(), "c".to_string()];
        for name in &pi_names { nodes.push(aig.add_input(name)); }

        let n_and = 3 + (next(&mut rng_state) % 3) as usize;
        for _ in 0..n_and {
            let i = (next(&mut rng_state) as usize) % nodes.len();
            let j = (next(&mut rng_state) as usize) % nodes.len();
            let li = next(&mut rng_state) & 1 == 1;
            let lj = next(&mut rng_state) & 1 == 1;
            let a = if li { nodes[i].inv() } else { nodes[i] };
            let b = if lj { nodes[j].inv() } else { nodes[j] };
            let ab = aig.and(a, b);
            nodes.push(ab);
        }
        let out_idx = (next(&mut rng_state) as usize) % nodes.len();
        let out_inv = next(&mut rng_state) & 1 == 1;
        let out_edge = if out_inv { nodes[out_idx].inv() } else { nodes[out_idx] };
        aig.add_output("y", out_edge);

        let toml = r#"
[[cell]]
name = "INV"
inputs = ["a"]
output = "y"
function = "!a"

[[cell]]
name = "AND2"
inputs = ["a", "b"]
output = "y"
function = "a & b"

[[cell]]
name = "NAND2"
inputs = ["a", "b"]
output = "y"
function = "!(a & b)"

[[cell]]
name = "OR2"
inputs = ["a", "b"]
output = "y"
function = "a | b"

[[cell]]
name = "NOR2"
inputs = ["a", "b"]
output = "y"
function = "!(a | b)"

[[cell]]
name = "XOR2"
inputs = ["a", "b"]
output = "y"
function = "a ^ b"
"#;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        use std::io::Write;
        tmp.write_all(toml.as_bytes()).unwrap();

        let lib = opt_cells::frontend::library::load_library(tmp.path().to_str().unwrap()).unwrap();
        let netlist = opt_cells::mapper::map_aig(&aig, &lib).unwrap();

        let pi_names_vec: Vec<String> = pi_names.to_vec();
        for assignment in common::enumerate_input_assignments(&pi_names_vec) {
            let aig_out = common::simulate_aig(&aig, &assignment);
            let mapped_out = common::simulate_netlist(&aig, &lib, &netlist, &assignment);
            prop_assert_eq!(aig_out, mapped_out);
        }
    }
}

#[test]
fn library_addition_does_not_increase_cost() {
    let mut aig = Aig::new();
    let a = aig.add_input("a");
    let b = aig.add_input("b");
    let y = aig.xor(a, b);
    aig.add_output("y", y);

    let toml_small = r#"
[[cell]]
name = "INV"
inputs = ["a"]
output = "y"
function = "!a"

[[cell]]
name = "AND2"
inputs = ["a", "b"]
output = "y"
function = "a & b"

[[cell]]
name = "OR2"
inputs = ["a", "b"]
output = "y"
function = "a | b"
"#;
    let toml_with_xor = format!("{}\n[[cell]]\nname = \"XOR2\"\ninputs = [\"a\", \"b\"]\noutput = \"y\"\nfunction = \"a ^ b\"\n", toml_small);

    use std::io::Write;
    let mut t1 = tempfile::NamedTempFile::new().unwrap();
    t1.write_all(toml_small.as_bytes()).unwrap();
    let lib1 = opt_cells::frontend::library::load_library(t1.path().to_str().unwrap()).unwrap();
    let n1 = opt_cells::mapper::map_aig(&aig, &lib1).unwrap();

    let mut t2 = tempfile::NamedTempFile::new().unwrap();
    t2.write_all(toml_with_xor.as_bytes()).unwrap();
    let lib2 = opt_cells::frontend::library::load_library(t2.path().to_str().unwrap()).unwrap();
    let n2 = opt_cells::mapper::map_aig(&aig, &lib2).unwrap();

    assert!(n2.total_cells <= n1.total_cells, "adding XOR2 should not increase cell count: {} vs {}", n2.total_cells, n1.total_cells);
}

