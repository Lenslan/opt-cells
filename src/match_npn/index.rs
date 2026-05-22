use std::collections::HashMap;

use crate::frontend::library::{CellId, CellLib};
use crate::match_npn::canonical::npn_canonical;

#[derive(Debug, Clone, Copy)]
pub struct InputMapping {
    pub cell_id: CellId,
    pub n_inputs: u8,
    /// `pin_perm[i]` = which CELL pin receives the i-th LEAF of the cut (after negation per `input_negation`).
    pub pin_perm: [u8; 6],
    /// Bit i set => the i-th LEAF arrives at its cell pin inverted.
    pub input_negation: u8,
    /// True => the cell output must be inverted to produce the cut's target function.
    pub output_negation: bool,
}

#[derive(Debug, Default)]
pub struct NpnLibIndex {
    index: HashMap<(u64, u32), Vec<InputMapping>>,
}

impl NpnLibIndex {
    pub fn build(lib: &CellLib) -> Self {
        let mut idx = NpnLibIndex {
            index: HashMap::new(),
        };
        for cell in &lib.cells {
            if cell.n_inputs == 0 || cell.n_inputs > 6 {
                continue;
            }
            let info = npn_canonical(cell.tt.0, cell.n_inputs);
            let mut pin_perm = [0u8; 6];
            pin_perm[..cell.n_inputs as usize]
                .copy_from_slice(&info.input_perm[..cell.n_inputs as usize]);
            let mapping = InputMapping {
                cell_id: cell.id,
                n_inputs: cell.n_inputs as u8,
                pin_perm,
                input_negation: info.input_negation,
                output_negation: info.output_negation,
            };
            idx.index
                .entry((info.canonical_tt, cell.n_inputs))
                .or_default()
                .push(mapping);
        }
        idx
    }

    pub fn matches(&self, cut_tt: u64, k: u32) -> Vec<InputMapping> {
        let cut_info = npn_canonical(cut_tt, k);
        let stored = match self.index.get(&(cut_info.canonical_tt, k)) {
            Some(v) => v,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for stored_map in stored {
            // Composition reasoning:
            //   canonical position i := cell_pin stored_map.pin_perm[i], possibly negated by stored_map.input_negation bit i.
            //   canonical position i := cut leaf cut_info.input_perm[i], possibly negated by cut_info.input_negation bit i.
            // Therefore cell pin stored_map.pin_perm[i] := cut leaf cut_info.input_perm[i],
            //   negated iff (cut_info.input_negation bit i) XOR (stored_map.input_negation bit i).
            //
            // Output negation: canonical = cell_out XOR stored_map.output_negation = cut_out XOR cut_info.output_negation.
            // So we must invert cell output to get cut_out iff stored_map.output_negation XOR cut_info.output_negation.
            let mut pin_to_leaf: [u8; 6] = [0; 6];
            let mut leaf_neg: u8 = 0;
            for i in 0..k as usize {
                let cell_pin = stored_map.pin_perm[i] as usize;
                let leaf_idx = cut_info.input_perm[i];
                pin_to_leaf[cell_pin] = leaf_idx;
                let bit_cut = (cut_info.input_negation >> i) & 1;
                let bit_cell = (stored_map.input_negation >> i) & 1;
                if (bit_cut ^ bit_cell) == 1 {
                    leaf_neg |= 1u8 << leaf_idx;
                }
            }
            let mut leaf_to_pin: [u8; 6] = [0; 6];
            for (cell_pin, &leaf) in pin_to_leaf.iter().enumerate().take(k as usize) {
                leaf_to_pin[leaf as usize] = cell_pin as u8;
            }
            out.push(InputMapping {
                cell_id: stored_map.cell_id,
                n_inputs: stored_map.n_inputs,
                pin_perm: leaf_to_pin,
                input_negation: leaf_neg,
                output_negation: stored_map.output_negation ^ cut_info.output_negation,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::Tt64;
    use crate::frontend::library::{CellDecl, CellLib};

    fn cell(id: u32, name: &str, n_inputs: u32, tt: u64) -> CellDecl {
        let inputs: Vec<String> = (0..n_inputs).map(|i| format!("p{}", i)).collect();
        CellDecl {
            id: CellId(id),
            name: name.into(),
            inputs,
            output_pin: "y".into(),
            tt: Tt64(tt),
            n_inputs,
        }
    }

    #[test]
    fn nand2_matches_nand_cut() {
        let lib = CellLib {
            cells: vec![cell(0, "NAND2", 2, 0x7)],
            notes: vec![],
        };
        let idx = NpnLibIndex::build(&lib);
        let matches = idx.matches(0x7, 2);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].cell_id, CellId(0));
        assert!(!matches[0].output_negation);
        assert_eq!(matches[0].input_negation, 0);
    }

    #[test]
    fn and4_matches_and4_with_input0_inverted() {
        // Library has plain AND4 with TT 0x8000.
        let lib = CellLib {
            cells: vec![cell(0, "AND4", 4, 0x8000)],
            notes: vec![],
        };
        let idx = NpnLibIndex::build(&lib);
        // Cut function: !a & b & c & d, TT = 0x0080.
        let matches = idx.matches(0x0080, 4);
        assert!(
            !matches.is_empty(),
            "AND4 should match !a&b&c&d cut via NPN equivalence"
        );
        let m = &matches[0];
        assert!(!m.output_negation, "no output flip needed");
        // exactly one input must be inverted
        assert_eq!(m.input_negation.count_ones(), 1);
    }

    #[test]
    fn empty_index_no_match() {
        let lib = CellLib {
            cells: vec![],
            notes: vec![],
        };
        let idx = NpnLibIndex::build(&lib);
        assert!(idx.matches(0x8, 2).is_empty());
    }
}
