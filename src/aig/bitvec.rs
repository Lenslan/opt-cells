//! Word-level building blocks lowered onto the 1-bit AIG.
//!
//! A word is a slice of edges, least significant bit first. Everything is unsigned.
//! Constant operands fold away through `Aig::and`, so e.g. comparing against a
//! literal produces a plain AND/OR chain rather than a full comparator.

use crate::aig::{Aig, Edge};

/// The low `width` bits of `value`.
pub fn constant(aig: &Aig, value: u64, width: usize) -> Vec<Edge> {
    (0..width)
        .map(|i| {
            if i < 64 && (value >> i) & 1 == 1 {
                aig.const1()
            } else {
                aig.const0()
            }
        })
        .collect()
}

/// Zero-extend or truncate to exactly `width` bits.
pub fn resize(aig: &Aig, mut bits: Vec<Edge>, width: usize) -> Vec<Edge> {
    bits.resize(width, aig.const0());
    bits
}

/// `sel ? if_true : if_false` for single bits.
pub fn mux(aig: &mut Aig, sel: Edge, if_true: Edge, if_false: Edge) -> Edge {
    if if_true == if_false {
        return if_true;
    }
    let true_path = aig.and(sel, if_true);
    let false_path = aig.and(sel.inv(), if_false);
    aig.or(true_path, false_path)
}

/// True iff any bit is set.
pub fn reduce_or(aig: &mut Aig, bits: &[Edge]) -> Edge {
    let mut acc = aig.const0();
    for &b in bits {
        acc = aig.or(acc, b);
    }
    acc
}

/// Ripple-carry `a + b + carry_in`, truncated to the operand width.
pub fn add(aig: &mut Aig, a: &[Edge], b: &[Edge], carry_in: Edge) -> Vec<Edge> {
    debug_assert_eq!(a.len(), b.len());
    let mut carry = carry_in;
    let mut out = Vec::with_capacity(a.len());
    for (&x, &y) in a.iter().zip(b) {
        let half = aig.xor(x, y);
        out.push(aig.xor(half, carry));
        let generate = aig.and(x, y);
        let propagate = aig.and(half, carry);
        carry = aig.or(generate, propagate);
    }
    out
}

/// `a - b` modulo `2^width` (two's complement: `a + ~b + 1`).
pub fn sub(aig: &mut Aig, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
    let nb: Vec<Edge> = b.iter().map(|e| e.inv()).collect();
    let one = aig.const1();
    add(aig, a, &nb, one)
}

/// Array multiplier: `a * b` truncated to the operand width.
pub fn mul(aig: &mut Aig, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
    debug_assert_eq!(a.len(), b.len());
    let w = a.len();
    let zero = aig.const0();
    let mut acc = vec![zero; w];
    for i in 0..w {
        if b[i] == zero {
            continue;
        }
        // Partial product `(a << i) & b[i]` only touches bits i..w.
        let partial: Vec<Edge> = (i..w).map(|j| aig.and(a[j - i], b[i])).collect();
        let upper = add(aig, &acc[i..], &partial, zero);
        acc[i..].copy_from_slice(&upper);
    }
    acc
}

/// `a == b`.
pub fn eq(aig: &mut Aig, a: &[Edge], b: &[Edge]) -> Edge {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = aig.const1();
    for (&x, &y) in a.iter().zip(b) {
        let differs = aig.xor(x, y);
        acc = aig.and(acc, differs.inv());
    }
    acc
}

/// Unsigned `a < b`, scanning from the least significant bit so that the most
/// significant differing bit decides.
pub fn ult(aig: &mut Aig, a: &[Edge], b: &[Edge]) -> Edge {
    debug_assert_eq!(a.len(), b.len());
    let mut lt = aig.const0();
    for (&x, &y) in a.iter().zip(b) {
        // lt' = (!x & y) | (lt & !(x & !y))
        let less_here = aig.and(x.inv(), y);
        let greater_here = aig.and(x, y.inv());
        let carried = aig.and(lt, greater_here.inv());
        lt = aig.or(less_here, carried);
    }
    lt
}

/// `table[index]` as a binary mux tree; an index beyond the table reads 0.
pub fn select(aig: &mut Aig, table: &[Edge], index: &[Edge]) -> Edge {
    let zero = aig.const0();
    if table.is_empty() {
        return zero;
    }
    // Number of index bits needed to address the last entry.
    let needed = (usize::BITS - (table.len() - 1).leading_zeros()) as usize;
    let k = needed.min(index.len());
    let mut level: Vec<Edge> = table.iter().copied().take(1usize << k).collect();
    level.resize(1usize << k, zero);
    for &sel in &index[..k] {
        level = level
            .chunks(2)
            .map(|pair| mux(aig, sel, pair[1], pair[0]))
            .collect();
    }
    let overflow = reduce_or(aig, &index[k..]);
    aig.and(level[0], overflow.inv())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::{NodeId, NodeKind};

    /// Evaluate `edges` as an unsigned integer with PI number `i` (creation order)
    /// taking bit `i` of `pattern`.
    fn eval(aig: &Aig, edges: &[Edge], pattern: u64) -> u64 {
        let mut value = vec![false; aig.num_nodes()];
        let mut pi = 0;
        for i in 0..aig.num_nodes() {
            value[i] = match &aig.node(NodeId(i as u32)).kind {
                NodeKind::Const0 => false,
                NodeKind::PrimaryInput { .. } => {
                    pi += 1;
                    (pattern >> (pi - 1)) & 1 == 1
                }
                NodeKind::And2 { l, r } => {
                    (value[l.node.0 as usize] ^ l.invert) && (value[r.node.0 as usize] ^ r.invert)
                }
            };
        }
        edges
            .iter()
            .enumerate()
            .map(|(i, e)| ((value[e.node.0 as usize] ^ e.invert) as u64) << i)
            .sum()
    }

    fn word(aig: &mut Aig, name: &str, w: usize) -> Vec<Edge> {
        (0..w)
            .map(|i| aig.add_input(&format!("{name}{i}")))
            .collect()
    }

    /// Exhaustively compare a two-operand builder against an integer model.
    fn check2(
        w: usize,
        build: impl Fn(&mut Aig, &[Edge], &[Edge]) -> Vec<Edge>,
        model: impl Fn(u64, u64) -> u64,
    ) {
        let mut aig = Aig::new();
        let a = word(&mut aig, "a", w);
        let b = word(&mut aig, "b", w);
        let out = build(&mut aig, &a, &b);
        let mask = (1u64 << w) - 1;
        for p in 0..(1u64 << (2 * w)) {
            let (x, y) = (p & mask, p >> w);
            assert_eq!(eval(&aig, &out, p), model(x, y), "a={x} b={y}");
        }
    }

    #[test]
    fn add_sub_mul_match_wrapping_integers() {
        check2(
            4,
            |g, a, b| {
                let z = g.const0();
                add(g, a, b, z)
            },
            |x, y| (x + y) & 15,
        );
        check2(4, sub, |x, y| x.wrapping_sub(y) & 15);
        check2(4, mul, |x, y| (x * y) & 15);
    }

    #[test]
    fn eq_and_ult_match_integers() {
        check2(4, |g, a, b| vec![eq(g, a, b)], |x, y| (x == y) as u64);
        check2(4, |g, a, b| vec![ult(g, a, b)], |x, y| (x < y) as u64);
    }

    #[test]
    fn compare_with_constant_folds_to_few_nodes() {
        let mut aig = Aig::new();
        let a = word(&mut aig, "a", 4);
        let before = aig.num_nodes();
        let ten = constant(&aig, 10, 4);
        let lt = ult(&mut aig, &a, &ten);
        for p in 0..16 {
            assert_eq!(eval(&aig, &[lt], p), (p < 10) as u64);
        }
        assert!(aig.num_nodes() - before <= 4, "constant operand must fold");
    }

    #[test]
    fn select_reads_table_and_zero_beyond_it() {
        let mut aig = Aig::new();
        let table = word(&mut aig, "t", 5);
        let index = word(&mut aig, "i", 4);
        let out = select(&mut aig, &table, &index);
        for p in 0..(1u64 << 9) {
            let (t, i) = (p & 31, p >> 5);
            let expect = if i < 5 { (t >> i) & 1 } else { 0 };
            assert_eq!(eval(&aig, &[out], p), expect, "t={t} i={i}");
        }
    }

    #[test]
    fn select_with_index_narrower_than_table() {
        let mut aig = Aig::new();
        let table = word(&mut aig, "t", 8);
        let index = word(&mut aig, "i", 2);
        let out = select(&mut aig, &table, &index);
        for p in 0..(1u64 << 10) {
            let (t, i) = (p & 255, p >> 8);
            assert_eq!(eval(&aig, &[out], p), (t >> i) & 1);
        }
    }

    #[test]
    fn mux_of_identical_arms_adds_no_node() {
        let mut aig = Aig::new();
        let s = aig.add_input("s");
        let a = aig.add_input("a");
        let n = aig.num_nodes();
        assert_eq!(mux(&mut aig, s, a, a), a);
        assert_eq!(aig.num_nodes(), n);
    }

    #[test]
    fn resize_and_constant() {
        let aig = Aig::new();
        let c = constant(&aig, 0b101, 3);
        assert_eq!(c, vec![aig.const1(), aig.const0(), aig.const1()]);
        assert_eq!(resize(&aig, c.clone(), 2).len(), 2);
        assert_eq!(resize(&aig, c, 5)[4], aig.const0());
    }
}
