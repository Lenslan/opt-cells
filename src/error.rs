use std::ops::Range;
use ariadne::{Color, Label, Report, ReportKind, Source};

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
    ParseDsl { message: String, span: Span, source_name: String, source_text: String },
    #[error("parse error in library: {message}")]
    ParseLibrary { message: String, source_name: String },
    #[error("elaboration error: {message}")]
    Elaborate { message: String, span: Span, source_name: String, source_text: String },
    #[error("mapping error: {message}")]
    Mapping { message: String },
}

impl OptCellsError {
    pub fn render(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        match self {
            OptCellsError::ParseDsl { message, span, source_name, source_text }
            | OptCellsError::Elaborate { message, span, source_name, source_text } => {
                let report = Report::build(ReportKind::Error, source_name.clone(), span.start)
                    .with_message(message)
                    .with_label(
                        Label::new((source_name.clone(), span.as_range()))
                            .with_message(message)
                            .with_color(Color::Red),
                    )
                    .finish();
                report.write((source_name.clone(), Source::from(source_text.clone())), out)
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
        let e = OptCellsError::Mapping { message: "no match".into() };
        assert!(format!("{}", e).contains("no match"));
    }
}
