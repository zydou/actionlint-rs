//! The `shellcheck` rule.
//!
//! Integrates the external `shellcheck` command to check `run:` scripts. Rust counterpart of the
//! original `rule_shellcheck.go`.

use crate::rule::{Pass, Rule, RuleBase};

/// The `shellcheck` rule.
#[derive(Debug)]
pub struct RuleShellcheck {
    base: RuleBase,
    #[allow(dead_code)]
    executable: String,
}

impl RuleShellcheck {
    /// Creates a new `shellcheck` rule using the given executable.
    ///
    /// Returns an error when the executable cannot be resolved on `$PATH`.
    pub fn new(executable: &str) -> Result<Self, String> {
        crate::process::resolve_external_command(executable)?;
        Ok(Self {
            base: RuleBase::new("shellcheck", "Checks shell scripts with shellcheck"),
            executable: executable.to_string(),
        })
    }
}

impl Pass for RuleShellcheck {}

impl Rule for RuleShellcheck {
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
        assert!(RuleShellcheck::new("this-does-not-exist-xyz").is_err());
    }

    #[test]
    fn present_executable_ok() {
        if cfg!(unix) && which_ok("sh") {
            let r = RuleShellcheck::new("echo").unwrap();
            assert_eq!(r.name(), "shellcheck");
        }
    }

    fn which_ok(cmd: &str) -> bool {
        crate::process::resolve_external_command(cmd).is_ok()
    }
}
