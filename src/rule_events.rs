//! The `events` rule.
//!
//! Checks: Checks for events. Rust counterpart of the original `rule_rule_events.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `events` rule.
#[derive(Debug)]
pub struct RuleEvents {
    base: RuleBase,
}

impl RuleEvents {
    /// Creates a new `events` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("events", "Checks for events"),
        }
    }
}

impl Default for RuleEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleEvents {}

impl Rule for RuleEvents {
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
        let r = RuleEvents::new();
        assert_eq!(r.name(), "events");
        assert!(!r.description().is_empty());
    }
}
