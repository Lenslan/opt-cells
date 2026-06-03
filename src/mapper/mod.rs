pub mod netlist;
pub mod phase1;
pub mod phase2;

pub use netlist::{CellInstance, MappedNetlist, PinInput};

use crate::aig::{enumerate_cuts, Aig, Edge, NodeKind};
use crate::error::OptCellsError;
use crate::frontend::library::CellLib;
use crate::match_npn::NpnLibIndex;

pub fn map_aig(aig: &Aig, lib: &CellLib) -> Result<MappedNetlist, OptCellsError> {
    let cuts = enumerate_cuts(aig);
    let idx = NpnLibIndex::build(lib);
    let has_inv = phase2::find_inv_cell(lib).is_some();
    let p1 = phase1::run(aig, &cuts, &idx, has_inv);
    phase2::run(aig, &cuts, lib, &p1)
}

pub fn rewrite_for_library(aig: &mut Aig, lib: &CellLib) -> bool {
    if !has_cube_cell(lib, 3, 3, 0) || !has_cube_cell(lib, 4, 1, 3) {
        return false;
    }

    let outputs: Vec<_> = aig.outputs().to_vec();
    let mut changed = false;
    for (output_index, (_, edge)) in outputs.iter().enumerate() {
        let Some(terms) = collect_and_cube_terms(aig, *edge) else {
            continue;
        };
        if terms.len() != 6 {
            continue;
        }

        let mut positive = Vec::new();
        let mut negative = Vec::new();
        for term in terms {
            if term.invert {
                negative.push(term);
            } else {
                positive.push(term);
            }
        }
        if positive.len() != 3 || negative.len() != 3 {
            continue;
        }

        let group_seed = positive.pop().unwrap();
        let mut group = group_seed;
        for term in negative {
            group = aig.and(group, term);
        }

        let mut rewritten = group;
        for term in positive {
            rewritten = aig.and(rewritten, term);
        }
        if rewritten != *edge {
            aig.set_output_edge(output_index, rewritten);
            changed = true;
        }
    }
    changed
}

fn has_cube_cell(lib: &CellLib, n_inputs: u32, positive: u32, negative: u32) -> bool {
    lib.cells.iter().any(|cell| {
        cell.n_inputs == n_inputs
            && cube_polarity_counts(cell.tt.0, n_inputs) == Some((positive, negative))
    })
}

fn cube_polarity_counts(tt: u64, n_inputs: u32) -> Option<(u32, u32)> {
    if tt.count_ones() != 1 {
        return None;
    }
    let minterm = tt.trailing_zeros();
    let mut positive = 0;
    let mut negative = 0;
    for i in 0..n_inputs {
        if (minterm >> i) & 1 == 1 {
            positive += 1;
        } else {
            negative += 1;
        }
    }
    Some((positive, negative))
}

fn collect_and_cube_terms(aig: &Aig, edge: Edge) -> Option<Vec<Edge>> {
    let mut terms = Vec::new();
    collect_and_cube_terms_inner(aig, edge, &mut terms).then_some(terms)
}

fn collect_and_cube_terms_inner(aig: &Aig, edge: Edge, terms: &mut Vec<Edge>) -> bool {
    match &aig.node(edge.node).kind {
        NodeKind::And2 { l, r } if !edge.invert => {
            collect_and_cube_terms_inner(aig, *l, terms)
                && collect_and_cube_terms_inner(aig, *r, terms)
        }
        NodeKind::PrimaryInput { .. } => {
            terms.push(edge);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::Tt64;
    use crate::frontend::library::{CellDecl, CellId};

    fn state_lib() -> CellLib {
        CellLib {
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
                    name: "INR4".into(),
                    inputs: vec!["A1".into(), "B1".into(), "B2".into(), "B3".into()],
                    output_pin: "ZN".into(),
                    tt: Tt64(0x0002),
                    n_inputs: 4,
                },
                CellDecl {
                    id: CellId(2),
                    name: "AN3".into(),
                    inputs: vec!["A1".into(), "A2".into(), "A3".into()],
                    output_pin: "Z".into(),
                    tt: Tt64(0x80),
                    n_inputs: 3,
                },
            ],
            notes: vec![],
        }
    }

    #[test]
    fn rewrite_exposes_inr4_friendly_cube_partition() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let c = aig.add_input("c");
        let d = aig.add_input("d");
        let e = aig.add_input("e");
        let f = aig.add_input("f");
        let ab = aig.and(a, b);
        let abc = aig.and(ab, c.inv());
        let abcd = aig.and(abc, d.inv());
        let abcde = aig.and(abcd, e);
        let y = aig.and(abcde, f.inv());
        aig.add_output("one_state", y);

        let lib = state_lib();
        assert_eq!(map_aig(&aig, &lib).unwrap().total_cells, 4);
        assert!(rewrite_for_library(&mut aig, &lib));
        assert_eq!(map_aig(&aig, &lib).unwrap().total_cells, 2);
    }
}
