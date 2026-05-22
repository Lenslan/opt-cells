use std::collections::HashMap;
use serde::Deserialize;

use crate::aig::{Aig, Tt64};
use crate::error::OptCellsError;
use crate::frontend::parser::expr_parser;
use crate::frontend::ast::{Expr, BitLiteral};
use chumsky::Parser;

#[derive(Debug, Deserialize)]
struct LibraryFile {
    cell: Vec<CellRaw>,
}

#[derive(Debug, Deserialize)]
struct CellRaw {
    name: String,
    inputs: Vec<String>,
    output: String,
    function: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId(pub u32);

#[derive(Debug, Clone)]
pub struct CellDecl {
    pub id: CellId,
    pub name: String,
    pub inputs: Vec<String>,
    pub output_pin: String,
    pub tt: Tt64,
    pub n_inputs: u32,
}

#[derive(Debug, Clone)]
pub struct CellLib {
    pub cells: Vec<CellDecl>,
    pub notes: Vec<String>,
}

pub fn load_library(path: &str) -> Result<CellLib, OptCellsError> {
    let text = std::fs::read_to_string(path)?;
    let parsed: LibraryFile = toml::from_str(&text).map_err(|e| OptCellsError::ParseLibrary {
        message: format!("TOML parse error: {}", e),
        source_name: path.into(),
    })?;
    if parsed.cell.is_empty() {
        return Err(OptCellsError::ParseLibrary {
            message: "library contains no cells".into(),
            source_name: path.into(),
        });
    }
    let mut cells = Vec::new();
    let mut notes = Vec::new();
    let mut name_seen: HashMap<String, ()> = HashMap::new();
    for (i, raw) in parsed.cell.into_iter().enumerate() {
        if name_seen.insert(raw.name.clone(), ()).is_some() {
            return Err(OptCellsError::ParseLibrary {
                message: format!("duplicate cell name '{}'", raw.name),
                source_name: path.into(),
            });
        }
        let n_inputs = raw.inputs.len() as u32;
        if n_inputs == 0 || n_inputs > 6 {
            notes.push(format!("cell '{}' has {} inputs (outside [1,6]); will not be matchable", raw.name, n_inputs));
            cells.push(CellDecl {
                id: CellId(i as u32),
                name: raw.name,
                inputs: raw.inputs,
                output_pin: raw.output,
                tt: Tt64::ZERO,
                n_inputs,
            });
            continue;
        }
        let expr = expr_parser()
            .parse(raw.function.as_str())
            .into_result()
            .map_err(|errs| {
                let m = errs.into_iter().next().map(|e| e.reason().to_string()).unwrap_or_default();
                OptCellsError::ParseLibrary {
                    message: format!("cell '{}' function parse error: {}", raw.name, m),
                    source_name: path.into(),
                }
            })?;
        let tt = compute_cell_tt(&expr, &raw.inputs).map_err(|m| OptCellsError::ParseLibrary {
            message: format!("cell '{}': {}", raw.name, m),
            source_name: path.into(),
        })?;
        cells.push(CellDecl {
            id: CellId(i as u32),
            name: raw.name,
            inputs: raw.inputs,
            output_pin: raw.output,
            tt,
            n_inputs,
        });
    }
    Ok(CellLib { cells, notes })
}

fn compute_cell_tt(expr: &Expr, inputs: &[String]) -> Result<Tt64, String> {
    let k = inputs.len() as u32;
    let mut aig = Aig::new();
    let mut env: HashMap<&str, crate::aig::Edge> = HashMap::new();
    for name in inputs {
        env.insert(name.as_str(), aig.add_input(name));
    }
    let edge = eval_expr_to_aig(expr, &mut aig, &env, inputs)?;
    Ok(tt_from_aig(&aig, edge, k))
}

fn eval_expr_to_aig<'a>(
    expr: &'a Expr,
    aig: &mut Aig,
    env: &HashMap<&'a str, crate::aig::Edge>,
    _inputs: &[String],
) -> Result<crate::aig::Edge, String> {
    use crate::frontend::ast::Expr::*;
    match expr {
        Lit { value: BitLiteral::Single(b), .. } => Ok(if *b { aig.const1() } else { aig.const0() }),
        Lit { value: BitLiteral::Vector { .. }, .. } => Err("vector literal not allowed in cell function".into()),
        Ref { name, sel, .. } => {
            if sel.is_some() {
                return Err(format!("bit-select not allowed in cell function on '{}'", name));
            }
            env.get(name.as_str()).copied()
                .ok_or_else(|| format!("function references '{}' which is not in inputs list", name))
        }
        Not { inner, .. } => Ok(eval_expr_to_aig(inner, aig, env, _inputs)?.inv()),
        And { lhs, rhs, .. } => {
            let l = eval_expr_to_aig(lhs, aig, env, _inputs)?;
            let r = eval_expr_to_aig(rhs, aig, env, _inputs)?;
            Ok(aig.and(l, r))
        }
        Or { lhs, rhs, .. } => {
            let l = eval_expr_to_aig(lhs, aig, env, _inputs)?;
            let r = eval_expr_to_aig(rhs, aig, env, _inputs)?;
            Ok(aig.or(l, r))
        }
        Xor { lhs, rhs, .. } => {
            let l = eval_expr_to_aig(lhs, aig, env, _inputs)?;
            let r = eval_expr_to_aig(rhs, aig, env, _inputs)?;
            Ok(aig.xor(l, r))
        }
        Eq { lhs, rhs, .. } | Neq { lhs, rhs, .. } => {
            let l = eval_expr_to_aig(lhs, aig, env, _inputs)?;
            let r = eval_expr_to_aig(rhs, aig, env, _inputs)?;
            let x = aig.xor(l, r);
            Ok(if matches!(expr, Neq { .. }) { x } else { x.inv() })
        }
    }
}

fn tt_from_aig(aig: &Aig, output_edge: crate::aig::Edge, k: u32) -> Tt64 {
    use crate::aig::NodeKind;
    let n = aig.num_nodes();
    let mut pi_ids: Vec<crate::aig::NodeId> = Vec::new();
    for i in 0..n {
        let id = crate::aig::NodeId(i as u32);
        if matches!(aig.node(id).kind, NodeKind::PrimaryInput { .. }) {
            pi_ids.push(id);
        }
    }
    let mut tt = vec![Tt64::ZERO; n];
    let mask = Tt64::mask(k);
    for i in 0..n {
        let id = crate::aig::NodeId(i as u32);
        match &aig.node(id).kind {
            NodeKind::Const0 => tt[i] = Tt64::ZERO,
            NodeKind::PrimaryInput { .. } => {
                let var_idx = pi_ids.iter().position(|x| *x == id).unwrap() as u32;
                tt[i] = Tt64::var(var_idx, k);
            }
            NodeKind::And2 { l, r } => {
                let l_tt = tt[l.node.0 as usize];
                let r_tt = tt[r.node.0 as usize];
                let l_eff = if l.invert { l_tt.not_in_k(k) } else { l_tt };
                let r_eff = if r.invert { r_tt.not_in_k(k) } else { r_tt };
                tt[i] = Tt64(l_eff.0 & r_eff.0 & mask);
            }
        }
    }
    let out_tt = tt[output_edge.node.0 as usize];
    if output_edge.invert { out_tt.not_in_k(k) } else { out_tt }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(text: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(text.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_basic_library() {
        let toml = r#"
[[cell]]
name = "INV"
inputs = ["a"]
output = "y"
function = "!a"

[[cell]]
name = "NAND2"
inputs = ["a", "b"]
output = "y"
function = "!(a & b)"
"#;
        let f = write_tmp(toml);
        let lib = load_library(f.path().to_str().unwrap()).unwrap();
        assert_eq!(lib.cells.len(), 2);
        assert_eq!(lib.cells[0].name, "INV");
        // INV TT: var 0 inverted over k=1: ~0x2 & 0x3 = 0x1
        assert_eq!(lib.cells[0].tt, Tt64(0x1));
        // NAND2 TT: !(a & b) over k=2: ~0x8 & 0xF = 0x7
        assert_eq!(lib.cells[1].tt, Tt64(0x7));
    }

    #[test]
    fn rejects_duplicate_cell() {
        let toml = r#"
[[cell]]
name = "X"
inputs = ["a"]
output = "y"
function = "a"

[[cell]]
name = "X"
inputs = ["a"]
output = "y"
function = "!a"
"#;
        let f = write_tmp(toml);
        let err = load_library(f.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{}", err).contains("duplicate"));
    }

    #[test]
    fn rejects_function_unknown_var() {
        let toml = r#"
[[cell]]
name = "X"
inputs = ["a"]
output = "y"
function = "a & b"
"#;
        let f = write_tmp(toml);
        let err = load_library(f.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{}", err).contains("not in inputs"));
    }

    #[test]
    fn and4_truth_table() {
        let toml = r#"
[[cell]]
name = "AND4"
inputs = ["a", "b", "c", "d"]
output = "y"
function = "a & b & c & d"
"#;
        let f = write_tmp(toml);
        let lib = load_library(f.path().to_str().unwrap()).unwrap();
        // AND of 4 vars over k=4: only pattern 1111 (= 15) → bit 15 set → 0x8000
        assert_eq!(lib.cells[0].tt, Tt64(0x8000));
    }
}
