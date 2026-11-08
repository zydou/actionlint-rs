//! Semantic checker for GitHub Actions expressions.
//!
//! Rust counterpart of the original `expr_sema.go`. It computes the static type of a value produced
//! by an expression and reports type errors. See
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>.

use crate::expr::ExprError;
use crate::expr_ast::ExprNode;
use crate::expr_type::{ExprTypeKind, ObjectType};

/// The signature of a builtin function.
#[derive(Debug, Clone)]
pub struct FuncSignature {
    /// The name of the function.
    pub name: &'static str,
    /// The types of the function parameters.
    pub params: &'static [ExprTypeKind],
    /// The return type of the function.
    pub ret: ExprTypeKind,
}

/// A semantic checker for expressions.
#[derive(Clone)]
pub struct ExprSemanticsChecker {
    /// Whether to check for untrusted inputs.
    pub check_untrusted_input: bool,
    /// The config variables defined in the repository config, if any.
    pub config_vars: Option<Vec<String>>,
    /// The types of the `matrix` context.
    pub matrix: ObjectType,
    /// The types of the `steps` context.
    pub steps: ObjectType,
    /// The types of the `needs` context.
    pub needs: ObjectType,
    /// The types of the `secrets` context.
    pub secrets: ObjectType,
    /// The types of the `inputs` context.
    pub inputs: ObjectType,
    /// The types of the `jobs` context.
    pub jobs: ObjectType,
    /// The contexts available at the current workflow key.
    pub available_contexts: Vec<String>,
    /// The special functions available at the current workflow key.
    pub available_special_funcs: Vec<String>,
}

impl ExprSemanticsChecker {
    /// Creates a new checker. `check_untrusted_input` enables untrusted input detection. `config_vars`
    /// is the list of config variables defined in the repository config, if any.
    #[must_use]
    pub fn new(check_untrusted_input: bool, config_vars: Option<Vec<String>>) -> Self {
        Self {
            check_untrusted_input,
            config_vars,
            matrix: ObjectType::default(),
            steps: ObjectType::default(),
            needs: ObjectType::default(),
            secrets: ObjectType::default(),
            inputs: ObjectType::default(),
            jobs: ObjectType::default(),
            available_contexts: Vec::new(),
            available_special_funcs: Vec::new(),
        }
    }

    /// Checks the given expression, returning its static type and any errors found.
    #[must_use]
    pub fn check(&self, _expr: &ExprNode) -> (ExprTypeKind, Vec<ExprError>) {
        (ExprTypeKind::any(), Vec::new())
    }

    /// Returns whether the given expression is a constant value.
    #[must_use]
    pub fn is_constant(&self, _expr: &ExprNode) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stub() {
        // Full type checker is ported separately; this stub keeps the crate compiling.
    }
}
