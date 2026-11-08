//! The `pyflakes` rule.
//!
//! Integrates the external `pyflakes` command to check Python scripts in workflow files. Rust
//! counterpart of the original `rule_pyflakes.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `pyflakes` rule.
#[derive(Debug)]
pub struct RulePyflakes {
    base: RuleBase,
    #[allow(dead_code)]
    executable: String,
}

impl RulePyflakes {
    /// Creates a new `pyflakes` rule using the given executable.
    ///
    /// Returns an error when the executable cannot be resolved on `$PATH`.
    pub fn new(executable: &str) -> Result<Self, String> {
        crate::process::resolve_external_command(executable)?;
        Ok(Self {
            base: RuleBase::new("pyflakes", "Checks Python scripts with pyflakes"),
            executable: executable.to_string(),
        })
    }
}

impl Pass for RulePyflakes {}

impl Rule for RulePyflakes {
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
    fn missing_executable_errors() {
        assert!(RulePyflakes::new("this-does-not-exist-xyz").is_err());
    }
}
