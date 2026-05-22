use std::collections::HashMap;

use opt_cells::aig::{Aig, NodeId, NodeKind};
use opt_cells::frontend::library::CellLib;
use opt_cells::mapper::MappedNetlist;

/// Simulate the AIG on `inputs` and return per-PO value.
#[allow(dead_code)]
pub fn simulate_aig(aig: &Aig, inputs: &HashMap<String, bool>) -> HashMap<String, bool> {
    let n = aig.num_nodes();
    let mut value = vec![false; n];
    for i in 0..n {
        let id = NodeId(i as u32);
        match &aig.node(id).kind {
            NodeKind::Const0 => value[i] = false,
            NodeKind::PrimaryInput { name } => {
                value[i] = *inputs.get(name).unwrap_or(&false);
            }
            NodeKind::And2 { l, r } => {
                let lv = value[l.node.0 as usize] ^ l.invert;
                let rv = value[r.node.0 as usize] ^ r.invert;
                value[i] = lv && rv;
            }
        }
    }
    let mut out: HashMap<String, bool> = HashMap::new();
    for (name, edge) in aig.outputs() {
        let v = value[edge.node.0 as usize] ^ edge.invert;
        out.insert(name.clone(), v);
    }
    out
}

/// Simulate the MappedNetlist by applying each cell's library function to its pin inputs.
#[allow(dead_code)]
pub fn simulate_netlist(
    aig: &Aig,
    lib: &CellLib,
    netlist: &MappedNetlist,
    inputs: &HashMap<String, bool>,
) -> HashMap<String, bool> {
    let n = aig.num_nodes();
    let mut value: Vec<bool> = vec![false; n];

    for (i, node) in aig.nodes().iter().enumerate() {
        if let NodeKind::PrimaryInput { name } = &node.kind {
            value[i] = *inputs.get(name).unwrap_or(&false);
        }
    }

    // Evaluate cells in topological order (sort by aig_node id ascending).
    let mut ordered: Vec<&opt_cells::mapper::CellInstance> = netlist.cells.iter().collect();
    ordered.sort_by_key(|c| c.aig_node.0);

    for c in ordered {
        let cell_decl = lib.cells.iter().find(|x| x.id == c.cell_id).expect("cell decl");
        // Compute cell output by evaluating native TT over input pin values.
        let mut pattern: u32 = 0;
        for (pin_idx, pi) in c.pin_inputs.iter().enumerate() {
            let v = value[pi.leaf.0 as usize] ^ pi.invert;
            if v { pattern |= 1u32 << pin_idx; }
        }
        let out = (cell_decl.tt.0 >> pattern) & 1 == 1;
        value[c.aig_node.0 as usize] = out;
    }

    let mut out_vals: HashMap<String, bool> = HashMap::new();
    for (name, node, invert) in &netlist.outputs {
        let v = value[node.0 as usize] ^ invert;
        out_vals.insert(name.clone(), v);
    }
    out_vals
}

#[allow(dead_code)]
pub fn enumerate_input_assignments(pi_names: &[String]) -> Vec<HashMap<String, bool>> {
    let n = pi_names.len();
    let total = 1u64 << n;
    let mut out = Vec::new();
    for pattern in 0..total {
        let mut m = HashMap::new();
        for (i, name) in pi_names.iter().enumerate() {
            m.insert(name.clone(), (pattern >> i) & 1 == 1);
        }
        out.push(m);
    }
    out
}
