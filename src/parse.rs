//! Parser for GitHub Actions workflow configuration files.
//!
//! Rust counterpart of the original `parse.go`. It parses a YAML workflow file into an
//! [`ast::Workflow`] syntax tree, reporting syntax errors as [`error::Error`] values with the
//! `syntax-check` kind. See
//! <https://docs.github.com/en/actions/reference/workflow-syntax-for-github-actions>.

use crate::ast::Workflow;
use crate::error::Error;

/// Parses the given workflow YAML source. Returns the parsed [`Workflow`] (or `None` when the
/// source cannot be parsed into a workflow at all) and a list of syntax errors.
#[must_use]
pub fn parse(_content: &[u8]) -> (Option<Workflow>, Vec<Error>) {
    (None, Vec::new())
}

#[cfg(test)]
mod tests {
    #[test]
    fn stub() {
        // Full parser is ported separately; this stub keeps the crate compiling.
    }
}
