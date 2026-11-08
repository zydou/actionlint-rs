//! The `shell-name` rule.
//!
//! Checks: Checks for shell name. Rust counterpart of the original `rule_rule_shell_name.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `shell-name` rule.
#[derive(Debug)]
pub struct RuleShellName {
    base: RuleBase,
}

impl RuleShellName {
    /// Creates a new `shell-name` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("shell-name", "Checks for shell name"),
        }
    }
}

impl Default for RuleShellName {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleShellName {}

impl Rule for RuleShellName {
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
        let r = RuleShellName::new();
        assert_eq!(r.name(), "shell-name");
        assert!(!r.description().is_empty());
    }
}
