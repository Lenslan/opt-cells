use crate::aig::node::{NodeId, NodeKind};
use crate::aig::truth_table::Tt64;
use crate::aig::Aig;

pub const MAX_K: u32 = 6;
pub const CUTS_PER_NODE: usize = 8;

/// A cut: up to 6 leaf NodeIds plus the truth table of the function at the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    pub leaves: Vec<NodeId>, // sorted ascending; len <= MAX_K
    pub tt: Tt64,            // truth table over `leaves` (k = leaves.len())
}

impl Cut {
    pub fn k(&self) -> u32 {
        self.leaves.len() as u32
    }
    pub fn is_trivial(&self, node: NodeId) -> bool {
        self.leaves.len() == 1 && self.leaves[0] == node
    }
    /// True iff `self.leaves ⊆ other.leaves`.
    pub fn dominates(&self, other: &Cut) -> bool {
        if self.leaves.len() > other.leaves.len() {
            return false;
        }
        let mut j = 0;
        for &x in &self.leaves {
            while j < other.leaves.len() && other.leaves[j] < x {
                j += 1;
            }
            if j >= other.leaves.len() || other.leaves[j] != x {
                return false;
            }
        }
        true
    }
}

/// Compute all cuts for every node in the AIG, in node-id (topological) order.
/// Returns one Vec<Cut> per node id, pruned to CUTS_PER_NODE entries.
pub fn enumerate_cuts(aig: &Aig) -> Vec<Vec<Cut>> {
    let n = aig.num_nodes();
    let mut all: Vec<Vec<Cut>> = (0..n).map(|_| Vec::new()).collect();

    for idx in 0..n {
        let id = NodeId(idx as u32);
        let node = aig.node(id);
        match &node.kind {
            NodeKind::Const0 => {
                all[idx].push(Cut {
                    leaves: vec![id],
                    tt: Tt64::ZERO,
                });
            }
            NodeKind::PrimaryInput { .. } => {
                // trivial cut with TT = "input 0" over k=1, which is bit 1 set: 0b10 = 0x2
                all[idx].push(Cut {
                    leaves: vec![id],
                    tt: Tt64(0x2),
                });
            }
            NodeKind::And2 { l, r } => {
                let mut combos: Vec<Cut> = Vec::new();
                // Always include the trivial cut.
                combos.push(Cut {
                    leaves: vec![id],
                    tt: Tt64(0x2),
                });

                for cl in &all[l.node.0 as usize] {
                    for cr in &all[r.node.0 as usize] {
                        if let Some(merged) = merge_cuts(cl, cr, l.invert, r.invert) {
                            combos.push(merged);
                        }
                    }
                }

                let pruned = prune_cuts(combos, id);
                all[idx] = pruned;
            }
        }
    }
    all
}

fn merge_cuts(cl: &Cut, cr: &Cut, l_inv: bool, r_inv: bool) -> Option<Cut> {
    // Merge sorted leaf lists.
    let mut leaves: Vec<NodeId> = Vec::with_capacity(MAX_K as usize);
    let (mut i, mut j) = (0, 0);
    while i < cl.leaves.len() && j < cr.leaves.len() {
        match cl.leaves[i].cmp(&cr.leaves[j]) {
            std::cmp::Ordering::Less => {
                leaves.push(cl.leaves[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                leaves.push(cr.leaves[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                leaves.push(cl.leaves[i]);
                i += 1;
                j += 1;
            }
        }
        if leaves.len() > MAX_K as usize {
            return None;
        }
    }
    while i < cl.leaves.len() {
        leaves.push(cl.leaves[i]);
        i += 1;
        if leaves.len() > MAX_K as usize {
            return None;
        }
    }
    while j < cr.leaves.len() {
        leaves.push(cr.leaves[j]);
        j += 1;
        if leaves.len() > MAX_K as usize {
            return None;
        }
    }

    // Build truth table over merged leaves.
    let k = leaves.len() as u32;
    let l_tt = expand_tt(&cl.leaves, cl.tt, &leaves, k);
    let r_tt = expand_tt(&cr.leaves, cr.tt, &leaves, k);
    let l_eff = if l_inv { l_tt.not_in_k(k) } else { l_tt };
    let r_eff = if r_inv { r_tt.not_in_k(k) } else { r_tt };
    Some(Cut {
        leaves,
        tt: l_eff.and(r_eff),
    })
}

/// Expand a TT over `sub_leaves` (k' inputs) to a TT over `full_leaves` (k inputs).
/// Assumes sub_leaves is a subset of full_leaves; both sorted ascending.
fn expand_tt(sub_leaves: &[NodeId], sub_tt: Tt64, full_leaves: &[NodeId], k: u32) -> Tt64 {
    if sub_leaves == full_leaves {
        return sub_tt;
    }
    let mut tt: u64 = 0;
    let n = 1u64 << k;
    // Map sub_leaf index -> full_leaf index.
    let mut sub_idx_in_full: Vec<u32> = Vec::with_capacity(sub_leaves.len());
    let mut j = 0;
    for s in sub_leaves {
        while full_leaves[j] != *s {
            j += 1;
        }
        sub_idx_in_full.push(j as u32);
        j += 1;
    }
    for p in 0..n {
        let mut p_sub: u64 = 0;
        for (i_sub, &i_full) in sub_idx_in_full.iter().enumerate() {
            if (p >> i_full) & 1 == 1 {
                p_sub |= 1u64 << i_sub;
            }
        }
        if (sub_tt.0 >> p_sub) & 1 == 1 {
            tt |= 1u64 << p;
        }
    }
    Tt64(tt & Tt64::mask(k))
}

fn prune_cuts(mut cuts: Vec<Cut>, root: NodeId) -> Vec<Cut> {
    // Deduplicate by leaf set: keep first occurrence.
    cuts.sort_by(|a, b| a.leaves.cmp(&b.leaves));
    cuts.dedup_by(|a, b| a.leaves == b.leaves);

    // Drop dominated cuts (keep dominators).
    let n = cuts.len();
    let mut keep = vec![true; n];
    for i in 0..n {
        if !keep[i] {
            continue;
        }
        for j in 0..n {
            if i == j || !keep[j] {
                continue;
            }
            if cuts[j].dominates(&cuts[i]) && cuts[j].leaves.len() < cuts[i].leaves.len() {
                keep[i] = false;
                break;
            }
        }
    }
    let mut filtered: Vec<Cut> = cuts
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, c)| c)
        .collect();

    // Ensure trivial cut is present (single leaf = root).
    if !filtered.iter().any(|c| c.is_trivial(root)) {
        filtered.push(Cut {
            leaves: vec![root],
            tt: Tt64(0x2),
        });
    }

    // Sort by leaf count, then by tt as tiebreak.
    filtered.sort_by(|a, b| a.leaves.len().cmp(&b.leaves.len()).then(a.tt.cmp(&b.tt)));
    filtered.truncate(CUTS_PER_NODE);
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_cuts() {
        let mut aig = Aig::new();
        let _a = aig.add_input("a");
        let cuts = enumerate_cuts(&aig);
        assert_eq!(cuts[1].len(), 1);
        assert_eq!(cuts[1][0].leaves.len(), 1);
    }

    #[test]
    fn and_node_has_trivial_and_full_cut() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab = aig.and(a, b);
        let cuts = enumerate_cuts(&aig);
        let cs = &cuts[ab.node.0 as usize];
        assert!(cs.iter().any(|c| c.leaves.len() == 1));
        let full = cs
            .iter()
            .find(|c| c.leaves.len() == 2)
            .expect("two-leaf cut missing");
        assert_eq!(full.leaves, vec![a.node, b.node]);
        // tt of {a, b} AND = 0x8 (var0 AND var1 over k=2)
        assert_eq!(full.tt, Tt64(0x8));
    }

    #[test]
    fn nested_and_collects_all_three_leaves() {
        // y = (a & b) & c → cuts at y should include {a, b, c}, tt = a&b&c
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let c = aig.add_input("c");
        let ab = aig.and(a, b);
        let abc = aig.and(ab, c);
        let cuts = enumerate_cuts(&aig);
        let cs = &cuts[abc.node.0 as usize];
        let three = cs
            .iter()
            .find(|c| c.leaves.len() == 3)
            .expect("three-leaf cut missing");
        // For 3 inputs sorted (a, b, c) at positions 0, 1, 2, the AND TT = 0x80
        assert_eq!(three.tt, Tt64(0x80));
    }

    #[test]
    fn cut_dominates_subset() {
        let c1 = Cut {
            leaves: vec![NodeId(1), NodeId(2)],
            tt: Tt64(0),
        };
        let c2 = Cut {
            leaves: vec![NodeId(1), NodeId(2), NodeId(3)],
            tt: Tt64(0),
        };
        assert!(c1.dominates(&c2));
        assert!(!c2.dominates(&c1));
    }
}
