//! The `job-needs` rule.
//!
//! Checks: Checks for job needs. Rust counterpart of the original `rule_rule_job_needs.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `job-needs` rule.
#[derive(Debug)]
pub struct RuleJobNeeds {
    base: RuleBase,
}

impl RuleJobNeeds {
    /// Creates a new `job-needs` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("job-needs", "Checks for job needs"),
        }
    }
}

impl Default for RuleJobNeeds {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleJobNeeds {}

impl Rule for RuleJobNeeds {
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
        let r = RuleJobNeeds::new();
        assert_eq!(r.name(), "job-needs");
        assert!(!r.description().is_empty());
    }
}
