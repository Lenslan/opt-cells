use std::collections::HashMap;

use crate::aig::{Aig, NodeId, NodeKind};
use crate::frontend::library::CellLib;
use crate::mapper::MappedNetlist;

pub struct ReportInput<'a> {
    pub aig: &'a Aig,
    pub lib: &'a CellLib,
    pub netlist: &'a MappedNetlist,
    pub input_path: &'a str,
    pub library_path: &'a str,
    /// Internal-name -> display-name (e.g. "state__3" -> "state[3]").
    pub display_names: &'a HashMap<String, String>,
}

pub fn render(r: &ReportInput) -> String {
    let mut out = String::new();
    let hdr = "═".repeat(63);
    let sep = "─".repeat(63);

    out.push_str(&hdr);
    out.push('\n');
    out.push_str("  opt-cells mapping report\n");
    out.push_str(&hdr);
    out.push('\n');
    out.push_str(&format!("  Input file       : {}\n", r.input_path));
    out.push_str(&format!("  Cell library     : {}\n", r.library_path));
    out.push_str(&format!("  Total cells used : {}\n", r.netlist.total_cells));
    out.push_str(&sep);
    out.push('\n');
    out.push_str("  Mapped netlist\n");
    out.push_str(&sep);
    out.push('\n');

    // Resolve PO names per AIG node.
    let mut po_at: HashMap<NodeId, Vec<(String, bool)>> = HashMap::new();
    for (name, node, invert) in &r.netlist.outputs {
        let label = r
            .display_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        po_at.entry(*node).or_default().push((label, *invert));
    }

    // Resolve cell uid driving each node for output expression.
    // NOTE: when an INV is inserted at the same aig_node as the positive-form cell,
    // both share aig_node; insertion order means later insert wins. For our purpose
    // (rendering "leaf labels"), we want to point at the positive-form cell since
    // it's the producer of the AIG node's actual value. The phase 2 algorithm pushes
    // the INV cell before the positive form, so the positive form overwrites — which
    // is the behavior we want.
    let mut node_to_uid: HashMap<NodeId, u32> = HashMap::new();
    for c in &r.netlist.cells {
        node_to_uid.insert(c.aig_node, c.uid);
    }

    for c in &r.netlist.cells {
        let cell = r.lib.cells.iter().find(|x| x.id == c.cell_id);
        let cell_name = cell.map(|x| x.name.as_str()).unwrap_or("???");
        // Build "(pinName=leafExpr, ...)"
        let mut pin_str = String::new();
        pin_str.push('(');
        if let Some(cell) = cell {
            for (i, pi) in c.pin_inputs.iter().enumerate() {
                if i > 0 {
                    pin_str.push_str(", ");
                }
                let pin_name = cell.inputs.get(i).map(|s| s.as_str()).unwrap_or("?");
                let leaf_label = leaf_label(r, pi.leaf, &node_to_uid);
                if pi.invert {
                    pin_str.push_str(&format!("{}=!{}", pin_name, leaf_label));
                } else {
                    pin_str.push_str(&format!("{}={}", pin_name, leaf_label));
                }
            }
        }
        pin_str.push(')');
        // Determine output target: PO name if this node drives one; else internal uX.
        let out_label = if let Some(pos) = po_at.get(&c.aig_node) {
            pos.iter()
                .map(|(n, inv)| {
                    let phys_inv = *inv ^ c.produces_negation;
                    if phys_inv {
                        format!("!{}", n)
                    } else {
                        n.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            format!("n{}", c.aig_node.0)
        };
        out.push_str(&format!(
            "  u{} : {}  {}  -> {}\n",
            c.uid, cell_name, pin_str, out_label
        ));
    }
    out.push('\n');
    out.push_str(&sep);
    out.push('\n');
    out.push_str("  Cell usage\n");
    out.push_str(&sep);
    out.push('\n');
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for c in &r.netlist.cells {
        let cell = r.lib.cells.iter().find(|x| x.id == c.cell_id);
        let name = cell.map(|x| x.name.as_str()).unwrap_or("???");
        *counts.entry(name).or_default() += 1;
    }
    let mut counts_vec: Vec<_> = counts.into_iter().collect();
    counts_vec.sort_by_key(|(name, _)| *name);
    for (name, n) in counts_vec {
        out.push_str(&format!("  {} × {}\n", name, n));
    }
    out.push_str(&hdr);
    out.push('\n');
    out
}

fn leaf_label(r: &ReportInput, leaf: NodeId, node_to_uid: &HashMap<NodeId, u32>) -> String {
    match &r.aig.node(leaf).kind {
        NodeKind::Const0 => "0".to_string(),
        NodeKind::PrimaryInput { name } => r
            .display_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone()),
        NodeKind::And2 { .. } => {
            if let Some(uid) = node_to_uid.get(&leaf) {
                format!("n{}_u{}", leaf.0, uid)
            } else {
                format!("n{}", leaf.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::Aig;
    use crate::aig::Tt64;
    use crate::frontend::library::{CellDecl, CellId};
    use crate::mapper::netlist::{CellInstance, PinInput};

    #[test]
    fn renders_nand_example() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab = aig.and(a, b);
        aig.add_output("y", ab.inv());
        let lib = CellLib {
            cells: vec![CellDecl {
                id: CellId(0),
                name: "NAND2".into(),
                inputs: vec!["a".into(), "b".into()],
                output_pin: "y".into(),
                tt: Tt64(0x7),
                n_inputs: 2,
            }],
            notes: vec![],
        };
        let netlist = MappedNetlist {
            cells: vec![CellInstance {
                uid: 0,
                cell_id: CellId(0),
                aig_node: ab.node,
                pin_inputs: vec![
                    PinInput {
                        leaf: a.node,
                        invert: false,
                    },
                    PinInput {
                        leaf: b.node,
                        invert: false,
                    },
                ],
                produces_negation: true,
            }],
            outputs: vec![("y".into(), ab.node, true)],
            total_cells: 1,
        };
        let display: HashMap<String, String> = HashMap::new();
        let s = render(&ReportInput {
            aig: &aig,
            lib: &lib,
            netlist: &netlist,
            input_path: "examples/nand.dsl",
            library_path: "libs/basic.toml",
            display_names: &display,
        });
        assert!(s.contains("Total cells used : 1"));
        assert!(s.contains("NAND2"));
        assert!(s.contains("(a=a, b=b)"));
        assert!(s.contains("NAND2 × 1"));
    }
}
