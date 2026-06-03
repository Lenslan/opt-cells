pub mod aig;
pub mod cli;
pub mod error;
pub mod frontend;
pub mod mapper;
pub mod match_npn;
pub mod report;

use std::collections::HashMap;

use crate::error::OptCellsError;

pub struct RunInputs {
    pub input_path: String,
    pub input_text: String,
    pub library_path: String,
}

pub fn run_pipeline(inputs: RunInputs) -> Result<(String, HashMap<String, String>), OptCellsError> {
    let prog = frontend::parser::parse_program(&inputs.input_text, &inputs.input_path)?;
    let elab = frontend::elaborate::elaborate(&prog, &inputs.input_path, &inputs.input_text)?;
    let lib = frontend::library::load_library(&inputs.library_path)?;
    let mut aig = elab.aig;
    let mut netlist = mapper::map_aig(&aig, &lib)?;
    let mut rewritten_aig = aig.clone();
    if mapper::rewrite_for_library(&mut rewritten_aig, &lib) {
        let rewritten_netlist = mapper::map_aig(&rewritten_aig, &lib)?;
        if rewritten_netlist.total_cells < netlist.total_cells {
            aig = rewritten_aig;
            netlist = rewritten_netlist;
        }
    }
    let report_string = report::render(&report::ReportInput {
        aig: &aig,
        lib: &lib,
        netlist: &netlist,
        input_path: &inputs.input_path,
        library_path: &inputs.library_path,
        display_names: &elab.display_names,
    });
    Ok((report_string, elab.display_names))
}
