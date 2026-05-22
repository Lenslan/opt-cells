#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    pub node: NodeId,
    pub invert: bool,
}

impl Edge {
    pub fn new(node: NodeId, invert: bool) -> Self {
        Edge { node, invert }
    }
    pub fn inv(self) -> Self {
        Edge { node: self.node, invert: !self.invert }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Constant 0 (constant 1 is achieved via inverted edge to const node).
    Const0,
    /// Primary input named `name`.
    PrimaryInput { name: String },
    /// Two-input AND with two edges.
    And2 { l: Edge, r: Edge },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AigNode {
    pub kind: NodeKind,
}
