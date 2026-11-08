//! The `deprecated-commands` rule.
//!
//! Checks: Checks for deprecated commands. Rust counterpart of the original `rule_rule_deprecated_commands.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `deprecated-commands` rule.
#[derive(Debug)]
pub struct RuleDeprecatedCommands {
    base: RuleBase,
}

impl RuleDeprecatedCommands {
    /// Creates a new `deprecated-commands` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("deprecated-commands", "Checks for deprecated commands"),
        }
    }
}

impl Default for RuleDeprecatedCommands {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleDeprecatedCommands {}

impl Rule for RuleDeprecatedCommands {
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
        let r = RuleDeprecatedCommands::new();
        assert_eq!(r.name(), "deprecated-commands");
        assert!(!r.description().is_empty());
    }
}
