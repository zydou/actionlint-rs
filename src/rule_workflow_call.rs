//! The `workflow-call` rule.
//!
//! Checks: Checks for workflow call. Rust counterpart of the original `rule_rule_workflow_call.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `workflow-call` rule.
#[derive(Debug)]
pub struct RuleWorkflowCall {
    base: RuleBase,
}

impl RuleWorkflowCall {
    /// Creates a new `workflow-call` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("workflow-call", "Checks for workflow call"),
        }
    }
}

impl Default for RuleWorkflowCall {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleWorkflowCall {}

impl Rule for RuleWorkflowCall {
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
        let r = RuleWorkflowCall::new();
        assert_eq!(r.name(), "workflow-call");
        assert!(!r.description().is_empty());
    }
}
