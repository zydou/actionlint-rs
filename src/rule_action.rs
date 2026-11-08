//! The `action` rule.
//!
//! Checks: Checks for actions. Rust counterpart of the original `rule_rule_action.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `action` rule.
#[derive(Debug)]
pub struct RuleAction {
    base: RuleBase,
}

impl RuleAction {
    /// Creates a new `action` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("action", "Checks for actions"),
        }
    }
}

impl Default for RuleAction {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleAction {}

impl Rule for RuleAction {
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
        let r = RuleAction::new();
        assert_eq!(r.name(), "action");
        assert!(!r.description().is_empty());
    }
}
