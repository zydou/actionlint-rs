//! The `permissions` rule.
//!
//! Checks: Checks for permissions. Rust counterpart of the original `rule_rule_permissions.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `permissions` rule.
#[derive(Debug)]
pub struct RulePermissions {
    base: RuleBase,
}

impl RulePermissions {
    /// Creates a new `permissions` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("permissions", "Checks for permissions"),
        }
    }
}

impl Default for RulePermissions {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RulePermissions {}

impl Rule for RulePermissions {
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
        let r = RulePermissions::new();
        assert_eq!(r.name(), "permissions");
        assert!(!r.description().is_empty());
    }
}
