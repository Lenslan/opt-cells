use std::collections::{HashMap, HashSet};

use crate::aig::{bitvec, Aig, Edge, NodeId};
use crate::error::{OptCellsError, Span};
use crate::frontend::ast::{BinOp, BitLiteral, BitSel, Decl, Expr, Program, Stmt, Width};

#[derive(Debug)]
pub struct ElabResult {
    pub aig: Aig,
    /// internal-name -> human label for report display (e.g. "state__3" -> "state[3]")
    pub display_names: HashMap<String, String>,
    /// AIG signal `(node, polarity)` -> user-requested net label for report/Tcl.
    pub net_aliases: HashMap<(NodeId, bool), String>,
}

/// Width of an unsized integer literal in a self-determined context (as in Verilog).
const UNSIZED_WIDTH: u32 = 32;

/// Static type of an expression. All values are unsigned bit vectors.
#[derive(Debug, Clone, Copy)]
struct Ty {
    /// Verilog self-determined width (an unsized literal counts as 32 bits).
    width: u32,
    /// Width ignoring the 32-bit padding of unsized literals: each literal counts
    /// only the bits its value needs. Used to size undeclared intermediates.
    natural: u32,
    /// True when an unsized literal takes part in `width`. Such expressions adapt
    /// to their context instead of triggering width-mismatch errors.
    flex: bool,
}

impl Ty {
    fn sized(width: u32) -> Ty {
        Ty {
            width,
            natural: width,
            flex: false,
        }
    }

    /// Result type of an operator whose operands are extended to a common width.
    fn join(self, other: Ty) -> Ty {
        Ty {
            width: self.width.max(other.width),
            natural: self.natural.max(other.natural),
            flex: self.flex || other.flex,
        }
    }
}

fn bit_key(name: &str, i: u32) -> String {
    format!("{}__{}", name, i)
}

fn bit_label(name: &str, i: u32) -> String {
    format!("{}[{}]", name, i)
}

struct Elaborator {
    aig: Aig,
    decl_widths: HashMap<String, Width>,
    inputs: HashSet<String>,
    signals: HashMap<String, Edge>,
    intermediates: HashSet<String>,
    /// Internal names of output bits that already have a driver.
    assigned_outputs: HashSet<String>,
    display: HashMap<String, String>,
    net_aliases: HashMap<(NodeId, bool), String>,
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
        self.inputs.insert(name.to_string());
        match w {
            Width::Bit => {
                let e = self.aig.add_input(name);
                self.signals.insert(name.to_string(), e);
                self.display.insert(name.to_string(), name.to_string());
            }
            Width::Vector { hi, lo } => {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                for i in lo..=hi {
                    let internal = bit_key(name, i);
                    let e = self.aig.add_input(&internal);
                    self.signals.insert(internal.clone(), e);
                    self.display.insert(internal, bit_label(name, i));
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

    /// Resolve `name` / `name[i]` / `name[hi:lo]` to the inclusive bit-index range
    /// it denotes, or `None` for a scalar signal.
    fn resolve_sel(
        &self,
        name: &str,
        sel: Option<BitSel>,
        span: Span,
    ) -> Result<Option<(u32, u32)>, OptCellsError> {
        let w = self
            .decl_widths
            .get(name)
            .ok_or_else(|| self.err(format!("undefined signal '{}'", name), span))?;
        match (*w, sel) {
            (Width::Bit, None) => Ok(None),
            (Width::Bit, Some(_)) => Err(self.err(
                format!("signal '{}' is single-bit; cannot index", name),
                span,
            )),
            (Width::Vector { hi, lo }, sel) => {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                match sel {
                    None => Ok(Some((lo, hi))),
                    Some(BitSel::Index(i)) => {
                        if i < lo || i > hi {
                            return Err(
                                self.err(format!("index {} out of range [{}:{}]", i, hi, lo), span)
                            );
                        }
                        Ok(Some((i, i)))
                    }
                    Some(BitSel::Range { hi: h2, lo: l2 }) => {
                        let (l2, h2) = (l2.min(h2), l2.max(h2));
                        if l2 < lo || h2 > hi {
                            return Err(self.err(
                                format!("slice [{}:{}] out of range [{}:{}]", h2, l2, hi, lo),
                                span,
                            ));
                        }
                        Ok(Some((l2, h2)))
                    }
                }
            }
        }
    }

    /// Current drivers of a signal reference, least significant bit first.
    fn ref_bits(
        &self,
        name: &str,
        sel: Option<BitSel>,
        span: Span,
    ) -> Result<Vec<Edge>, OptCellsError> {
        match self.resolve_sel(name, sel, span)? {
            None => {
                let e = self.signals.get(name).copied().ok_or_else(|| {
                    self.err(format!("signal '{}' used before assignment", name), span)
                })?;
                Ok(vec![e])
            }
            Some((lo, hi)) => (lo..=hi)
                .map(|i| {
                    self.signals.get(&bit_key(name, i)).copied().ok_or_else(|| {
                        self.err(
                            format!("signal '{}' used before assignment", bit_label(name, i)),
                            span,
                        )
                    })
                })
                .collect(),
        }
    }

    /// Operand width shared by both sides of a comparison. Two explicitly sized
    /// operands must agree; an operand containing an unsized literal adapts.
    fn compare_width(&self, lhs: &Expr, rhs: &Expr, span: Span) -> Result<u32, OptCellsError> {
        let l = self.ty(lhs)?;
        let r = self.ty(rhs)?;
        if !l.flex && !r.flex && l.width != r.width {
            return Err(self.err(
                format!(
                    "width mismatch: lhs is {} bits, rhs is {} bits",
                    l.width, r.width
                ),
                span,
            ));
        }
        Ok(l.width.max(r.width))
    }

    /// Type-check `e` (recursively) and return its type.
    fn ty(&self, e: &Expr) -> Result<Ty, OptCellsError> {
        match e {
            Expr::Lit {
                value: BitLiteral::Unsized(v),
                ..
            } => Ok(Ty {
                width: UNSIZED_WIDTH,
                natural: (u64::BITS - v.leading_zeros()).max(1),
                flex: true,
            }),
            Expr::Lit {
                value: BitLiteral::Vector { width, .. },
                span,
            } => {
                if *width == 0 {
                    return Err(self.err("literal width must be at least 1", *span));
                }
                Ok(Ty::sized(*width))
            }
            Expr::Ref { name, sel, span } => Ok(match self.resolve_sel(name, *sel, *span)? {
                None => Ty::sized(1),
                Some((lo, hi)) => Ty::sized(hi - lo + 1),
            }),
            Expr::DynIndex { name, index, span } => {
                if self.resolve_sel(name, None, *span)?.is_none() {
                    return Err(self.err(
                        format!("signal '{}' is single-bit; cannot index", name),
                        *span,
                    ));
                }
                self.ty(index)?;
                Ok(Ty::sized(1))
            }
            Expr::Not { inner, .. } => {
                self.ty(inner)?;
                Ok(Ty::sized(1))
            }
            Expr::BitNot { inner, .. } => self.ty(inner),
            Expr::And { lhs, rhs, .. } | Expr::Or { lhs, rhs, .. } | Expr::Xor { lhs, rhs, .. } => {
                Ok(self.ty(lhs)?.join(self.ty(rhs)?))
            }
            Expr::Binary {
                op: BinOp::Add | BinOp::Sub | BinOp::Mul,
                lhs,
                rhs,
                ..
            } => Ok(self.ty(lhs)?.join(self.ty(rhs)?)),
            Expr::Binary { lhs, rhs, span, .. }
            | Expr::Eq { lhs, rhs, span }
            | Expr::Neq { lhs, rhs, span } => {
                self.compare_width(lhs, rhs, *span)?;
                Ok(Ty::sized(1))
            }
            Expr::Mux {
                sel,
                if_true,
                if_false,
                span,
            } => {
                let s = self.ty(sel)?;
                if !s.flex && s.width != 1 {
                    return Err(self.err(
                        format!("mux selector must be 1 bit, got {} bits", s.width),
                        *span,
                    ));
                }
                let t = self.ty(if_true)?;
                let f = self.ty(if_false)?;
                if !t.flex && !f.flex && t.width != f.width {
                    return Err(self.err(
                        format!(
                            "mux branch width mismatch: {} bits vs {} bits",
                            t.width, f.width
                        ),
                        *span,
                    ));
                }
                Ok(t.join(f))
            }
        }
    }

    /// Evaluate `e` at its own (self-determined) width and reduce it to a truth
    /// value: 1 iff any bit is set.
    fn eval_bool(&mut self, e: &Expr) -> Result<Edge, OptCellsError> {
        let w = self.ty(e)?.width;
        let bits = self.eval(e, w)?;
        Ok(bitvec::reduce_or(&mut self.aig, &bits))
    }

    /// Build `e` as exactly `width` bits (LSB first). Operands of width-preserving
    /// operators are zero-extended or truncated to `width` first, which gives the
    /// same low bits as Verilog's context-determined evaluation.
    fn eval(&mut self, e: &Expr, width: u32) -> Result<Vec<Edge>, OptCellsError> {
        let w = width as usize;
        match e {
            Expr::Lit {
                value: BitLiteral::Unsized(v),
                ..
            } => Ok(bitvec::constant(&self.aig, *v, w)),
            Expr::Lit {
                value: BitLiteral::Vector { width: lw, value },
                ..
            } => {
                let sized = bitvec::constant(&self.aig, *value, *lw as usize);
                Ok(bitvec::resize(&self.aig, sized, w))
            }
            Expr::Ref { name, sel, span } => {
                let bits = self.ref_bits(name, *sel, *span)?;
                Ok(bitvec::resize(&self.aig, bits, w))
            }
            Expr::DynIndex { name, index, span } => {
                let (lo, _) = self
                    .resolve_sel(name, None, *span)?
                    .expect("ty() rejects dynamic index on scalars");
                let bits = self.ref_bits(name, None, *span)?;
                // table[j] = bit with declared index j; indices below `lo` read 0.
                let mut table = vec![self.aig.const0(); lo as usize];
                table.extend(bits);
                let index_width = self.ty(index)?.width;
                let index_bits = self.eval(index, index_width)?;
                let bit = bitvec::select(&mut self.aig, &table, &index_bits);
                Ok(bitvec::resize(&self.aig, vec![bit], w))
            }
            Expr::Not { inner, .. } => {
                let truth = self.eval_bool(inner)?;
                Ok(bitvec::resize(&self.aig, vec![truth.inv()], w))
            }
            Expr::BitNot { inner, .. } => {
                Ok(self.eval(inner, width)?.iter().map(|b| b.inv()).collect())
            }
            Expr::And { lhs, rhs, .. } | Expr::Or { lhs, rhs, .. } | Expr::Xor { lhs, rhs, .. } => {
                let l = self.eval(lhs, width)?;
                let r = self.eval(rhs, width)?;
                Ok(l.into_iter()
                    .zip(r)
                    .map(|(a, b)| match e {
                        Expr::And { .. } => self.aig.and(a, b),
                        Expr::Or { .. } => self.aig.or(a, b),
                        _ => self.aig.xor(a, b),
                    })
                    .collect())
            }
            Expr::Binary {
                op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul),
                lhs,
                rhs,
                ..
            } => {
                let l = self.eval(lhs, width)?;
                let r = self.eval(rhs, width)?;
                Ok(match op {
                    BinOp::Add => {
                        let zero = self.aig.const0();
                        bitvec::add(&mut self.aig, &l, &r, zero)
                    }
                    BinOp::Sub => bitvec::sub(&mut self.aig, &l, &r),
                    _ => bitvec::mul(&mut self.aig, &l, &r),
                })
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let cw = self.compare_width(lhs, rhs, *span)?;
                let l = self.eval(lhs, cw)?;
                let r = self.eval(rhs, cw)?;
                let bit = match op {
                    BinOp::Lt => bitvec::ult(&mut self.aig, &l, &r),
                    BinOp::Gt => bitvec::ult(&mut self.aig, &r, &l),
                    BinOp::Le => bitvec::ult(&mut self.aig, &r, &l).inv(),
                    _ => bitvec::ult(&mut self.aig, &l, &r).inv(),
                };
                Ok(bitvec::resize(&self.aig, vec![bit], w))
            }
            Expr::Eq { lhs, rhs, span } | Expr::Neq { lhs, rhs, span } => {
                let cw = self.compare_width(lhs, rhs, *span)?;
                let l = self.eval(lhs, cw)?;
                let r = self.eval(rhs, cw)?;
                let equal = bitvec::eq(&mut self.aig, &l, &r);
                let bit = if matches!(e, Expr::Neq { .. }) {
                    equal.inv()
                } else {
                    equal
                };
                Ok(bitvec::resize(&self.aig, vec![bit], w))
            }
            Expr::Mux {
                sel,
                if_true,
                if_false,
                ..
            } => {
                let s = self.eval_bool(sel)?;
                let t = self.eval(if_true, width)?;
                let f = self.eval(if_false, width)?;
                Ok(t.into_iter()
                    .zip(f)
                    .map(|(a, b)| bitvec::mux(&mut self.aig, s, a, b))
                    .collect())
            }
        }
    }

    fn record_net_alias(&mut self, label: String, edge: Edge) {
        if label.starts_with("eco_") {
            self.net_aliases.insert((edge.node, edge.invert), label);
        }
    }

    /// First assignment to an undeclared name defines an intermediate signal whose
    /// width follows the right-hand side.
    fn define_intermediate(&mut self, s: &Stmt, rhs_ty: Ty) -> Result<(), OptCellsError> {
        let name = &s.lhs.name;
        if s.lhs.sel.is_some() {
            return Err(self.err(
                format!("assignment to undeclared signal '{}'", name),
                s.lhs.span,
            ));
        }
        let width = rhs_ty.natural;
        let bits = self.eval(&s.rhs, width)?;
        self.intermediates.insert(name.clone());
        if width == 1 {
            self.decl_widths.insert(name.clone(), Width::Bit);
            self.signals.insert(name.clone(), bits[0]);
            self.record_net_alias(name.clone(), bits[0]);
        } else {
            self.decl_widths.insert(
                name.clone(),
                Width::Vector {
                    hi: width - 1,
                    lo: 0,
                },
            );
            for (i, bit) in bits.into_iter().enumerate() {
                let i = i as u32;
                self.signals.insert(bit_key(name, i), bit);
                self.record_net_alias(bit_label(name, i), bit);
            }
        }
        Ok(())
    }

    fn assign_stmt(&mut self, s: &Stmt) -> Result<(), OptCellsError> {
        let rhs_ty = self.ty(&s.rhs)?;
        let name = &s.lhs.name;
        let Some(target_w) = self.decl_widths.get(name).copied() else {
            return self.define_intermediate(s, rhs_ty);
        };
        if self.inputs.contains(name) {
            return Err(self.err(format!("cannot assign to input '{}'", name), s.lhs.span));
        }
        if self.intermediates.contains(name) {
            return Err(self.err(
                format!("duplicate assignment to intermediate signal '{}'", name),
                s.lhs.span,
            ));
        }

        // (internal name, display label) of every output bit driven, LSB first.
        let targets: Vec<(String, String)> = match (target_w, s.lhs.sel) {
            (Width::Bit, None) => vec![(name.clone(), name.clone())],
            (Width::Bit, Some(_)) => {
                return Err(self.err(
                    format!(
                        "signal '{}' is single-bit; cannot use bit index on lhs",
                        name
                    ),
                    s.lhs.span,
                ))
            }
            (Width::Vector { .. }, sel) => {
                let (lo, hi) = self
                    .resolve_sel(name, sel, s.lhs.span)?
                    .expect("vector signal resolves to a range");
                (lo..=hi)
                    .map(|i| (bit_key(name, i), bit_label(name, i)))
                    .collect()
            }
        };

        let lhs_width = targets.len() as u32;
        if !rhs_ty.flex && rhs_ty.width > lhs_width {
            return Err(self.err(
                format!(
                    "width mismatch: lhs is {} bits, rhs is {} bits",
                    lhs_width, rhs_ty.width
                ),
                s.span,
            ));
        }
        if let Some((_, label)) = targets
            .iter()
            .find(|(internal, _)| self.assigned_outputs.contains(internal))
        {
            return Err(self.err(
                format!("duplicate assignment to output '{}'", label),
                s.lhs.span,
            ));
        }

        let bits = self.eval(&s.rhs, lhs_width)?;
        for ((internal, label), bit) in targets.into_iter().zip(bits) {
            self.assigned_outputs.insert(internal.clone());
            self.signals.insert(internal.clone(), bit);
            self.aig.add_output(&internal, bit);
            self.display.insert(internal, label);
        }
        Ok(())
    }
}

pub fn elaborate(
    program: &Program,
    source_name: &str,
    source_text: &str,
) -> Result<ElabResult, OptCellsError> {
    let mut el = Elaborator {
        aig: Aig::new(),
        decl_widths: HashMap::new(),
        inputs: HashSet::new(),
        signals: HashMap::new(),
        intermediates: HashSet::new(),
        assigned_outputs: HashSet::new(),
        display: HashMap::new(),
        net_aliases: HashMap::new(),
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
    Ok(ElabResult {
        aig: el.aig,
        display_names: el.display,
        net_aliases: el.net_aliases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parser::program_parser;
    use chumsky::Parser;

    fn elab(src: &str) -> ElabResult {
        let prog = program_parser()
            .parse(src)
            .into_result()
            .expect("parse failed");
        elaborate(&prog, "test", src).expect("elaborate failed")
    }

    fn elab_err(src: &str) -> String {
        let prog = program_parser().parse(src).into_result().expect("parse ok");
        format!("{}", elaborate(&prog, "test", src).unwrap_err())
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
        assert!(elab_err("input a; output y; y = a & b;").contains("undefined"));
    }

    #[test]
    fn multi_bit_lhs_drives_every_bit_zero_extended() {
        let r = elab("input a; output y[3:0]; y = a;");
        let outs = r.aig.outputs();
        let names: Vec<&str> = outs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["y__0", "y__1", "y__2", "y__3"]);
        assert_eq!(outs[1].1, r.aig.const0());
        assert_eq!(r.display_names["y__3"], "y[3]");
    }

    #[test]
    fn decode_example_state_eq_0111() {
        let r = elab("input state[3:0]; output decoded; decoded = (state == 4'b0111);");
        assert_eq!(r.aig.outputs().len(), 1);
        let (name, _edge) = &r.aig.outputs()[0];
        assert_eq!(name, "decoded");
    }

    #[test]
    fn width_mismatch_in_eq() {
        assert!(elab_err("input s[3:0]; output y; y = (s == 3'b011);").contains("width mismatch"));
    }

    #[test]
    fn compare_with_constant_adds_no_padding_logic() {
        // The 32-bit zero extension of `cnt` and `10` must fold away completely.
        let r = elab("input cnt[3:0]; output y; y = cnt < 10;");
        assert!(r.aig.num_nodes() <= 5 + 4, "nodes = {}", r.aig.num_nodes());
    }

    #[test]
    fn intermediate_width_follows_operands_not_literal_padding() {
        // t is 4 bits (wraps); u is 3 bits because 5 needs three.
        let r = elab("input a[3:0]; output y, z; t = a + 1; u = 5; y = t[3]; z = u[2];");
        assert_eq!(r.aig.outputs().len(), 2);
        assert!(elab_err("input a[3:0]; output y; t = a + 1; y = t[4];").contains("out of range"));
    }

    #[test]
    fn eco_vector_intermediate_aliases_each_bit() {
        let r = elab("input a[1:0], b[1:0]; output y; eco_s = a ^ b; y = eco_s[0] & eco_s[1];");
        let mut labels: Vec<&str> = r.net_aliases.values().map(|s| s.as_str()).collect();
        labels.sort();
        assert_eq!(labels, ["eco_s[0]", "eco_s[1]"]);
    }
}
