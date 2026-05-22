use std::collections::{HashMap, VecDeque};

use crate::aig::cuts::Cut;
use crate::aig::{Aig, NodeId, NodeKind};
use crate::error::OptCellsError;
use crate::frontend::library::{CellId, CellLib};
use crate::mapper::netlist::{CellInstance, MappedNetlist, PinInput};
use crate::mapper::phase1::Phase1Result;

pub fn run(
    aig: &Aig,
    cuts: &[Vec<Cut>],
    lib: &CellLib,
    p1: &Phase1Result,
) -> Result<MappedNetlist, OptCellsError> {
    let inv_cell_id = find_inv_cell(lib);

    let mut required: HashMap<(NodeId, bool), Option<u32>> = HashMap::new();
    let mut queue: VecDeque<(NodeId, bool)> = VecDeque::new();

    for (_, edge) in aig.outputs() {
        queue.push_back((edge.node, edge.invert));
    }

    let mut uid_counter: u32 = 0;
    let mut cells: Vec<CellInstance> = Vec::new();

    while let Some((node, neg_polarity)) = queue.pop_front() {
        if required.contains_key(&(node, neg_polarity)) { continue; }

        // PIs / Const0: wires only.
        if matches!(aig.node(node).kind, NodeKind::Const0 | NodeKind::PrimaryInput { .. }) {
            required.insert((node, neg_polarity), None);
            continue;
        }

        let choice_opt = if neg_polarity { p1.best_neg[node.0 as usize].as_ref() } else { p1.best_pos[node.0 as usize].as_ref() };
        let choice = choice_opt.ok_or_else(|| OptCellsError::Mapping {
            message: format!("no library cell can implement function at AIG node {}", node.0),
        })?;

        if choice.via_inv {
            let inv_id = inv_cell_id.ok_or_else(|| OptCellsError::Mapping {
                message: format!(
                    "node {} requires inversion but library has no INV-class cell",
                    node.0
                ),
            })?;
            let uid = uid_counter; uid_counter += 1;
            cells.push(CellInstance {
                uid,
                cell_id: CellId(inv_id),
                aig_node: node,
                pin_inputs: vec![PinInput { leaf: node, invert: false }],
                produces_negation: true,
            });
            required.insert((node, neg_polarity), Some(uid));
            queue.push_back((node, false));
        } else {
            let cut = &cuts[node.0 as usize][choice.cut_index];
            let uid = uid_counter; uid_counter += 1;
            let n_inputs = choice.mapping.n_inputs as usize;
            let mut pin_inputs: Vec<PinInput> = vec![PinInput { leaf: NodeId(0), invert: false }; n_inputs];
            for (leaf_pos, &leaf) in cut.leaves.iter().enumerate() {
                let pin = choice.mapping.pin_perm[leaf_pos] as usize;
                let invert = (choice.mapping.input_negation >> leaf_pos) & 1 == 1;
                pin_inputs[pin] = PinInput { leaf, invert };
            }
            cells.push(CellInstance {
                uid,
                cell_id: choice.mapping.cell_id,
                aig_node: node,
                pin_inputs,
                produces_negation: choice.mapping.output_negation,
            });
            required.insert((node, neg_polarity), Some(uid));
            for (leaf_pos, &leaf) in cut.leaves.iter().enumerate() {
                let neg = (choice.mapping.input_negation >> leaf_pos) & 1 == 1;
                queue.push_back((leaf, neg));
            }
        }
    }

    let outputs: Vec<(String, NodeId, bool)> = aig
        .outputs()
        .iter()
        .map(|(name, edge)| (name.clone(), edge.node, edge.invert))
        .collect();

    let total = cells.len();
    Ok(MappedNetlist { cells, outputs, total_cells: total })
}

fn find_inv_cell(lib: &CellLib) -> Option<u32> {
    lib.cells.iter()
        .find(|c| c.n_inputs == 1 && c.tt.0 == 0x1)
        .map(|c| c.id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::enumerate_cuts;
    use crate::aig::Tt64;
    use crate::frontend::library::CellDecl;
    use crate::match_npn::NpnLibIndex;
    use crate::mapper::phase1;

    fn nand2_lib() -> CellLib {
        CellLib {
            cells: vec![
                CellDecl { id: CellId(0), name: "INV".into(), inputs: vec!["a".into()], output_pin: "y".into(), tt: Tt64(0x1), n_inputs: 1 },
                CellDecl { id: CellId(1), name: "NAND2".into(), inputs: vec!["a".into(), "b".into()], output_pin: "y".into(), tt: Tt64(0x7), n_inputs: 2 },
            ],
            notes: vec![],
        }
    }

    #[test]
    fn nand_maps_to_single_nand2_cell() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab = aig.and(a, b);
        aig.add_output("y", ab.inv());
        let cuts = enumerate_cuts(&aig);
        let lib = nand2_lib();
        let idx = NpnLibIndex::build(&lib);
        let p1 = phase1::run(&aig, &cuts, &idx);
        let nl = run(&aig, &cuts, &lib, &p1).unwrap();
        assert_eq!(nl.total_cells, 1);
        assert_eq!(nl.cells[0].cell_id, CellId(1));
    }
}
