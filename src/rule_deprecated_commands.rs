//! The `deprecated-commands` rule.
//!
//! Checks: Checks for deprecated "set-output", "save-state", "set-env", and "add-path" commands at
//! "run:". Rust counterpart of the original `rule_deprecated_commands.go`. See:
//! - <https://github.blog/changelog/2020-10-01-github-actions-deprecating-set-env-and-add-path-commands/>
//! - <https://github.blog/changelog/2022-10-11-github-actions-deprecating-save-state-and-set-output-commands/>

use std::sync::OnceLock;

use regex::Regex;

use crate::ast::{Exec, Step};
use crate::rule::{Pass, Rule, RuleBase};

/// The regex matching deprecated workflow commands. Counterpart of `deprecatedCommandsPattern`.
fn deprecated_commands_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Match either `::(save-state|set-output|set-env) name=...::...` (group 1 is the command)
        // or `::add-path::...` (group 2 is the command).
        Regex::new(r"(?:::(save-state|set-output|set-env)\s+name=[a-zA-Z][a-zA-Z_-]*::\S+|::(add-path)::\S+)")
            .expect("deprecated commands regex must be valid")
    })
}

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
            base: RuleBase::new(
                "deprecated-commands",
                "Checks for deprecated \"set-output\", \"save-state\", \"set-env\", and \"add-path\" commands at \"run:\"",
            ),
        }
    }
}

impl Default for RuleDeprecatedCommands {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleDeprecatedCommands {
    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        let Exec::Run(r) = &n.exec else {
            return Ok(());
        };

        for caps in deprecated_commands_pattern().captures_iter(&r.run.value) {
            let c = caps
                .get(1)
                .map(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| caps.get(2).map(|m| m.as_str()))
                .unwrap_or_default();

            let a = match c {
                "set-output" => r#"echo "{name}={value}" >> $GITHUB_OUTPUT"#,
                "save-state" => r#"echo "{name}={value}" >> $GITHUB_STATE"#,
                "set-env" => r#"echo "{name}={value}" >> $GITHUB_ENV"#,
                "add-path" => r#"echo "{path}" >> $GITHUB_PATH"#,
                _ => continue,
            };

            self.base.error_at(
                r.run.pos,
                format!(
                    "workflow command {c:?} was deprecated. use `{a}` instead: https://docs.github.com/en/actions/using-workflows/workflow-commands-for-github-actions"
                ),
            );
        }
        Ok(())
    }
}

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
    use crate::ast::{ExecRun, Pos};

    fn step_with_run(run: &str) -> Step {
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: crate::ast::String::at(run, Pos::new(5, 9)),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(5, 9),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(5, 7),
        }
    }

    #[test]
    fn set_output_deprecated() {
        let mut r = RuleDeprecatedCommands::new();
        let s = step_with_run("echo ::set-output name=foo::bar");
        r.visit_step(&s).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("set-output"));
        assert!(errs[0].message.contains("$GITHUB_OUTPUT"));
    }

    #[test]
    fn add_path_deprecated() {
        let mut r = RuleDeprecatedCommands::new();
        let s = step_with_run("echo ::add-path::/usr/local/bin");
        r.visit_step(&s).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("add-path"));
        assert!(errs[0].message.contains("$GITHUB_PATH"));
    }

    #[test]
    fn multiple_commands() {
        let mut r = RuleDeprecatedCommands::new();
        let s = step_with_run("::set-env name=a::b\necho ::save-state name=c::d");
        r.visit_step(&s).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn modern_commands_ok() {
        let mut r = RuleDeprecatedCommands::new();
        let s = step_with_run("echo \"{name}={value}\" >> $GITHUB_OUTPUT");
        r.visit_step(&s).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn action_step_noop() {
        let mut r = RuleDeprecatedCommands::new();
        let s = Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Action(crate::ast::ExecAction {
                uses: crate::ast::String::at("actions/checkout@v4", Pos::new(5, 9)),
                inputs: Default::default(),
                entrypoint: None,
                args: None,
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(5, 7),
        };
        r.visit_step(&s).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
