use crate::error::Span as MySpan;
use crate::frontend::ast::*;
use chumsky::prelude::*;

fn to_span(r: SimpleSpan) -> MySpan {
    MySpan::new(r.start, r.end)
}

pub fn expr_parser<'src>(
) -> impl Parser<'src, &'src str, Expr, extra::Err<Rich<'src, char>>> + Clone {
    recursive(|expr| {
        // unsigned integer (decimal)
        let uint = text::int(10).map(|s: &str| s.parse::<u32>().unwrap());

        // vector literal: <width>'<radix><digits>
        let vec_lit = text::int(10)
            .then_ignore(just('\''))
            .then(one_of("bhd"))
            .then(
                any()
                    .filter(|c: &char| c.is_ascii_alphanumeric())
                    .repeated()
                    .at_least(1)
                    .collect::<String>(),
            )
            .map_with(|((width_s, radix), digits): ((&str, char), String), e| {
                let width: u32 = width_s.parse().unwrap();
                let r = match radix {
                    'b' => 2,
                    'h' => 16,
                    'd' => 10,
                    _ => unreachable!(),
                };
                let value = u64::from_str_radix(&digits, r).unwrap();
                Expr::Lit {
                    value: BitLiteral::Vector { width, value },
                    span: to_span(e.span()),
                }
            });

        // unsized decimal integer (0, 1, 10, ...): only when not followed by '
        let int_lit = text::int(10)
            .then_ignore(just('\'').not().rewind())
            .try_map(|s: &str, span| {
                s.parse::<u64>()
                    .ok()
                    .filter(|v| *v <= u32::MAX as u64)
                    .map(|v| Expr::Lit {
                        value: BitLiteral::Unsized(v),
                        span: to_span(span),
                    })
                    .ok_or_else(|| {
                        Rich::custom(
                            span,
                            "unsized integer literal does not fit in 32 bits; use a sized literal",
                        )
                    })
            });

        // select: [hi:lo] with integer bounds, or [expr]
        enum Sel {
            Range(u32, u32),
            Expr(Expr),
        }
        let range_sel = uint
            .then_ignore(just(':').padded())
            .then(uint)
            .delimited_by(just('[').padded(), just(']').padded())
            .map(|(hi, lo)| Sel::Range(hi, lo));
        let expr_sel = expr
            .clone()
            .delimited_by(just('[').padded(), just(']').padded())
            .map(Sel::Expr);

        // identifier reference; a plain integer index stays a static bit select
        let signal_ref = text::ident()
            .then(choice((range_sel, expr_sel)).or_not())
            .map_with(|(name, sel): (&str, Option<Sel>), e| {
                let name = name.to_string();
                let span = to_span(e.span());
                let sel = match sel {
                    None => None,
                    Some(Sel::Range(hi, lo)) => Some(BitSel::Range { hi, lo }),
                    Some(Sel::Expr(Expr::Lit {
                        value: BitLiteral::Unsized(i),
                        ..
                    })) => Some(BitSel::Index(i as u32)),
                    Some(Sel::Expr(index)) => {
                        return Expr::DynIndex {
                            name,
                            index: Box::new(index),
                            span,
                        }
                    }
                };
                Expr::Ref { name, sel, span }
            });

        let parens = expr
            .clone()
            .delimited_by(just('(').padded(), just(')').padded());

        let primary = choice((parens, vec_lit, int_lit, signal_ref)).padded();

        // unary: ! (logical) or ~ (bitwise)  (right-associative via recursive)
        let unary = recursive(|unary| {
            choice((
                one_of("!~")
                    .padded()
                    .then(unary)
                    .map_with(|(op, inner), e| {
                        let inner = Box::new(inner);
                        let span = to_span(e.span());
                        if op == '!' {
                            Expr::Not { inner, span }
                        } else {
                            Expr::BitNot { inner, span }
                        }
                    }),
                primary.clone(),
            ))
        });

        let binary = |lhs: Expr, (op, rhs): (BinOp, Expr), span: MySpan| Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };

        // mul_expr: unary ("*" unary)*
        let mul_op = just('*').to(BinOp::Mul).padded();
        let mul_expr = unary.clone().foldl_with(
            mul_op.then(unary.clone()).repeated(),
            move |lhs, rest, e| binary(lhs, rest, to_span(e.span())),
        );

        // add_expr: mul_expr (("+" | "-") mul_expr)*
        let add_op = choice((just('+').to(BinOp::Add), just('-').to(BinOp::Sub))).padded();
        let add_expr = mul_expr.clone().foldl_with(
            add_op.then(mul_expr.clone()).repeated(),
            move |lhs, rest, e| binary(lhs, rest, to_span(e.span())),
        );

        // rel_expr: add_expr (("<=" | ">=" | "<" | ">") add_expr)*
        let rel_op = choice((
            just("<=").to(BinOp::Le),
            just(">=").to(BinOp::Ge),
            just("<").to(BinOp::Lt),
            just(">").to(BinOp::Gt),
        ))
        .padded();
        let rel_expr = add_expr.clone().foldl_with(
            rel_op.then(add_expr.clone()).repeated(),
            move |lhs, rest, e| binary(lhs, rest, to_span(e.span())),
        );

        // eq_expr: rel_expr (("==" | "!=") rel_expr)?
        let eq_op = choice((just("==").to(0u8), just("!=").to(1u8))).padded();

        let eq_expr = rel_expr
            .clone()
            .then(eq_op.then(rel_expr.clone()).or_not())
            .map_with(|(lhs, rest), e| match rest {
                None => lhs,
                Some((op, rhs)) => {
                    let span = to_span(e.span());
                    if op == 0 {
                        Expr::Eq {
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                            span,
                        }
                    } else {
                        Expr::Neq {
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                            span,
                        }
                    }
                }
            });

        // and_expr: eq_expr ("&" eq_expr)*  (left-associative)
        let and_expr = eq_expr.clone().foldl_with(
            just('&').padded().ignore_then(eq_expr.clone()).repeated(),
            |lhs, rhs, e| Expr::And {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span: to_span(e.span()),
            },
        );

        // xor_expr: and_expr ("^" and_expr)*
        let xor_expr = and_expr.clone().foldl_with(
            just('^').padded().ignore_then(and_expr.clone()).repeated(),
            |lhs, rhs, e| Expr::Xor {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span: to_span(e.span()),
            },
        );

        // or_expr: xor_expr ("|" xor_expr)*
        let or_expr = xor_expr.clone().foldl_with(
            just('|').padded().ignore_then(xor_expr.clone()).repeated(),
            |lhs, rhs, e| Expr::Or {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span: to_span(e.span()),
            },
        );

        // conditional expression: cond ? if_true : if_false
        // This is the lowest-precedence expression form and is right-associative
        // because each branch is parsed as a full expression.
        or_expr
            .clone()
            .then(
                just('?')
                    .padded()
                    .ignore_then(expr.clone())
                    .then_ignore(just(':').padded())
                    .then(expr.clone())
                    .or_not(),
            )
            .map_with(|(sel, arms), e| match arms {
                Some((if_true, if_false)) => Expr::Mux {
                    sel: Box::new(sel),
                    if_true: Box::new(if_true),
                    if_false: Box::new(if_false),
                    span: to_span(e.span()),
                },
                None => sel,
            })
            .padded()
    })
}

pub fn decl_parser<'src>(
) -> impl Parser<'src, &'src str, Vec<Decl>, extra::Err<Rich<'src, char>>> + Clone {
    let ident = text::ident().padded().map(|s: &str| s.to_string());
    let uint = text::int(10)
        .padded()
        .map(|s: &str| s.parse::<u32>().unwrap());

    let width = uint
        .then_ignore(just(':').padded())
        .then(uint)
        .delimited_by(just('[').padded(), just(']').padded())
        .map(|(hi, lo)| Width::Vector { hi, lo })
        .or_not()
        .map(|w| w.unwrap_or(Width::Bit));

    let name_with_width = ident.then(width);

    // Reusable input declaration body
    let one_input = text::keyword("input")
        .padded()
        .ignore_then(
            name_with_width
                .map_with(|(name, w), e| Decl::Input {
                    name,
                    width: w,
                    span: to_span(e.span()),
                })
                .separated_by(just(',').padded())
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(';').padded());

    // Reusable output declaration body
    let one_output = text::keyword("output")
        .padded()
        .ignore_then(
            name_with_width
                .map_with(|(name, w), e| Decl::Output {
                    name,
                    width: w,
                    span: to_span(e.span()),
                })
                .separated_by(just(',').padded())
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(';').padded());

    choice((one_input, one_output))
        .repeated()
        .collect::<Vec<Vec<Decl>>>()
        .map(|v| v.into_iter().flatten().collect())
}

pub fn stmt_parser<'src>(
) -> impl Parser<'src, &'src str, Stmt, extra::Err<Rich<'src, char>>> + Clone {
    let ident = text::ident().padded().map(|s: &str| s.to_string());
    let uint = text::int(10)
        .padded()
        .map(|s: &str| s.parse::<u32>().unwrap());

    // target: name, name[i] or name[hi:lo]
    let sel = uint
        .then(just(':').padded().ignore_then(uint).or_not())
        .delimited_by(just('[').padded(), just(']').padded())
        .map(|(a, b)| match b {
            Some(lo) => BitSel::Range { hi: a, lo },
            None => BitSel::Index(a),
        });

    let lvalue = ident.then(sel.or_not()).map_with(|(name, sel), e| Lvalue {
        name,
        sel,
        span: to_span(e.span()),
    });

    lvalue
        .then_ignore(just('=').padded())
        .then(expr_parser())
        .then_ignore(just(';').padded())
        .map_with(|(lhs, rhs), e| Stmt {
            lhs,
            rhs,
            span: to_span(e.span()),
        })
}

pub fn program_parser<'src>(
) -> impl Parser<'src, &'src str, Program, extra::Err<Rich<'src, char>>> + Clone {
    decl_parser()
        .then(stmt_parser().repeated().collect::<Vec<_>>())
        .then_ignore(end())
        .map(|(decls, stmts)| Program { decls, stmts })
}

pub fn parse_program(
    source: &str,
    source_name: &str,
) -> Result<Program, crate::error::OptCellsError> {
    program_parser()
        .parse(source)
        .into_result()
        .map_err(|errs| {
            let first = errs.into_iter().next().unwrap();
            crate::error::OptCellsError::ParseDsl {
                message: format!("{}", first.reason()),
                span: to_span(*first.span()),
                source_name: source_name.into(),
                source_text: source.into(),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Expr {
        expr_parser()
            .parse(src)
            .into_result()
            .expect("parse failed")
    }

    #[test]
    fn single_bit_literal_0() {
        let e = parse("0");
        assert!(matches!(
            e,
            Expr::Lit {
                value: BitLiteral::Unsized(0),
                ..
            }
        ));
    }

    #[test]
    fn single_bit_literal_1() {
        let e = parse("1");
        assert!(matches!(
            e,
            Expr::Lit {
                value: BitLiteral::Unsized(1),
                ..
            }
        ));
    }

    #[test]
    fn identifier() {
        let e = parse("abc");
        if let Expr::Ref { name, sel, .. } = e {
            assert_eq!(name, "abc");
            assert!(sel.is_none());
        } else {
            panic!("not a ref");
        }
    }

    #[test]
    fn indexed_signal() {
        let e = parse("state[3]");
        if let Expr::Ref {
            name,
            sel: Some(BitSel::Index(i)),
            ..
        } = e
        {
            assert_eq!(name, "state");
            assert_eq!(i, 3);
        } else {
            panic!("not indexed ref");
        }
    }

    #[test]
    fn range_select() {
        let e = parse("state[3:0]");
        if let Expr::Ref {
            sel: Some(BitSel::Range { hi, lo }),
            ..
        } = e
        {
            assert_eq!((hi, lo), (3, 0));
        } else {
            panic!("not range ref");
        }
    }

    #[test]
    fn vector_literal_binary() {
        let e = parse("4'b0111");
        if let Expr::Lit {
            value: BitLiteral::Vector { width, value },
            ..
        } = e
        {
            assert_eq!((width, value), (4, 0b0111));
        } else {
            panic!("not vector lit");
        }
    }

    #[test]
    fn vector_literal_hex() {
        let e = parse("8'hFF");
        if let Expr::Lit {
            value: BitLiteral::Vector { width, value },
            ..
        } = e
        {
            assert_eq!((width, value), (8, 0xFF));
        } else {
            panic!("not vector lit");
        }
    }

    #[test]
    fn not_expression() {
        let e = parse("!a");
        assert!(matches!(e, Expr::Not { .. }));
    }

    #[test]
    fn and_expression() {
        let e = parse("a & b");
        assert!(matches!(e, Expr::And { .. }));
    }

    #[test]
    fn precedence_not_before_and() {
        let e = parse("!a & b");
        if let Expr::And { lhs, rhs, .. } = e {
            assert!(matches!(*lhs, Expr::Not { .. }));
            assert!(matches!(*rhs, Expr::Ref { .. }));
        } else {
            panic!("not AND");
        }
    }

    #[test]
    fn precedence_and_before_or() {
        let e = parse("a | b & c");
        assert!(matches!(e, Expr::Or { .. }));
    }

    #[test]
    fn equality_expression() {
        let e = parse("state == 4'b0111");
        assert!(matches!(e, Expr::Eq { .. }));
    }

    #[test]
    fn conditional_expression() {
        let e = parse("sel ? a : b");
        if let Expr::Mux {
            sel,
            if_true,
            if_false,
            ..
        } = e
        {
            assert!(matches!(*sel, Expr::Ref { ref name, .. } if name == "sel"));
            assert!(matches!(*if_true, Expr::Ref { ref name, .. } if name == "a"));
            assert!(matches!(*if_false, Expr::Ref { ref name, .. } if name == "b"));
        } else {
            panic!("not mux");
        }
    }

    #[test]
    fn conditional_has_lower_precedence_than_or() {
        let e = parse("a | sel ? b : c");
        if let Expr::Mux { sel, .. } = e {
            assert!(matches!(*sel, Expr::Or { .. }));
        } else {
            panic!("not mux");
        }
    }

    #[test]
    fn parens_override_precedence() {
        let e = parse("(a | b) & c");
        if let Expr::And { lhs, .. } = e {
            assert!(matches!(*lhs, Expr::Or { .. }));
        } else {
            panic!("expected outer AND");
        }
    }

    #[test]
    fn nand_expression_from_motivating_example() {
        let e = parse("!(a & b)");
        if let Expr::Not { inner, .. } = e {
            assert!(matches!(*inner, Expr::And { .. }));
        } else {
            panic!("expected NOT");
        }
    }

    fn bin(e: &Expr) -> (BinOp, &Expr, &Expr) {
        match e {
            Expr::Binary { op, lhs, rhs, .. } => (*op, lhs, rhs),
            other => panic!("not a binary op: {other:?}"),
        }
    }

    #[test]
    fn unsized_integer_literal() {
        assert!(matches!(
            parse("10"),
            Expr::Lit {
                value: BitLiteral::Unsized(10),
                ..
            }
        ));
    }

    #[test]
    fn tilde_is_bitwise_and_bang_is_logical() {
        assert!(matches!(parse("~a"), Expr::BitNot { .. }));
        assert!(matches!(parse("!a"), Expr::Not { .. }));
    }

    #[test]
    fn mul_binds_tighter_than_add_and_add_is_left_associative() {
        let e = parse("a - b + c * d");
        let (op, lhs, rhs) = bin(&e);
        assert_eq!(op, BinOp::Add);
        assert_eq!(bin(lhs).0, BinOp::Sub);
        assert_eq!(bin(rhs).0, BinOp::Mul);
    }

    #[test]
    fn relational_operators() {
        for (src, want) in [
            ("a < b", BinOp::Lt),
            ("a <= b", BinOp::Le),
            ("a > b", BinOp::Gt),
            ("a >= b", BinOp::Ge),
        ] {
            assert_eq!(bin(&parse(src)).0, want, "{src}");
        }
    }

    #[test]
    fn precedence_add_then_relational_then_equality_then_and() {
        // parses as   a & (((b + c) < d) == e)
        let e = parse("a & b + c < d == e");
        let Expr::And { rhs, .. } = e else {
            panic!("expected AND at the top");
        };
        let Expr::Eq { lhs, .. } = *rhs else {
            panic!("expected == under AND");
        };
        let (op, sum, _) = bin(&lhs);
        assert_eq!(op, BinOp::Lt);
        assert_eq!(bin(sum).0, BinOp::Add);
    }

    #[test]
    fn dynamic_index_takes_an_expression() {
        let e = parse("bitmap[idx + 1]");
        let Expr::DynIndex { name, index, .. } = e else {
            panic!("expected dynamic index");
        };
        assert_eq!(name, "bitmap");
        assert_eq!(bin(&index).0, BinOp::Add);
    }

    #[test]
    fn slice_assignment_target() {
        let p = parse_full("output y[3:0]; y[2:1] = 2'b10;");
        assert_eq!(p.stmts[0].lhs.sel, Some(BitSel::Range { hi: 2, lo: 1 }));
    }

    fn parse_full(src: &str) -> Program {
        program_parser()
            .parse(src)
            .into_result()
            .expect("parse failed")
    }

    #[test]
    fn input_bit_decl() {
        let p = parse_full("input a;");
        assert_eq!(p.decls.len(), 1);
        assert!(matches!(
            p.decls[0],
            Decl::Input {
                width: Width::Bit,
                ..
            }
        ));
    }

    #[test]
    fn input_vector_decl() {
        let p = parse_full("input state[3:0];");
        if let Decl::Input {
            width: Width::Vector { hi, lo },
            ..
        } = &p.decls[0]
        {
            assert_eq!((*hi, *lo), (3, 0));
        } else {
            panic!();
        }
    }

    #[test]
    fn multiple_inputs_comma() {
        let p = parse_full("input a, b, c;");
        assert_eq!(p.decls.len(), 3);
    }

    #[test]
    fn output_decl() {
        let p = parse_full("output y;");
        assert!(matches!(p.decls[0], Decl::Output { .. }));
    }

    #[test]
    fn simple_assignment() {
        let p = parse_full("input a, b; output y; y = a & b;");
        assert_eq!(p.stmts.len(), 1);
        assert_eq!(p.stmts[0].lhs.name, "y");
    }

    #[test]
    fn motivating_example_one() {
        let p = parse_full("input a, b; output y; y = !(a & b);");
        assert_eq!(p.stmts.len(), 1);
        assert!(matches!(p.stmts[0].rhs, Expr::Not { .. }));
    }

    #[test]
    fn motivating_example_two() {
        let p = parse_full("input state[3:0]; output decoded; decoded = (state == 4'b0111);");
        assert_eq!(p.stmts.len(), 1);
        assert!(matches!(p.stmts[0].rhs, Expr::Eq { .. }));
    }
}
