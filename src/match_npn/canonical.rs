use crate::aig::Tt64;

/// NPN canonical form description.
///
/// `canonical_tt` is the lex-smallest truth table (as u64) among all transformations
/// (input negation × input permutation × output negation) applied to the input TT.
/// The fields `input_perm`, `input_negation`, `output_negation` record the transform
/// that takes the original TT to its canonical: applying these to the input TT yields
/// `canonical_tt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpnInfo {
    pub canonical_tt: u64,
    pub k: u32,
    pub input_perm: [u8; 6], // input_perm[i] = original input index that ends up at canonical position i
    pub input_negation: u8,  // bit i set => input at canonical position i was negated
    pub output_negation: bool, // true => output was inverted to reach canonical
}

/// Apply (input_perm, input_negation, output_negation) to truth table `tt`
/// over k inputs, producing a new TT over the same k inputs in a permuted/negated view.
pub fn apply_transform(tt: u64, k: u32, perm: &[u8; 6], in_neg: u8, out_neg: bool) -> u64 {
    let n = 1u64 << k;
    let mut result: u64 = 0;
    let mask = Tt64::mask(k);
    for p in 0..n {
        // For canonical position i, the canonical pattern bit is bit i of p.
        // The corresponding original input is perm[i]; if in_neg has bit i set, flip the value.
        let mut orig_p: u64 = 0;
        for i in 0..k {
            let mut v = (p >> i) & 1;
            if (in_neg >> i) & 1 == 1 {
                v ^= 1;
            }
            orig_p |= v << (perm[i as usize] as u64);
        }
        let mut bit = (tt >> orig_p) & 1;
        if out_neg {
            bit ^= 1;
        }
        result |= bit << p;
    }
    result & mask
}

pub fn npn_canonical(tt: u64, k: u32) -> NpnInfo {
    assert!(k <= 6);
    let mask = Tt64::mask(k);
    let tt = tt & mask;

    let mut best_tt: u64 = u64::MAX;
    let mut best_perm: [u8; 6] = [0; 6];
    let mut best_neg: u8 = 0;
    let mut best_out: bool = false;

    let perms = permutations(k as usize);
    for p in &perms {
        let mut full_perm: [u8; 6] = [0; 6];
        full_perm[..k as usize].copy_from_slice(p);
        for in_neg in 0u8..(1 << k) {
            for &out_neg in &[false, true] {
                let candidate = apply_transform(tt, k, &full_perm, in_neg, out_neg);
                let candidate_masked = candidate & mask;
                if candidate_masked < best_tt {
                    best_tt = candidate_masked;
                    best_perm = full_perm;
                    best_neg = in_neg;
                    best_out = out_neg;
                }
            }
        }
    }
    NpnInfo {
        canonical_tt: best_tt,
        k,
        input_perm: best_perm,
        input_negation: best_neg,
        output_negation: best_out,
    }
}

pub fn transforms_to_canonical(tt: u64, k: u32, canonical_tt: u64) -> Vec<NpnInfo> {
    assert!(k <= 6);
    let mask = Tt64::mask(k);
    let tt = tt & mask;
    let canonical_tt = canonical_tt & mask;

    let mut out = Vec::new();
    let perms = permutations(k as usize);
    for p in &perms {
        let mut full_perm: [u8; 6] = [0; 6];
        full_perm[..k as usize].copy_from_slice(p);
        for in_neg in 0u8..(1 << k) {
            for &out_neg in &[false, true] {
                let candidate = apply_transform(tt, k, &full_perm, in_neg, out_neg) & mask;
                if candidate == canonical_tt {
                    out.push(NpnInfo {
                        canonical_tt,
                        k,
                        input_perm: full_perm,
                        input_negation: in_neg,
                        output_negation: out_neg,
                    });
                }
            }
        }
    }
    out
}

fn permutations(k: usize) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut current: Vec<u8> = (0..k as u8).collect();
    heap_permute(&mut current, k, &mut result);
    result
}

fn heap_permute(arr: &mut Vec<u8>, n: usize, out: &mut Vec<Vec<u8>>) {
    if n <= 1 {
        out.push(arr.clone());
        return;
    }
    for i in 0..n {
        heap_permute(arr, n - 1, out);
        let swap_with = if n % 2 == 0 { i } else { 0 };
        arr.swap(swap_with, n - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_of_const_zero() {
        let info = npn_canonical(0, 2);
        assert_eq!(info.canonical_tt, 0);
    }

    #[test]
    fn and2_and_nand2_same_npn_class() {
        // AND2 over k=2: pattern 11 → bit 3 set → 0x8
        // NAND2 over k=2: ~0x8 & 0xF = 0x7
        let a = npn_canonical(0x8, 2);
        let b = npn_canonical(0x7, 2);
        assert_eq!(a.canonical_tt, b.canonical_tt);
    }

    #[test]
    fn and4_and_and4_with_one_input_inverted_same_class() {
        // AND4: 0x8000 (only 1111)
        // AND4 with input 3 inverted: only 0111 → bit 7 set → 0x0080
        let a = npn_canonical(0x8000, 4);
        let b = npn_canonical(0x0080, 4);
        assert_eq!(
            a.canonical_tt, b.canonical_tt,
            "AND4 and AND4-with-one-inverted-input should share NPN class"
        );
    }

    #[test]
    fn canonical_is_idempotent() {
        let info = npn_canonical(0x6996, 4); // XOR4
        let info2 = npn_canonical(info.canonical_tt, 4);
        assert_eq!(info.canonical_tt, info2.canonical_tt);
    }

    #[test]
    fn transform_round_trip() {
        let tt = 0x8u64; // AND2
        let info = npn_canonical(tt, 2);
        let recomputed = apply_transform(
            tt,
            2,
            &info.input_perm,
            info.input_negation,
            info.output_negation,
        );
        assert_eq!(recomputed, info.canonical_tt);
    }
}
