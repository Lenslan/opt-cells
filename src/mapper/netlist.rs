use crate::aig::NodeId;
use crate::frontend::library::CellId;

#[derive(Debug, Clone)]
pub struct CellInstance {
    pub uid: u32,
    pub cell_id: CellId,
    pub aig_node: NodeId,
    /// This instance physically drives the `(aig_node, output_negated)` signal:
    /// its output value == natural_value(aig_node) XOR output_negated.
    pub output_negated: bool,
    /// One entry per cell PIN, indexed by pin position in the library declaration.
    pub pin_inputs: Vec<PinInput>,
}

#[derive(Debug, Clone)]
pub struct PinInput {
    pub leaf: NodeId,
    /// Which polarity of `leaf` this pin consumes. The `(leaf, leaf_negated)` signal
    /// is produced by a real cell instance (or is a positive primary input). It is
    /// never a free inversion applied at the pin.
    pub leaf_negated: bool,
}

#[derive(Debug, Clone)]
pub struct MappedNetlist {
    pub cells: Vec<CellInstance>,
    /// Output drivers: (primary output name as declared, AIG node providing it, invert flag).
    pub outputs: Vec<(String, NodeId, bool)>,
    pub total_cells: usize,
}

impl MappedNetlist {
    pub fn empty() -> Self {
        MappedNetlist {
            cells: vec![],
            outputs: vec![],
            total_cells: 0,
        }
    }
}
