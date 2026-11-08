//! The `credentials` rule.
//!
//! Checks: Checks for credentials. Rust counterpart of the original `rule_rule_credentials.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `credentials` rule.
#[derive(Debug)]
pub struct RuleCredentials {
    base: RuleBase,
}

impl RuleCredentials {
    /// Creates a new `credentials` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("credentials", "Checks for credentials"),
        }
    }
}

impl Default for RuleCredentials {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleCredentials {}

impl Rule for RuleCredentials {
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
        let r = RuleCredentials::new();
        assert_eq!(r.name(), "credentials");
        assert!(!r.description().is_empty());
    }
}
