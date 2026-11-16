use crate::error::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub decls: Vec<Decl>,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decl {
    Input {
        name: String,
        width: Width,
        span: Span,
    },
    Output {
        name: String,
        width: Width,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Bit,
    Vector { hi: u32, lo: u32 },
}

impl Width {
    pub fn size(&self) -> u32 {
        match self {
            Width::Bit => 1,
            Width::Vector { hi, lo } => hi.saturating_sub(*lo) + 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub lhs: Lvalue,
    pub rhs: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lvalue {
    pub name: String,
    /// `None` = whole signal, `Index` = one bit, `Range` = slice.
    pub sel: Option<BitSel>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Lit {
        value: BitLiteral,
        span: Span,
    },
    Ref {
        name: String,
        sel: Option<BitSel>,
        span: Span,
    },
    /// `name[index]` where `index` is not a plain integer: a run-time bit select.
    DynIndex {
        name: String,
        index: Box<Expr>,
        span: Span,
    },
    /// Logical negation `!x`: 1 bit, true iff `x` is zero.
    Not {
        inner: Box<Expr>,
        span: Span,
    },
    /// Bitwise complement `~x`.
    BitNot {
        inner: Box<Expr>,
        span: Span,
    },
    /// Arithmetic and relational operators (all unsigned).
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    And {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Or {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Xor {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Eq {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Neq {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Mux {
        sel: Box<Expr>,
        if_true: Box<Expr>,
        if_false: Box<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitLiteral {
    /// Plain decimal integer such as `0`, `1` or `10`. Like a Verilog unsized
    /// literal it is 32 bits wide in self-determined contexts.
    Unsized(u64),
    Vector {
        width: u32,
        value: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitSel {
    Index(u32),
    Range { hi: u32, lo: u32 },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Lit { span, .. }
            | Expr::Ref { span, .. }
            | Expr::DynIndex { span, .. }
            | Expr::Not { span, .. }
            | Expr::BitNot { span, .. }
            | Expr::Binary { span, .. }
            | Expr::And { span, .. }
            | Expr::Or { span, .. }
            | Expr::Xor { span, .. }
            | Expr::Eq { span, .. }
            | Expr::Neq { span, .. }
            | Expr::Mux { span, .. } => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_size_bit() {
        assert_eq!(Width::Bit.size(), 1);
    }

    #[test]
    fn width_size_vector() {
        assert_eq!(Width::Vector { hi: 3, lo: 0 }.size(), 4);
        assert_eq!(Width::Vector { hi: 7, lo: 0 }.size(), 8);
    }

    #[test]
    fn expr_span_round_trip() {
        let e = Expr::Lit {
            value: BitLiteral::Unsized(1),
            span: Span::new(1, 2),
        };
        assert_eq!(e.span(), Span::new(1, 2));
    }
}
