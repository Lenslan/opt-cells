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

/// Simulate the MappedNetlist. Each cell drives the signal (aig_node, output_negated);
/// a pin consumes the signal (leaf, leaf_negated). Real INV cells are evaluated like any
/// other cell — there are no free pin inversions.
#[allow(dead_code)]
pub fn simulate_netlist(
    aig: &Aig,
    lib: &CellLib,
    netlist: &MappedNetlist,
    inputs: &HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut sig: HashMap<(NodeId, bool), bool> = HashMap::new();

    // Seed external signals: positive primary inputs and both polarities of constants.
    for (i, node) in aig.nodes().iter().enumerate() {
        let id = NodeId(i as u32);
        match &node.kind {
            NodeKind::PrimaryInput { name } => {
                sig.insert((id, false), *inputs.get(name).unwrap_or(&false));
            }
            NodeKind::Const0 => {
                sig.insert((id, false), false);
                sig.insert((id, true), true);
            }
            _ => {}
        }
    }

    // Fixpoint: fill a cell's output once all its pin sources are known. DAG => terminates.
    let mut progress = true;
    while progress {
        progress = false;
        for c in &netlist.cells {
            let key = (c.aig_node, c.output_negated);
            if sig.contains_key(&key) {
                continue;
            }
            let mut pattern: u32 = 0;
            let mut ready = true;
            for (pin_idx, pi) in c.pin_inputs.iter().enumerate() {
                match sig.get(&(pi.leaf, pi.leaf_negated)) {
                    Some(true) => pattern |= 1u32 << pin_idx,
                    Some(false) => {}
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            let cell_decl = lib
                .cells
                .iter()
                .find(|x| x.id == c.cell_id)
                .expect("cell decl");
            let out = (cell_decl.tt.0 >> pattern) & 1 == 1;
            sig.insert(key, out);
            progress = true;
        }
    }

    let mut out_vals: HashMap<String, bool> = HashMap::new();
    for (name, node, invert) in &netlist.outputs {
        let v = *sig.get(&(*node, *invert)).unwrap_or(&false);
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
