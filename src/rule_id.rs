//! The `id` rule.
//!
//! Checks: Checks for IDs. Rust counterpart of the original `rule_rule_id.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `id` rule.
#[derive(Debug)]
pub struct RuleId {
    base: RuleBase,
}

impl RuleId {
    /// Creates a new `id` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("id", "Checks for IDs"),
        }
    }
}

impl Default for RuleId {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleId {}

impl Rule for RuleId {
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
        let r = RuleId::new();
        assert_eq!(r.name(), "id");
        assert!(!r.description().is_empty());
    }
}
