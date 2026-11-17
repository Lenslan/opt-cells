use std::collections::HashMap;

use crate::aig::{Aig, NodeId, NodeKind};
use crate::error::OptCellsError;
use crate::frontend::library::{CellDecl, CellId, CellLib};
use crate::mapper::MappedNetlist;

/// Namespace of every cell and net the ECO script creates. A DSL intermediate opts into it
/// by starting its name with this marker.
const ECO: &str = "eco_";

/// Name prefix for one run: `eco_`, or `eco_<custom>_` when the caller tags the run.
///
/// The tag is written verbatim into Tcl words and `inst/pin` paths, so only
/// `[A-Za-z0-9_]+` is accepted.
pub fn name_prefix(custom: Option<&str>) -> Result<String, OptCellsError> {
    let Some(custom) = custom else {
        return Ok(ECO.to_string());
    };
    if custom.is_empty()
        || !custom
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(OptCellsError::InvalidPrefix(custom.to_string()));
    }
    Ok(format!("{ECO}{custom}_"))
}

pub struct ReportInput<'a> {
    pub aig: &'a Aig,
    pub lib: &'a CellLib,
    pub netlist: &'a MappedNetlist,
    pub input_path: &'a str,
    pub library_path: &'a str,
    /// Internal-name -> display-name (e.g. "state__3" -> "state[3]").
    pub display_names: &'a HashMap<String, String>,
    /// AIG signal `(node, polarity)` -> user-requested internal net label.
    pub net_aliases: &'a HashMap<(NodeId, bool), String>,
    /// Prefix of every created cell and net name; see [`name_prefix`].
    pub name_prefix: &'a str,
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
            alias_label(r, c.aig_node, c.output_negated)
                .unwrap_or_else(|| format!("n{}", c.aig_node.0))
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
            &instance_name(r.name_prefix, &cell.name, c.uid),
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
            &ctx.internal_net_name(c.aig_node, c.output_negated, c.uid),
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
        let inst_name = instance_name(r.name_prefix, &cell.name, c.uid);
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
            .or_else(|| self.alias_net_name(node, negated))
            .unwrap_or_else(|| internal_net_name(self.report.name_prefix, node, uid))
    }

    fn driven_signal_net_name(&self, node: NodeId, negated: bool) -> String {
        self.po_at
            .get(&(node, negated))
            .and_then(|names| names.first())
            .cloned()
            .or_else(|| self.alias_net_name(node, negated))
            .unwrap_or_else(|| match self.signal_uid.get(&(node, negated)) {
                Some(uid) => internal_net_name(self.report.name_prefix, node, *uid),
                None => format!("{}n{}", self.report.name_prefix, node.0),
            })
    }

    fn internal_net_name(&self, node: NodeId, negated: bool, uid: u32) -> String {
        self.alias_net_name(node, negated)
            .unwrap_or_else(|| internal_net_name(self.report.name_prefix, node, uid))
    }

    fn alias_net_name(&self, node: NodeId, negated: bool) -> Option<String> {
        match &self.report.aig.node(node).kind {
            NodeKind::Const0 => None,
            NodeKind::PrimaryInput { .. } if !negated => None,
            _ => alias_label(self.report, node, negated),
        }
    }
}

fn instance_name(prefix: &str, cell_name: &str, uid: u32) -> String {
    format!("{}{}_u{}", prefix, cell_name, uid)
}

fn internal_net_name(prefix: &str, node: NodeId, uid: u32) -> String {
    format!("{}n{}_u{}", prefix, node.0, uid)
}

/// The net label a DSL intermediate requested for `(node, negated)`, moved into this
/// run's namespace: its leading `eco_` marker becomes the run prefix.
fn alias_label(r: &ReportInput, node: NodeId, negated: bool) -> Option<String> {
    let alias = r.net_aliases.get(&(node, negated))?;
    Some(match alias.strip_prefix(ECO) {
        Some(rest) => format!("{}{}", r.name_prefix, rest),
        None => alias.clone(),
    })
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
        name.replace('\\', "\\\\").replace('}', "\\}")
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
                alias_label(r, leaf, true).unwrap_or_else(|| match signal_uid.get(&(leaf, true)) {
                    Some(uid) => format!("n{}_u{}", leaf.0, uid),
                    None => format!("n{}", leaf.0),
                })
            }
        }
        NodeKind::And2 { .. } => alias_label(r, leaf, leaf_negated).unwrap_or_else(|| {
            match signal_uid.get(&(leaf, leaf_negated)) {
                Some(uid) => format!("n{}_u{}", leaf.0, uid),
                None => format!("n{}", leaf.0),
            }
        }),
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
    fn name_prefix_defaults_to_eco() {
        assert_eq!(name_prefix(None).unwrap(), "eco_");
    }

    #[test]
    fn name_prefix_inserts_custom_tag_after_eco() {
        assert_eq!(name_prefix(Some("test")).unwrap(), "eco_test_");
        assert_eq!(name_prefix(Some("Fix_2")).unwrap(), "eco_Fix_2_");
    }

    #[test]
    fn name_prefix_rejects_empty_and_unsafe_tags() {
        for bad in ["", "a b", "a/b", "a-b", "a.b", "x[0]", "$x", "caf\u{e9}"] {
            match name_prefix(Some(bad)) {
                Err(OptCellsError::InvalidPrefix(got)) => assert_eq!(got, bad),
                other => panic!("{bad:?} must be rejected, got {other:?}"),
            }
        }
    }

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
        let aliases: HashMap<(NodeId, bool), String> = HashMap::new();
        let s = render(&ReportInput {
            aig: &aig,
            lib: &lib,
            netlist: &netlist,
            input_path: "examples/nand.dsl",
            library_path: "libs/basic.toml",
            display_names: &display,
            net_aliases: &aliases,
            name_prefix: "eco_",
        });
        assert!(s.contains("Total cells used : 1"));
        assert!(s.contains("NAND2"));
        assert!(s.contains("(a=a, b=b)"));
        assert!(s.contains("NAND2 x 1"));
        assert!(s.contains("create_cell"));
        assert!(s.contains("connect_net"));
    }

    /// `net_aliases` built by hand may hold labels outside the `eco_` namespace; those
    /// are the caller's exact names and are not rewritten.
    #[test]
    fn alias_without_the_eco_marker_is_kept_as_is() {
        // y = !(a & b) as AND2 (u1) feeding INV (u0), with the AND2 output labelled "mid".
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab = aig.and(a, b);
        aig.add_output("y", ab.inv());
        let lib = CellLib {
            cells: vec![
                CellDecl {
                    id: CellId(0),
                    name: "INV".into(),
                    inputs: vec!["a".into()],
                    output_pin: "y".into(),
                    tt: Tt64(0x1),
                    n_inputs: 1,
                },
                CellDecl {
                    id: CellId(1),
                    name: "AND2".into(),
                    inputs: vec!["a".into(), "b".into()],
                    output_pin: "y".into(),
                    tt: Tt64(0x8),
                    n_inputs: 2,
                },
            ],
            notes: vec![],
        };
        let pin = |leaf| PinInput {
            leaf,
            leaf_negated: false,
        };
        let netlist = MappedNetlist {
            cells: vec![
                CellInstance {
                    uid: 0,
                    cell_id: CellId(0),
                    aig_node: ab.node,
                    pin_inputs: vec![pin(ab.node)],
                    output_negated: true,
                },
                CellInstance {
                    uid: 1,
                    cell_id: CellId(1),
                    aig_node: ab.node,
                    pin_inputs: vec![pin(a.node), pin(b.node)],
                    output_negated: false,
                },
            ],
            outputs: vec![("y".into(), ab.node, true)],
            total_cells: 2,
        };
        let display: HashMap<String, String> = HashMap::new();
        let aliases = HashMap::from([((ab.node, false), "mid".to_string())]);

        let s = render(&ReportInput {
            aig: &aig,
            lib: &lib,
            netlist: &netlist,
            input_path: "inline.dsl",
            library_path: "lib.toml",
            display_names: &display,
            net_aliases: &aliases,
            name_prefix: "eco_test_",
        });

        assert!(s.contains("  u1 : AND2  (a=a, b=b)  -> mid\n"), "{s}");
        assert!(s.contains("create_net      mid\n"), "{s}");
        assert!(s.contains("[get_pin eco_test_INV_u0/a]"), "{s}");
    }
}
