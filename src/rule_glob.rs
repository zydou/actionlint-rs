//! The `glob` rule.
//!
//! Checks: Checks for glob. Rust counterpart of the original `rule_rule_glob.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `glob` rule.
#[derive(Debug)]
pub struct RuleGlob {
    base: RuleBase,
}

impl RuleGlob {
    /// Creates a new `glob` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("glob", "Checks for glob"),
        }
    }
}

impl Default for RuleGlob {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleGlob {}

impl Rule for RuleGlob {
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
        let r = RuleGlob::new();
        assert_eq!(r.name(), "glob");
        assert!(!r.description().is_empty());
    }
}
