use std::collections::HashMap;

use crate::aig::{Aig, Edge};
use crate::error::{OptCellsError, Span};
use crate::frontend::ast::{BitLiteral, BitSel, Decl, Expr, Program, Stmt, Width};

#[derive(Debug)]
pub struct ElabResult {
    pub aig: Aig,
    /// internal-name -> human label for report display (e.g. "state__3" -> "state[3]")
    pub display_names: HashMap<String, String>,
}

struct Elaborator {
    aig: Aig,
    decl_widths: HashMap<String, Width>,
    signals: HashMap<String, Edge>,
    display: HashMap<String, String>,
    source_name: String,
    source_text: String,
}

impl Elaborator {
    fn err(&self, message: impl Into<String>, span: Span) -> OptCellsError {
        OptCellsError::Elaborate {
            message: message.into(),
            span,
            source_name: self.source_name.clone(),
            source_text: self.source_text.clone(),
        }
    }

    fn declare_input(&mut self, name: &str, w: Width, span: Span) -> Result<(), OptCellsError> {
        if self.decl_widths.contains_key(name) {
            return Err(self.err(format!("duplicate input declaration: {}", name), span));
        }
        self.decl_widths.insert(name.to_string(), w);
        match w {
            Width::Bit => {
                let e = self.aig.add_input(name);
                self.signals.insert(name.to_string(), e);
                self.display.insert(name.to_string(), name.to_string());
            }
            Width::Vector { hi, lo } => {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                for i in lo..=hi {
                    let internal = format!("{}__{}", name, i);
                    let display = format!("{}[{}]", name, i);
                    let e = self.aig.add_input(&internal);
                    self.signals.insert(internal.clone(), e);
                    self.display.insert(internal, display);
                }
            }
        }
        Ok(())
    }

    fn declare_output(&mut self, name: &str, w: Width, span: Span) -> Result<(), OptCellsError> {
        if self.decl_widths.contains_key(name) {
            return Err(self.err(format!("duplicate declaration: {}", name), span));
        }
        self.decl_widths.insert(name.to_string(), w);
        Ok(())
    }

    fn lookup_signal(&self, name: &str, sel: Option<BitSel>, span: Span) -> Result<Edge, OptCellsError> {
        let w = self.decl_widths.get(name)
            .ok_or_else(|| self.err(format!("undefined signal '{}'", name), span))?;
        match (w, sel) {
            (Width::Bit, None) => self.signals.get(name).copied()
                .ok_or_else(|| self.err(format!("signal '{}' used before assignment", name), span)),
            (Width::Bit, Some(_)) => Err(self.err(format!("signal '{}' is single-bit; cannot index", name), span)),
            (Width::Vector { hi, lo }, Some(BitSel::Index(i))) => {
                let lo_val = (*lo).min(*hi);
                let hi_val = (*lo).max(*hi);
                if i < lo_val || i > hi_val {
                    return Err(self.err(format!("index {} out of range [{}:{}]", i, hi_val, lo_val), span));
                }
                let internal = format!("{}__{}", name, i);
                self.signals.get(&internal).copied()
                    .ok_or_else(|| self.err(format!("signal '{}' used before assignment", internal), span))
            }
            (Width::Vector { .. }, None) | (Width::Vector { .. }, Some(BitSel::Range { .. })) => {
                Err(self.err(format!("vector use of '{}' must appear in equality comparison only", name), span))
            }
        }
    }

    fn build_expr(&mut self, e: &Expr) -> Result<Edge, OptCellsError> {
        match e {
            Expr::Lit { value: BitLiteral::Single(b), .. } => Ok(if *b { self.aig.const1() } else { self.aig.const0() }),
            Expr::Lit { value: BitLiteral::Vector { .. }, span } => {
                Err(self.err("vector literal not allowed in scalar context (must appear in == or != only)", *span))
            }
            Expr::Ref { name, sel, span } => self.lookup_signal(name, *sel, *span),
            Expr::Not { inner, .. } => Ok(self.build_expr(inner)?.inv()),
            Expr::And { lhs, rhs, .. } => {
                let l = self.build_expr(lhs)?;
                let r = self.build_expr(rhs)?;
                Ok(self.aig.and(l, r))
            }
            Expr::Or { lhs, rhs, .. } => {
                let l = self.build_expr(lhs)?;
                let r = self.build_expr(rhs)?;
                Ok(self.aig.or(l, r))
            }
            Expr::Xor { lhs, rhs, .. } => {
                let l = self.build_expr(lhs)?;
                let r = self.build_expr(rhs)?;
                Ok(self.aig.xor(l, r))
            }
            Expr::Eq { lhs, rhs, span } | Expr::Neq { lhs, rhs, span } => {
                let neg = matches!(e, Expr::Neq { .. });
                let edge = self.build_eq(lhs, rhs, *span)?;
                Ok(if neg { edge.inv() } else { edge })
            }
        }
    }

    fn build_eq(&mut self, lhs: &Expr, rhs: &Expr, _span: Span) -> Result<Edge, OptCellsError> {
        // Single-bit version for Task 9. Task 10 will replace with vector-aware version.
        let l = self.build_expr(lhs)?;
        let r = self.build_expr(rhs)?;
        let xor = self.aig.xor(l, r);
        Ok(xor.inv())
    }

    fn assign_stmt(&mut self, s: &Stmt) -> Result<(), OptCellsError> {
        let rhs = self.build_expr(&s.rhs)?;
        let target_w = self.decl_widths.get(&s.lhs.name).copied()
            .ok_or_else(|| self.err(format!("assignment to undeclared signal '{}'", s.lhs.name), s.lhs.span))?;
        match (target_w, s.lhs.index) {
            (Width::Bit, None) => {
                self.signals.insert(s.lhs.name.clone(), rhs);
                self.aig.add_output(&s.lhs.name, rhs);
                self.display.entry(s.lhs.name.clone()).or_insert_with(|| s.lhs.name.clone());
                Ok(())
            }
            (Width::Bit, Some(_)) => Err(self.err(
                format!("signal '{}' is single-bit; cannot use bit index on lhs", s.lhs.name), s.lhs.span,
            )),
            (Width::Vector { hi, lo }, Some(i)) => {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                if i < lo || i > hi {
                    return Err(self.err(
                        format!("lhs index {} out of range [{}:{}]", i, hi, lo), s.lhs.span,
                    ));
                }
                let internal = format!("{}__{}", s.lhs.name, i);
                let display = format!("{}[{}]", s.lhs.name, i);
                self.signals.insert(internal.clone(), rhs);
                self.display.insert(internal.clone(), display);
                self.aig.add_output(&internal, rhs);
                Ok(())
            }
            (Width::Vector { .. }, None) => Err(self.err(
                "multi-bit assignment not supported; write one assignment per bit (e.g., y[0] = ..., y[1] = ...)",
                s.lhs.span,
            )),
        }
    }
}

pub fn elaborate(program: &Program, source_name: &str, source_text: &str) -> Result<ElabResult, OptCellsError> {
    let mut el = Elaborator {
        aig: Aig::new(),
        decl_widths: HashMap::new(),
        signals: HashMap::new(),
        display: HashMap::new(),
        source_name: source_name.into(),
        source_text: source_text.into(),
    };
    for d in &program.decls {
        match d {
            Decl::Input { name, width, span } => el.declare_input(name, *width, *span)?,
            Decl::Output { name, width, span } => el.declare_output(name, *width, *span)?,
        }
    }
    for s in &program.stmts {
        el.assign_stmt(s)?;
    }
    Ok(ElabResult { aig: el.aig, display_names: el.display })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parser::program_parser;
    use chumsky::Parser;

    fn elab(src: &str) -> ElabResult {
        let prog = program_parser().parse(src).into_result().expect("parse failed");
        elaborate(&prog, "test", src).expect("elaborate failed")
    }

    #[test]
    fn nand_example() {
        let r = elab("input a, b; output y; y = !(a & b);");
        // 1 const0 + 2 PIs + 1 AND = 4 nodes
        assert_eq!(r.aig.num_nodes(), 4);
        let (name, edge) = &r.aig.outputs()[0];
        assert_eq!(name, "y");
        assert!(edge.invert);
    }

    #[test]
    fn shared_subexpression() {
        let r = elab("input a, b; output o1, o2; o1 = a & b; o2 = (a & b) | a;");
        assert!(r.aig.num_nodes() < 7); // would be larger without share
    }

    #[test]
    fn undefined_signal_errors() {
        let prog = program_parser()
            .parse("input a; output y; y = a & b;")
            .into_result().expect("parse ok");
        let err = elaborate(&prog, "test", "input a; output y; y = a & b;").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("undefined"));
    }

    #[test]
    fn multi_bit_lhs_rejected() {
        let prog = program_parser()
            .parse("input a; output y[3:0]; y = a;")
            .into_result().expect("parse ok");
        let err = elaborate(&prog, "test", "input a; output y[3:0]; y = a;").unwrap_err();
        assert!(format!("{}", err).contains("multi-bit"));
    }
}
