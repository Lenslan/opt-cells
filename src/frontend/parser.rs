use chumsky::prelude::*;
use crate::error::Span as MySpan;
use crate::frontend::ast::*;

fn to_span(r: SimpleSpan) -> MySpan {
    MySpan::new(r.start, r.end)
}

pub fn expr_parser<'src>() -> impl Parser<'src, &'src str, Expr, extra::Err<Rich<'src, char>>> + Clone {
    recursive(|expr| {
        // unsigned integer (decimal)
        let uint = text::int(10)
            .map(|s: &str| s.parse::<u32>().unwrap());

        // vector literal: <width>'<radix><digits>
        let vec_lit = text::int(10)
            .then_ignore(just('\''))
            .then(one_of("bhd"))
            .then(any().filter(|c: &char| c.is_ascii_alphanumeric()).repeated().at_least(1).collect::<String>())
            .map_with(|((width_s, radix), digits): ((&str, char), String), e| {
                let width: u32 = width_s.parse().unwrap();
                let r = match radix {
                    'b' => 2,
                    'h' => 16,
                    'd' => 10,
                    _ => unreachable!(),
                };
                let value = u64::from_str_radix(&digits, r).unwrap();
                Expr::Lit { value: BitLiteral::Vector { width, value }, span: to_span(e.span()) }
            });

        // single bit literal: only when not followed by '
        let bit_lit = choice((
            just('0').to(false),
            just('1').to(true),
        ))
        .then_ignore(just('\'').not().rewind())
        .map_with(|b, e| Expr::Lit { value: BitLiteral::Single(b), span: to_span(e.span()) });

        // bit-select: [i] or [hi:lo]
        let bit_sel = uint.clone()
            .then(just(':').padded().ignore_then(uint.clone()).or_not())
            .delimited_by(just('[').padded(), just(']').padded())
            .map(|(a, b)| match b {
                Some(lo) => BitSel::Range { hi: a, lo },
                None => BitSel::Index(a),
            });

        // identifier reference
        let signal_ref = text::ident()
            .then(bit_sel.or_not())
            .map_with(|(name, sel): (&str, Option<BitSel>), e| {
                Expr::Ref { name: name.to_string(), sel, span: to_span(e.span()) }
            });

        let parens = expr.clone().delimited_by(just('(').padded(), just(')').padded());

        let primary = choice((
            parens,
            vec_lit,
            bit_lit,
            signal_ref,
        ))
        .padded();

        // unary: ! or ~  (right-associative via recursive)
        let unary = recursive(|unary| {
            choice((
                one_of("!~").padded()
                    .ignore_then(unary)
                    .map_with(|inner, e| Expr::Not { inner: Box::new(inner), span: to_span(e.span()) }),
                primary.clone(),
            ))
        });

        // eq_expr: unary (("==" | "!=") unary)?
        let eq_op = choice((
            just("==").to(0u8),
            just("!=").to(1u8),
        )).padded();

        let eq_expr = unary.clone()
            .then(eq_op.then(unary.clone()).or_not())
            .map_with(|(lhs, rest), e| match rest {
                None => lhs,
                Some((op, rhs)) => {
                    let span = to_span(e.span());
                    if op == 0 {
                        Expr::Eq { lhs: Box::new(lhs), rhs: Box::new(rhs), span }
                    } else {
                        Expr::Neq { lhs: Box::new(lhs), rhs: Box::new(rhs), span }
                    }
                }
            });

        // and_expr: eq_expr ("&" eq_expr)*  (left-associative)
        let and_expr = eq_expr.clone()
            .foldl_with(
                just('&').padded().ignore_then(eq_expr.clone()).repeated(),
                |lhs, rhs, e| Expr::And { lhs: Box::new(lhs), rhs: Box::new(rhs), span: to_span(e.span()) },
            );

        // xor_expr: and_expr ("^" and_expr)*
        let xor_expr = and_expr.clone()
            .foldl_with(
                just('^').padded().ignore_then(and_expr.clone()).repeated(),
                |lhs, rhs, e| Expr::Xor { lhs: Box::new(lhs), rhs: Box::new(rhs), span: to_span(e.span()) },
            );

        // or_expr: xor_expr ("|" xor_expr)*
        let or_expr = xor_expr.clone()
            .foldl_with(
                just('|').padded().ignore_then(xor_expr.clone()).repeated(),
                |lhs, rhs, e| Expr::Or { lhs: Box::new(lhs), rhs: Box::new(rhs), span: to_span(e.span()) },
            );

        or_expr.padded()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Expr {
        expr_parser().parse(src).into_result().expect("parse failed")
    }

    #[test]
    fn single_bit_literal_0() {
        let e = parse("0");
        assert!(matches!(e, Expr::Lit { value: BitLiteral::Single(false), .. }));
    }

    #[test]
    fn single_bit_literal_1() {
        let e = parse("1");
        assert!(matches!(e, Expr::Lit { value: BitLiteral::Single(true), .. }));
    }

    #[test]
    fn identifier() {
        let e = parse("abc");
        if let Expr::Ref { name, sel, .. } = e {
            assert_eq!(name, "abc");
            assert!(sel.is_none());
        } else { panic!("not a ref"); }
    }

    #[test]
    fn indexed_signal() {
        let e = parse("state[3]");
        if let Expr::Ref { name, sel: Some(BitSel::Index(i)), .. } = e {
            assert_eq!(name, "state");
            assert_eq!(i, 3);
        } else { panic!("not indexed ref"); }
    }

    #[test]
    fn range_select() {
        let e = parse("state[3:0]");
        if let Expr::Ref { sel: Some(BitSel::Range { hi, lo }), .. } = e {
            assert_eq!((hi, lo), (3, 0));
        } else { panic!("not range ref"); }
    }

    #[test]
    fn vector_literal_binary() {
        let e = parse("4'b0111");
        if let Expr::Lit { value: BitLiteral::Vector { width, value }, .. } = e {
            assert_eq!((width, value), (4, 0b0111));
        } else { panic!("not vector lit"); }
    }

    #[test]
    fn vector_literal_hex() {
        let e = parse("8'hFF");
        if let Expr::Lit { value: BitLiteral::Vector { width, value }, .. } = e {
            assert_eq!((width, value), (8, 0xFF));
        } else { panic!("not vector lit"); }
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
        } else { panic!("not AND"); }
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
    fn parens_override_precedence() {
        let e = parse("(a | b) & c");
        if let Expr::And { lhs, .. } = e {
            assert!(matches!(*lhs, Expr::Or { .. }));
        } else { panic!("expected outer AND"); }
    }

    #[test]
    fn nand_expression_from_motivating_example() {
        let e = parse("!(a & b)");
        if let Expr::Not { inner, .. } = e {
            assert!(matches!(*inner, Expr::And { .. }));
        } else { panic!("expected NOT"); }
    }
}
