//! Expression syntax errors.
//!
//! Rust counterpart of the original `expr.go`. An [`ExprError`] is produced while lexing or
//! parsing a `${{ }}` expression.

use std::fmt;

/// An error caused by lexing or parsing an expression.
///
/// For more details on the expression syntax see
/// <https://docs.github.com/en/actions/learn-github-actions/expressions>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    /// The error message.
    pub message: String,
    /// The byte offset at which the error occurred.
    pub offset: usize,
    /// The 1-based line number at which the error occurred.
    pub line: usize,
    /// The 1-based column number at which the error occurred.
    pub column: usize,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.line, self.column, self.offset, self.message
        )
    }
}

impl std::error::Error for ExprError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let e = ExprError {
            message: "unexpected token".to_string(),
            offset: 10,
            line: 1,
            column: 5,
        };
        assert_eq!(e.to_string(), "1:5:10: unexpected token");
    }
}
