use ariadne::{Color, Label, Report, ReportKind, Source};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
    pub fn as_range(&self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(thiserror::Error, Debug)]
pub enum OptCellsError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error in DSL: {message}")]
    ParseDsl {
        message: String,
        span: Span,
        source_name: String,
        source_text: String,
    },
    /// Library-level error (e.g. TOML syntax, validation). Library files are
    /// addressed by name only; we don't track byte-level source positions for them.
    #[error("parse error in library: {message}")]
    ParseLibrary {
        message: String,
        source_name: String,
    },
    #[error("elaboration error: {message}")]
    Elaborate {
        message: String,
        span: Span,
        source_name: String,
        source_text: String,
    },
    #[error("mapping error: {message}")]
    Mapping { message: String },
}

impl OptCellsError {
    pub fn render(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        match self {
            OptCellsError::ParseDsl {
                message,
                span,
                source_name,
                source_text,
            }
            | OptCellsError::Elaborate {
                message,
                span,
                source_name,
                source_text,
            } => {
                let report = Report::build(ReportKind::Error, source_name.clone(), span.start)
                    .with_message(message)
                    .with_label(
                        Label::new((source_name.clone(), span.as_range()))
                            .with_message(message)
                            .with_color(Color::Red),
                    )
                    .finish();
                report.write(
                    (source_name.clone(), Source::from(source_text.clone())),
                    out,
                )
            }
            other => writeln!(out, "error: {}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_range_round_trip() {
        let s = Span::new(3, 10);
        assert_eq!(s.as_range(), 3..10);
    }

    #[test]
    fn error_display_includes_message() {
        let e = OptCellsError::Mapping {
            message: "no match".into(),
        };
        assert!(format!("{}", e).contains("no match"));
    }

    #[test]
    fn render_parse_dsl_produces_output() {
        let err = OptCellsError::ParseDsl {
            message: "unexpected token".into(),
            span: Span::new(0, 5),
            source_name: "test.dsl".into(),
            source_text: "input a;".into(),
        };
        let mut buf: Vec<u8> = Vec::new();
        err.render(&mut buf).expect("render ok");
        assert!(!buf.is_empty(), "render should produce non-empty output");
    }

    #[test]
    fn render_mapping_uses_fallback() {
        let err = OptCellsError::Mapping {
            message: "no cell".into(),
        };
        let mut buf: Vec<u8> = Vec::new();
        err.render(&mut buf).expect("render ok");
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("error:"));
        assert!(s.contains("no cell"));
    }
}
