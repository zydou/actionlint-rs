//! The `Rule` trait and shared rule infrastructure.
//!
//! Rust counterpart of the original `rule.go`. A [`Rule`] traverses a workflow syntax tree via
//! the [`crate::pass::Pass`] callbacks and collects [`crate::error::Error`] values.

use std::fmt;
use std::io::Write;

use crate::ast::{Job, Step, Workflow};
use crate::config::Config;
use crate::error::Error;

/// Shared state and helpers common to all rules.
///
/// This is the counterpart of `RuleBase` in the original. Each rule embeds a [`RuleBase`] and
/// delegates the [`Pass`]-independent methods to it.
#[derive(Default)]
pub struct RuleBase {
    name: &'static str,
    description: &'static str,
    errors: Vec<Error>,
    debug: Option<std::sync::Mutex<Box<dyn Write + Send>>>,
    config: Option<Config>,
}

impl RuleBase {
    /// Creates a new `RuleBase` with the given rule name and description.
    #[must_use]
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            errors: Vec::new(),
            debug: None,
            config: None,
        }
    }

    /// Records a new error at the given position with the given message.
    pub fn error(&mut self, line: usize, col: usize, message: impl Into<String>) {
        self.errors.push(Error::at(line, col, self.name, message));
    }

    /// Records a new error located at `pos`.
    pub fn error_at(&mut self, pos: crate::ast::Pos, message: impl Into<String>) {
        self.error(pos.line, pos.col, message);
    }

    /// Records a new error with a formatted message located at `pos`.
    pub fn errorf_at(&mut self, pos: crate::ast::Pos, message: fmt::Arguments<'_>) {
        self.error(pos.line, pos.col, message.to_string());
    }

    /// Returns the errors found by the rule, consuming them.
    pub fn take_errors(&mut self) -> Vec<Error> {
        std::mem::take(&mut self.errors)
    }

    /// Returns the errors found by the rule.
    pub fn errors(&self) -> &[Error] {
        &self.errors
    }

    /// Returns the rule name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the rule description.
    pub fn description(&self) -> &'static str {
        self.description
    }

    /// Enables debug output to the given writer.
    pub fn enable_debug(&mut self, out: Box<dyn Write + Send>) {
        self.debug = Some(std::sync::Mutex::new(out));
    }

    /// Writes a debug log line prefixed with the rule name. Does nothing unless debug output was
    /// enabled via [`Self::enable_debug`].
    pub fn debug(&self, message: fmt::Arguments<'_>) {
        if let Some(w) = &self.debug
            && let Ok(mut w) = w.lock()
        {
            let _ = writeln!(w, "[{}] {}", self.name, message);
        }
    }

    /// Populates the user configuration for this rule.
    pub fn set_config(&mut self, config: Config) {
        self.config = Some(config);
    }

    /// Returns the user configuration, if any was set.
    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref()
    }
}

impl fmt::Debug for RuleBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuleBase")
            .field("name", &self.name)
            .field("errors", &self.errors.len())
            .finish()
    }
}

/// A pass that traverses a workflow syntax tree.
///
/// This is the Rust counterpart of the `Pass` interface. Each callback returns an internal error
/// when the traversal cannot continue.
pub trait Pass {
    /// Called when visiting a [`Step`] node.
    fn visit_step(&mut self, _node: &Step) -> Result<(), String> {
        Ok(())
    }

    /// Called when visiting a [`Job`] node before visiting its children.
    fn visit_job_pre(&mut self, _node: &Job) -> Result<(), String> {
        Ok(())
    }

    /// Called when visiting a [`Job`] node after visiting its children.
    fn visit_job_post(&mut self, _node: &Job) -> Result<(), String> {
        Ok(())
    }

    /// Called when visiting a [`Workflow`] node before visiting its children.
    fn visit_workflow_pre(&mut self, _node: &Workflow) -> Result<(), String> {
        Ok(())
    }

    /// Called when visiting a [`Workflow`] node after visiting its children.
    fn visit_workflow_post(&mut self, _node: &Workflow) -> Result<(), String> {
        Ok(())
    }
}

/// A [`Pass`] that also produces errors and carries rule metadata.
pub trait Rule: Pass {
    /// Returns the errors found by this rule, consuming them.
    fn take_errors(&mut self) -> Vec<Error>;

    /// Returns the rule name.
    fn name(&self) -> &'static str;

    /// Returns the rule description.
    fn description(&self) -> &'static str;

    /// Enables debug output.
    fn enable_debug(&mut self, out: Box<dyn Write + Send>);

    /// Populates the user configuration.
    fn set_config(&mut self, config: Config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Pos;

    struct DummyRule {
        base: RuleBase,
    }

    impl DummyRule {
        fn new() -> Self {
            Self {
                base: RuleBase::new("dummy", "dummy rule"),
            }
        }
    }

    impl Pass for DummyRule {
        fn visit_step(&mut self, _node: &Step) -> Result<(), String> {
            self.base.error_at(Pos::new(1, 2), "an error");
            Ok(())
        }
    }

    impl Rule for DummyRule {
        fn take_errors(&mut self) -> Vec<Error> {
            self.base.take_errors()
        }

        fn name(&self) -> &'static str {
            self.base.name()
        }

        fn description(&self) -> &'static str {
            self.base.description()
        }

        fn enable_debug(&mut self, out: Box<dyn Write + Send>) {
            self.base.enable_debug(out);
        }

        fn set_config(&mut self, config: Config) {
            self.base.set_config(config);
        }
    }

    #[test]
    fn rule_records_errors() {
        let mut r = DummyRule::new();
        let step_job = Job {
            id: "j".into(),
            name: None,
            needs: vec![],
            runs_on: None,
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![Step {
                id: None,
                if_condition: None,
                name: None,
                exec: crate::ast::Exec::Run(crate::ast::ExecRun {
                    run: "echo".into(),
                    shell: None,
                    working_directory: None,
                    run_pos: Pos::new(1, 1),
                }),
                env: None,
                continue_on_error: None,
                timeout_minutes: None,
                pos: Pos::new(1, 1),
            }],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(1, 1),
        };
        r.visit_step(&step_job.steps[0]).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 1);
        assert_eq!(errs[0].column, 2);
        assert_eq!(errs[0].kind, "dummy");
        assert_eq!(r.name(), "dummy");
        assert_eq!(r.description(), "dummy rule");
    }

    #[test]
    fn default_pass_is_noop() {
        struct Noop;
        impl Pass for Noop {}
        let mut n = Noop;
        assert!(n.visit_step(&create_step()).is_ok());
        assert!(n.visit_job_pre(&create_job()).is_ok());
        assert!(n.visit_workflow_pre(&create_workflow()).is_ok());
    }

    fn create_step() -> Step {
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: crate::ast::Exec::Run(crate::ast::ExecRun {
                run: "echo".into(),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(1, 1),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(1, 1),
        }
    }

    fn create_job() -> Job {
        Job {
            id: "j".into(),
            name: None,
            needs: vec![],
            runs_on: None,
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(1, 1),
        }
    }

    fn create_workflow() -> Workflow {
        Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        }
    }
}
