use std::collections::HashMap;

use crate::aig::node::{AigNode, Edge, NodeId, NodeKind};

#[derive(Debug, Clone)]
pub struct Aig {
    nodes: Vec<AigNode>,
    /// `(l_node, l_invert, r_node, r_invert)` → existing AND2 NodeId (l <= r by NodeId order).
    and_cache: HashMap<(u32, bool, u32, bool), NodeId>,
    pi_cache: HashMap<String, NodeId>,
    /// Primary outputs in declaration order.
    outputs: Vec<(String, Edge)>,
}

impl Aig {
    pub fn new() -> Self {
        let mut aig = Aig {
            nodes: Vec::new(),
            and_cache: HashMap::new(),
            pi_cache: HashMap::new(),
            outputs: Vec::new(),
        };
        // index 0 reserved for Const0
        aig.nodes.push(AigNode { kind: NodeKind::Const0 });
        aig
    }

    pub fn const0(&self) -> Edge { Edge::new(NodeId(0), false) }
    pub fn const1(&self) -> Edge { Edge::new(NodeId(0), true) }

    pub fn add_input(&mut self, name: &str) -> Edge {
        if let Some(id) = self.pi_cache.get(name) {
            return Edge::new(*id, false);
        }
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(AigNode { kind: NodeKind::PrimaryInput { name: name.into() } });
        self.pi_cache.insert(name.into(), id);
        Edge::new(id, false)
    }

    /// Build AND of two edges; honors AIG constant simplification:
    /// `0 & x = 0`, `1 & x = x`, `x & x = x`, `x & !x = 0`.
    pub fn and(&mut self, a: Edge, b: Edge) -> Edge {
        if a.node == NodeId(0) {
            return if a.invert { b } else { self.const0() };
        }
        if b.node == NodeId(0) {
            return if b.invert { a } else { self.const0() };
        }
        if a == b { return a; }
        if a.node == b.node && a.invert != b.invert { return self.const0(); }

        // Normalize: smaller node id on the left.
        let (l, r) = if a.node <= b.node { (a, b) } else { (b, a) };
        let key = (l.node.0, l.invert, r.node.0, r.invert);
        if let Some(id) = self.and_cache.get(&key) {
            return Edge::new(*id, false);
        }
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(AigNode { kind: NodeKind::And2 { l, r } });
        self.and_cache.insert(key, id);
        Edge::new(id, false)
    }

    pub fn or(&mut self, a: Edge, b: Edge) -> Edge {
        let na = a.inv();
        let nb = b.inv();
        let nab = self.and(na, nb);
        nab.inv()
    }

    pub fn xor(&mut self, a: Edge, b: Edge) -> Edge {
        let a_nb = self.and(a, b.inv());
        let na_b = self.and(a.inv(), b);
        self.or(a_nb, na_b)
    }

    pub fn add_output(&mut self, name: &str, e: Edge) {
        self.outputs.push((name.into(), e));
    }

    pub fn nodes(&self) -> &[AigNode] { &self.nodes }
    pub fn node(&self, id: NodeId) -> &AigNode { &self.nodes[id.0 as usize] }
    pub fn num_nodes(&self) -> usize { self.nodes.len() }
    pub fn outputs(&self) -> &[(String, Edge)] { &self.outputs }
}

impl Default for Aig {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_node_at_index_zero() {
        let aig = Aig::new();
        assert_eq!(aig.num_nodes(), 1);
        assert!(matches!(aig.node(NodeId(0)).kind, NodeKind::Const0));
    }

    #[test]
    fn pi_dedup() {
        let mut aig = Aig::new();
        let a1 = aig.add_input("a");
        let a2 = aig.add_input("a");
        assert_eq!(a1, a2);
        assert_eq!(aig.num_nodes(), 2);
    }

    #[test]
    fn and_hash_consing() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab1 = aig.and(a, b);
        let ab2 = aig.and(a, b);
        let ba = aig.and(b, a);
        assert_eq!(ab1, ab2);
        assert_eq!(ab1, ba);
        assert_eq!(aig.num_nodes(), 4); // const0, a, b, a&b
    }

    #[test]
    fn and_with_const() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let c0 = aig.const0();
        let c1 = aig.const1();
        assert_eq!(aig.and(a, c0), c0);
        assert_eq!(aig.and(a, c1), a);
        assert_eq!(aig.and(a, a), a);
        assert_eq!(aig.and(a, a.inv()), c0);
    }

    #[test]
    fn or_via_de_morgan() {
        let mut aig = Aig::new();
        let a = aig.add_input("a");
        let b = aig.add_input("b");
        let ab_or = aig.or(a, b);
        // a|b = !(!a & !b); should produce a not-inverted AND2 node followed by an inverted edge
        assert!(ab_or.invert);
    }
}
