pub mod netlist;
pub mod phase1;
pub mod phase2;

pub use netlist::{CellInstance, MappedNetlist, PinInput};

use crate::aig::{enumerate_cuts, Aig};
use crate::error::OptCellsError;
use crate::frontend::library::CellLib;
use crate::match_npn::NpnLibIndex;

pub fn map_aig(aig: &Aig, lib: &CellLib) -> Result<MappedNetlist, OptCellsError> {
    let cuts = enumerate_cuts(aig);
    let idx = NpnLibIndex::build(lib);
    let p1 = phase1::run(aig, &cuts, &idx);
    phase2::run(aig, &cuts, lib, &p1)
}
