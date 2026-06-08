mod common;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use opt_cells::aig::{Aig, NodeId, NodeKind};
use opt_cells::frontend::elaborate;
use opt_cells::frontend::library::{load_library, CellDecl, CellLib};
use opt_cells::frontend::parser;
use opt_cells::mapper::{map_aig, rewrite_for_library, MappedNetlist};
use opt_cells::report::{render_tcl, ReportInput};

struct MappedCase {
    original_aig: Aig,
    mapped_aig: Aig,
    lib: CellLib,
    netlist: MappedNetlist,
    display_names: HashMap<String, String>,
    net_aliases: HashMap<(NodeId, bool), String>,
    tcl: String,
}

fn workspace_basic_lib_path() -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("workspace/lib/basic.toml");
    path.to_string_lossy().into_owned()
}

fn map_inline(input_text: &str) -> MappedCase {
    let input_path = "inline.dsl";
    let library_path = workspace_basic_lib_path();
    let program = parser::parse_program(input_text, input_path).expect("parse DSL");
    let elab = elaborate::elaborate(&program, input_path, input_text).expect("elaborate DSL");
    let original_aig = elab.aig.clone();
    let lib = load_library(&library_path).expect("load workspace/lib/basic.toml");

    let mut mapped_aig = elab.aig;
    let mut netlist = map_aig(&mapped_aig, &lib).expect("map AIG");
    let mut rewritten_aig = mapped_aig.clone();
    if rewrite_for_library(&mut rewritten_aig, &lib) {
        let rewritten_netlist = map_aig(&rewritten_aig, &lib).expect("map rewritten AIG");
        if rewritten_netlist.total_cells < netlist.total_cells {
            mapped_aig = rewritten_aig;
            netlist = rewritten_netlist;
        }
    }

    let display_names = elab.display_names;
    let net_aliases = elab.net_aliases;
    let tcl = render_tcl(&ReportInput {
        aig: &mapped_aig,
        lib: &lib,
        netlist: &netlist,
        input_path,
        library_path: &library_path,
        display_names: &display_names,
        net_aliases: &net_aliases,
    });

    MappedCase {
        original_aig,
        mapped_aig,
        lib,
        netlist,
        display_names,
        net_aliases,
        tcl,
    }
}

fn primary_input_names(aig: &Aig) -> Vec<String> {
    aig.nodes()
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::PrimaryInput { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn assert_logic_equivalent(case: &MappedCase) {
    let pi_names = primary_input_names(&case.original_aig);
    assert!(
        pi_names.len() <= 12,
        "exhaustive simulation is intentionally bounded"
    );

    for assignment in common::enumerate_input_assignments(&pi_names) {
        let expected = common::simulate_aig(&case.original_aig, &assignment);
        let actual =
            common::simulate_netlist(&case.mapped_aig, &case.lib, &case.netlist, &assignment);
        assert_eq!(
            expected, actual,
            "mapped netlist differs from input logic for assignment {assignment:?}\n{}",
            case.tcl
        );
    }
}

fn cell_decl(case: &MappedCase, id: opt_cells::frontend::library::CellId) -> &CellDecl {
    case.lib
        .cells
        .iter()
        .find(|cell| cell.id == id)
        .expect("mapped cell exists in library")
}

fn instance_name(cell_name: &str, uid: u32) -> String {
    format!("eco_{}_u{}", cell_name, uid)
}

fn internal_net_name(node: NodeId, uid: u32) -> String {
    format!("eco_n{}_u{}", node.0, uid)
}

fn expected_tcl_atom(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.' | '/'))
    {
        name.to_string()
    } else {
        format!("{{{}}}", name.replace('\\', "\\\\").replace('}', "\\}"))
    }
}

fn po_labels(case: &MappedCase, node: NodeId, negated: bool) -> Vec<String> {
    case.netlist
        .outputs
        .iter()
        .filter(|(_, out_node, out_negated)| *out_node == node && *out_negated == negated)
        .map(|(name, _, _)| {
            case.display_names
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone())
        })
        .collect()
}

fn is_primary_output_signal(case: &MappedCase, node: NodeId, negated: bool) -> bool {
    case.netlist
        .outputs
        .iter()
        .any(|(_, out_node, out_negated)| *out_node == node && *out_negated == negated)
}

fn signal_uid(case: &MappedCase) -> HashMap<(NodeId, bool), u32> {
    case.netlist
        .cells
        .iter()
        .map(|cell| ((cell.aig_node, cell.output_negated), cell.uid))
        .collect()
}

fn alias_net_name(case: &MappedCase, node: NodeId, negated: bool) -> Option<String> {
    match &case.mapped_aig.node(node).kind {
        NodeKind::Const0 => None,
        NodeKind::PrimaryInput { .. } if !negated => None,
        _ => case.net_aliases.get(&(node, negated)).cloned(),
    }
}

fn driven_signal_net_name(
    case: &MappedCase,
    signal_uid: &HashMap<(NodeId, bool), u32>,
    node: NodeId,
    negated: bool,
) -> String {
    po_labels(case, node, negated)
        .into_iter()
        .next()
        .or_else(|| alias_net_name(case, node, negated))
        .unwrap_or_else(|| match signal_uid.get(&(node, negated)) {
            Some(uid) => internal_net_name(node, *uid),
            None => format!("eco_n{}", node.0),
        })
}

fn source_net_name(
    case: &MappedCase,
    signal_uid: &HashMap<(NodeId, bool), u32>,
    node: NodeId,
    negated: bool,
) -> String {
    match &case.mapped_aig.node(node).kind {
        NodeKind::Const0 => {
            if negated {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        NodeKind::PrimaryInput { name } if !negated => case
            .display_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone()),
        _ => driven_signal_net_name(case, signal_uid, node, negated),
    }
}

fn driven_net_name(case: &MappedCase, node: NodeId, negated: bool, uid: u32) -> String {
    po_labels(case, node, negated)
        .into_iter()
        .next()
        .or_else(|| alias_net_name(case, node, negated))
        .unwrap_or_else(|| internal_net_name(node, uid))
}

fn parse_create_cells(tcl: &str) -> HashMap<String, String> {
    tcl.lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.first() != Some(&"create_cell") {
                return None;
            }
            let instance = parts.get(1)?.to_string();
            let lib_ref = parts.get(3)?;
            let cell = lib_ref.strip_prefix("*/")?.strip_suffix(']')?.to_string();
            Some((instance, cell))
        })
        .collect()
}

fn parse_create_nets(tcl: &str) -> HashSet<String> {
    tcl.lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.first() == Some(&"create_net") {
                parts.get(1).map(|name| name.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn parse_connects(tcl: &str) -> HashSet<(String, String, String)> {
    tcl.lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.first() != Some(&"connect_net") {
                return None;
            }
            let net = parts.get(1)?.to_string();
            assert_eq!(parts.get(2), Some(&"[get_pin"), "malformed Tcl: {line}");
            let inst_pin = parts.get(3)?.strip_suffix(']')?;
            let (inst, pin) = inst_pin.split_once('/')?;
            Some((net, inst.to_string(), pin.to_string()))
        })
        .collect()
}

fn assert_tcl_matches_netlist(case: &MappedCase) {
    for line in case.tcl.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            line.trim_start().starts_with("create_cell")
                || line.trim_start().starts_with("create_net")
                || line.trim_start().starts_with("connect_net"),
            "unexpected Tcl command: {line}"
        );
    }

    let created_cells = parse_create_cells(&case.tcl);
    assert_eq!(
        created_cells.len(),
        case.netlist.cells.len(),
        "create_cell count must match mapped cells\n{}",
        case.tcl
    );

    for mapped in &case.netlist.cells {
        let decl = cell_decl(case, mapped.cell_id);
        let inst = instance_name(&decl.name, mapped.uid);
        assert_eq!(
            created_cells.get(&inst),
            Some(&decl.name),
            "missing or wrong create_cell for {inst}\n{}",
            case.tcl
        );
    }

    let created_nets = parse_create_nets(&case.tcl);
    let expected_nets: HashSet<String> = case
        .netlist
        .cells
        .iter()
        .filter(|cell| !is_primary_output_signal(case, cell.aig_node, cell.output_negated))
        .map(|cell| {
            alias_net_name(case, cell.aig_node, cell.output_negated)
                .unwrap_or_else(|| internal_net_name(cell.aig_node, cell.uid))
        })
        .collect();
    assert_eq!(
        created_nets, expected_nets,
        "create_net commands must match non-output driven nets\n{}",
        case.tcl
    );

    let signal_uid = signal_uid(case);
    let mut expected_connects = HashSet::new();
    for mapped in &case.netlist.cells {
        let decl = cell_decl(case, mapped.cell_id);
        let inst = instance_name(&decl.name, mapped.uid);
        for (pin_idx, input) in mapped.pin_inputs.iter().enumerate() {
            let pin = decl.inputs.get(pin_idx).expect("pin exists").clone();
            let net = source_net_name(case, &signal_uid, input.leaf, input.leaf_negated);
            expected_connects.insert((expected_tcl_atom(&net), inst.clone(), pin));
        }

        let out_net = driven_net_name(case, mapped.aig_node, mapped.output_negated, mapped.uid);
        expected_connects.insert((expected_tcl_atom(&out_net), inst, decl.output_pin.clone()));
    }

    let actual_connects = parse_connects(&case.tcl);
    assert_eq!(
        actual_connects, expected_connects,
        "connect_net commands must match mapped cell pins\n{}",
        case.tcl
    );
}

fn assert_default_names_are_well_formed(case: &MappedCase) {
    for mapped in &case.netlist.cells {
        let decl = cell_decl(case, mapped.cell_id);
        let inst = instance_name(&decl.name, mapped.uid);
        assert!(
            inst.starts_with("eco_") && inst.ends_with(&format!("_u{}", mapped.uid)),
            "bad instance name: {inst}"
        );
    }

    for net in parse_create_nets(&case.tcl) {
        assert!(
            net.starts_with("eco_"),
            "created net must use the eco_ namespace: {net}"
        );
        if !case.net_aliases.values().any(|alias| alias == &net) {
            assert!(
                net.starts_with("eco_n") && net.contains("_u"),
                "default internal net must look like eco_n<node>_u<uid>: {net}"
            );
        }
    }
}

fn assert_case_is_consistent(case: &MappedCase) {
    assert_logic_equivalent(case);
    assert_tcl_matches_netlist(case);
    assert_default_names_are_well_formed(case);
}

fn used_cell_names(case: &MappedCase) -> HashSet<String> {
    case.netlist
        .cells
        .iter()
        .map(|cell| cell_decl(case, cell.cell_id).name.clone())
        .collect()
}

#[test]
fn simple_and_maps_to_one_basic_library_cell() {
    let case = map_inline("input a, b; output y; y = a & b;");

    assert_eq!(case.netlist.total_cells, 1, "{}", case.tcl);
    assert_eq!(
        used_cell_names(&case),
        HashSet::from(["AN2D0BWP7T40P140".to_string()])
    );
    assert!(case.tcl.contains("eco_AN2D0BWP7T40P140_u0"));
    assert!(case.tcl.contains("[get_pin eco_AN2D0BWP7T40P140_u0/A1]"));
    assert!(case.tcl.contains("[get_pin eco_AN2D0BWP7T40P140_u0/A2]"));
    assert!(case.tcl.contains("[get_pin eco_AN2D0BWP7T40P140_u0/Z]"));
    assert_case_is_consistent(&case);
}

#[test]
fn inverted_primary_input_uses_real_inverter_cell() {
    let case = map_inline("input a; output y; y = !a;");

    assert_eq!(case.netlist.total_cells, 1, "{}", case.tcl);
    assert_eq!(
        used_cell_names(&case),
        HashSet::from(["INVD0BWP7T40P140".to_string()])
    );
    assert!(case.tcl.contains("eco_INVD0BWP7T40P140_u0"));
    assert!(
        !case.tcl.contains("!a"),
        "no free pin inversion\n{}",
        case.tcl
    );
    assert_case_is_consistent(&case);
}

#[test]
fn complex_multi_output_logic_uses_at_least_ten_cells_and_is_equivalent() {
    let case = map_inline(
        r#"
input a, b, c, d, e, f, g, h, i, s;
output y0, y1, y2, y3, y4, y5, y6, y7, y8, y9, y10, y11;
y0 = a & b;
y1 = a | c;
y2 = !(b & c);
y3 = !(d | e);
y4 = a ^ b ^ c;
y5 = (d & e) | f;
y6 = (g | h) & i;
y7 = s ? h : i;
y8 = (a & b) | (c & d);
y9 = !((e | f) & (g | h));
y10 = a & b & c & d;
y11 = a | b | c | d;
"#,
    );

    assert!(
        case.netlist.total_cells >= 10,
        "complex case should exercise at least 10 cells\n{}",
        case.tcl
    );
    let used = used_cell_names(&case);
    for expected in [
        "AN2D0BWP7T40P140",
        "OR2D0BWP7T40P140",
        "ND2D0BWP7T40P140",
        "NR2D0BWP7T40P140",
        "XOR3D0BWP7T40P140",
        "MUX2D0BWP7T40P140",
        "AO22D0BWP7T40P140",
        "AN4D0BWP7T40P140",
        "OR4D0BWP7T40P140",
    ] {
        assert!(used.contains(expected), "missing {expected}\n{}", case.tcl);
    }
    assert_case_is_consistent(&case);
}

#[test]
fn vector_inputs_and_eco_aliases_are_named_correctly_in_tcl() {
    let case = map_inline(
        r#"
input state[3:0], a;
output y;
eco_parity = state[0] ^ state[1] ^ state[2] ^ state[3];
y = eco_parity ^ a;
"#,
    );

    assert!(case.netlist.total_cells >= 2, "{}", case.tcl);
    assert!(
        case.tcl.contains("create_net      eco_parity"),
        "{}",
        case.tcl
    );
    assert!(
        case.tcl.contains("connect_net     {state[3]}"),
        "vector input names must be Tcl-braced\n{}",
        case.tcl
    );
    assert!(
        case.tcl.contains("connect_net     {state[0]}"),
        "vector input names must be Tcl-braced\n{}",
        case.tcl
    );
    assert!(
        !case.tcl.contains("create_net      state["),
        "primary inputs must not be created as ECO nets\n{}",
        case.tcl
    );
    assert_case_is_consistent(&case);
}

#[test]
fn constant_outputs_are_a_zero_cell_corner_case() {
    let case = map_inline("output y0, y1; y0 = 0; y1 = 1;");

    assert_eq!(case.netlist.total_cells, 0);
    assert!(case.tcl.trim().is_empty(), "{}", case.tcl);
    assert_case_is_consistent(&case);
}
