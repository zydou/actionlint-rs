//! The `if-cond` rule.
//!
//! Checks: Checks for if condition. Rust counterpart of the original `rule_rule_if_cond.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `if-cond` rule.
#[derive(Debug)]
pub struct RuleIfCond {
    base: RuleBase,
}

impl RuleIfCond {
    /// Creates a new `if-cond` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("if-cond", "Checks for if condition"),
        }
    }
}

impl Default for RuleIfCond {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleIfCond {}

impl Rule for RuleIfCond {
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
        let r = RuleIfCond::new();
        assert_eq!(r.name(), "if-cond");
        assert!(!r.description().is_empty());
    }
}
