use std::collections::HashMap;

use crate::aig::{Aig, NodeId, NodeKind};
use crate::frontend::library::{CellDecl, CellId, CellLib};
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
    let hdr = "=".repeat(63);
    let sep = "-".repeat(63);

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

    let mut po_at: HashMap<(NodeId, bool), Vec<String>> = HashMap::new();
    for (name, node, invert) in &r.netlist.outputs {
        let label = r
            .display_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        po_at.entry((*node, *invert)).or_default().push(label);
    }

    let mut signal_uid: HashMap<(NodeId, bool), u32> = HashMap::new();
    for c in &r.netlist.cells {
        signal_uid.insert((c.aig_node, c.output_negated), c.uid);
    }

    for c in &r.netlist.cells {
        let cell = r.lib.cells.iter().find(|x| x.id == c.cell_id);
        let cell_name = cell.map(|x| x.name.as_str()).unwrap_or("???");
        let mut pin_str = String::from("(");
        if let Some(cell) = cell {
            for (i, pi) in c.pin_inputs.iter().enumerate() {
                if i > 0 {
                    pin_str.push_str(", ");
                }
                let pin_name = cell.inputs.get(i).map(|s| s.as_str()).unwrap_or("?");
                let label = leaf_label(r, pi.leaf, pi.leaf_negated, &signal_uid);
                pin_str.push_str(&format!("{}={}", pin_name, label));
            }
        }
        pin_str.push(')');

        let out_label = if let Some(names) = po_at.get(&(c.aig_node, c.output_negated)) {
            names.join(", ")
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
    out.push_str("  Tcl script\n");
    out.push_str(&sep);
    out.push('\n');
    out.push_str(&render_tcl(r));
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
        out.push_str(&format!("  {} x {}\n", name, n));
    }
    out.push_str(&hdr);
    out.push('\n');
    out
}

pub fn render_tcl(r: &ReportInput) -> String {
    let mut out = String::new();
    let ctx = TclContext::new(r);

    for c in &r.netlist.cells {
        let Some(cell) = ctx.cell_decl(c.cell_id) else {
            continue;
        };
        push_tcl_line(
            &mut out,
            "create_cell",
            &instance_name(&cell.name, c.uid),
            Some(&format!("[get_lib_cells */{}]", cell.name)),
        );
    }

    for c in &r.netlist.cells {
        if ctx.is_primary_output_signal(c.aig_node, c.output_negated) {
            continue;
        }
        push_tcl_line(
            &mut out,
            "create_net",
            &internal_net_name(c.aig_node, c.uid),
            None,
        );
    }

    if !r.netlist.cells.is_empty() {
        out.push('\n');
    }

    for c in &r.netlist.cells {
        let Some(cell) = ctx.cell_decl(c.cell_id) else {
            continue;
        };
        let inst_name = instance_name(&cell.name, c.uid);
        for (i, pi) in c.pin_inputs.iter().enumerate() {
            let Some(pin_name) = cell.inputs.get(i) else {
                continue;
            };
            let net_name = ctx.source_net_name(pi.leaf, pi.leaf_negated);
            push_tcl_line(
                &mut out,
                "connect_net",
                &tcl_atom(&net_name),
                Some(&format!("[get_pin {}/{}]", inst_name, pin_name)),
            );
        }

        let out_net = ctx.driven_net_name(c.aig_node, c.output_negated, c.uid);
        push_tcl_line(
            &mut out,
            "connect_net",
            &tcl_atom(&out_net),
            Some(&format!("[get_pin {}/{}]", inst_name, cell.output_pin)),
        );
    }

    out
}

struct TclContext<'ctx, 'data> {
    report: &'ctx ReportInput<'data>,
    po_at: HashMap<(NodeId, bool), Vec<String>>,
    signal_uid: HashMap<(NodeId, bool), u32>,
}

impl<'ctx, 'data> TclContext<'ctx, 'data> {
    fn new(report: &'ctx ReportInput<'data>) -> Self {
        let mut po_at: HashMap<(NodeId, bool), Vec<String>> = HashMap::new();
        for (name, node, invert) in &report.netlist.outputs {
            let label = report
                .display_names
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone());
            po_at.entry((*node, *invert)).or_default().push(label);
        }

        let mut signal_uid: HashMap<(NodeId, bool), u32> = HashMap::new();
        for c in &report.netlist.cells {
            signal_uid.insert((c.aig_node, c.output_negated), c.uid);
        }

        TclContext {
            report,
            po_at,
            signal_uid,
        }
    }

    fn cell_decl(&self, id: CellId) -> Option<&CellDecl> {
        self.report.lib.cells.iter().find(|x| x.id == id)
    }

    fn is_primary_output_signal(&self, node: NodeId, negated: bool) -> bool {
        self.po_at.contains_key(&(node, negated))
    }

    fn source_net_name(&self, node: NodeId, negated: bool) -> String {
        match &self.report.aig.node(node).kind {
            NodeKind::Const0 => {
                if negated {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            NodeKind::PrimaryInput { name } if !negated => self
                .report
                .display_names
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone()),
            _ => self.driven_signal_net_name(node, negated),
        }
    }

    fn driven_net_name(&self, node: NodeId, negated: bool, uid: u32) -> String {
        self.po_at
            .get(&(node, negated))
            .and_then(|names| names.first())
            .cloned()
            .unwrap_or_else(|| internal_net_name(node, uid))
    }

    fn driven_signal_net_name(&self, node: NodeId, negated: bool) -> String {
        self.po_at
            .get(&(node, negated))
            .and_then(|names| names.first())
            .cloned()
            .unwrap_or_else(|| match self.signal_uid.get(&(node, negated)) {
                Some(uid) => internal_net_name(node, *uid),
                None => format!("eco_n{}", node.0),
            })
    }
}

fn instance_name(cell_name: &str, uid: u32) -> String {
    format!("eco_{}_u{}", cell_name, uid)
}

fn internal_net_name(node: NodeId, uid: u32) -> String {
    format!("eco_n{}_u{}", node.0, uid)
}

fn push_tcl_line(out: &mut String, cmd: &str, arg: &str, tail: Option<&str>) {
    match tail {
        Some(tail) => out.push_str(&format!("{:<15} {:<38} {}\n", cmd, arg, tail)),
        None => out.push_str(&format!("{:<15} {}\n", cmd, arg)),
    }
}

fn tcl_atom(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.' | '/'))
    {
        name.to_string()
    } else {
        format!("{{{}}}", name.replace('\\', "\\\\").replace('}', "\\}"))
    }
}

fn leaf_label(
    r: &ReportInput,
    leaf: NodeId,
    leaf_negated: bool,
    signal_uid: &HashMap<(NodeId, bool), u32>,
) -> String {
    match &r.aig.node(leaf).kind {
        NodeKind::Const0 => {
            if leaf_negated {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        NodeKind::PrimaryInput { name } => {
            if !leaf_negated {
                r.display_names
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            } else {
                match signal_uid.get(&(leaf, true)) {
                    Some(uid) => format!("n{}_u{}", leaf.0, uid),
                    None => format!("n{}", leaf.0),
                }
            }
        }
        NodeKind::And2 { .. } => match signal_uid.get(&(leaf, leaf_negated)) {
            Some(uid) => format!("n{}_u{}", leaf.0, uid),
            None => format!("n{}", leaf.0),
        },
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
                        leaf_negated: false,
                    },
                    PinInput {
                        leaf: b.node,
                        leaf_negated: false,
                    },
                ],
                output_negated: true,
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
        assert!(s.contains("NAND2 x 1"));
        assert!(s.contains("create_cell"));
        assert!(s.contains("connect_net"));
    }
}
