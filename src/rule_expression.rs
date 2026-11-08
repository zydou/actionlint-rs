//! The `expression` rule.
//!
//! Checks: Checks for expressions. Rust counterpart of the original `rule_rule_expression.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `expression` rule.
#[derive(Debug)]
pub struct RuleExpression {
    base: RuleBase,
}

impl RuleExpression {
    /// Creates a new `expression` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("expression", "Checks for expressions"),
        }
    }
}

impl Default for RuleExpression {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleExpression {}

impl Rule for RuleExpression {
    fn take_errors(&mut self) -> Vec<crate::error::Error> {
        self.base.take_errors()
    }

    fn name(&self) -> &'static str {
        self.base.name()
    }

    fn description(&self) -> &'static str {
        self.base.description()
    }

    fn enable_debug(&mut self, out: Box<dyn std::io::Write + Send>) {
        self.base.enable_debug(out);
    }

    fn set_config(&mut self, config: crate::config::Config) {
        self.base.set_config(config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_description() {
        let r = RuleExpression::new();
        assert_eq!(r.name(), "expression");
        assert!(!r.description().is_empty());
    }
}
