use crate::aig::cuts::Cut;
use crate::aig::{Aig, NodeKind};
use crate::match_npn::{InputMapping, NpnLibIndex};

#[derive(Debug, Clone)]
pub struct BestChoice {
    pub cost: u32,
    /// Index into the cuts vector for the node.
    pub cut_index: usize,
    pub mapping: InputMapping,
    /// True if this "best" was achieved by computing positive form + adding an INV cell.
    pub via_inv: bool,
}

#[derive(Debug)]
pub struct Phase1Result {
    pub best_pos: Vec<Option<BestChoice>>,
    pub best_neg: Vec<Option<BestChoice>>,
}

const INF: u32 = u32::MAX / 4;

pub fn run(aig: &Aig, cuts: &[Vec<Cut>], idx: &NpnLibIndex) -> Phase1Result {
    let n = aig.num_nodes();
    let mut best_pos: Vec<Option<BestChoice>> = vec![None; n];
    let mut best_neg: Vec<Option<BestChoice>> = vec![None; n];
    let mut cost_pos: Vec<u32> = vec![INF; n];
    let mut cost_neg: Vec<u32> = vec![INF; n];

    for idx_n in 0..n {
        match &aig.node(crate::aig::NodeId(idx_n as u32)).kind {
            NodeKind::Const0 => {
                cost_pos[idx_n] = 0;
                cost_neg[idx_n] = 0;
                continue;
            }
            NodeKind::PrimaryInput { .. } => {
                cost_pos[idx_n] = 0;
                cost_neg[idx_n] = 0;
                continue;
            }
            NodeKind::And2 { .. } => {}
        }

        let node_cuts = &cuts[idx_n];
        for (cut_index, cut) in node_cuts.iter().enumerate() {
            // Skip trivial cut (no cell could implement a single-leaf subgraph rooted at AND node).
            if cut.leaves.len() == 1 && cut.leaves[0].0 as usize == idx_n {
                continue;
            }
            let mappings = idx.matches(cut.tt.0, cut.k());
            for m in mappings {
                let mut leaves_total: u32 = 0;
                let mut feasible = true;
                for (leaf_pos, &leaf) in cut.leaves.iter().enumerate() {
                    let want_negated = (m.input_negation >> leaf_pos) & 1 == 1;
                    let leaf_cost = if want_negated {
                        cost_neg[leaf.0 as usize]
                    } else {
                        cost_pos[leaf.0 as usize]
                    };
                    if leaf_cost >= INF {
                        feasible = false;
                        break;
                    }
                    leaves_total = leaves_total.saturating_add(leaf_cost);
                }
                if !feasible {
                    continue;
                }
                let total = leaves_total.saturating_add(1);
                if !m.output_negation {
                    if total < cost_pos[idx_n] {
                        cost_pos[idx_n] = total;
                        best_pos[idx_n] = Some(BestChoice {
                            cost: total,
                            cut_index,
                            mapping: m,
                            via_inv: false,
                        });
                    }
                } else if total < cost_neg[idx_n] {
                    cost_neg[idx_n] = total;
                    best_neg[idx_n] = Some(BestChoice {
                        cost: total,
                        cut_index,
                        mapping: m,
                        via_inv: false,
                    });
                }
            }
        }

        // Consider "compute positive + INV" route for negative.
        if cost_pos[idx_n] < INF {
            let inv_cost = cost_pos[idx_n].saturating_add(1);
            if inv_cost < cost_neg[idx_n] {
                cost_neg[idx_n] = inv_cost;
                best_neg[idx_n] = Some(BestChoice {
                    cost: inv_cost,
                    cut_index: usize::MAX,
                    mapping: InputMapping {
                        cell_id: crate::frontend::library::CellId(u32::MAX),
                        n_inputs: 0,
                        pin_perm: [0; 6],
                        input_negation: 0,
                        output_negation: false,
                    },
                    via_inv: true,
                });
            }
        }
    }

    Phase1Result { best_pos, best_neg }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::{enumerate_cuts, Aig};
    use crate::frontend::library::{CellDecl, CellId, CellLib};

    fn make_nand2_lib() -> CellLib {
        CellLib {
            cells: vec![CellDecl {
                id: CellId(0),
                name: "NAND2".into(),
                inputs: vec!["a".into(), "b".into()],
                output_pin: "y".into(),
                tt: crate::aig::Tt64(0x7),
                n_inputs: 2,
            }],
            notes: vec![],
        }
    }

    #[test]
    fn nand_example_phase1_cost_is_1() {
        // y = !(a & b); should be NAND2 = 1 cell when computing negative form of (a & b).
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab = aig.and(a, b);
        aig.add_output("y", ab.inv());
        let cuts = enumerate_cuts(&aig);
        let lib = make_nand2_lib();
        let idx = NpnLibIndex::build(&lib);
        let res = run(&aig, &cuts, &idx);
        let neg_choice = res.best_neg[ab.node.0 as usize]
            .as_ref()
            .expect("must have neg choice");
        assert_eq!(neg_choice.cost, 1);
        assert!(!neg_choice.via_inv);
    }
}
