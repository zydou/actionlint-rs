//! The `id` rule.
//!
//! Checks: Checks for duplication and naming convention of job/step IDs. Rust counterpart of the
//! original `rule_id.go`.

use std::collections::HashMap;

use crate::ast::{Job, Pos, Step};
use crate::rule::{Pass, Rule, RuleBase};
// A job/step ID must start with a letter or `_` and contain only alphanumerics, `-` or `_`.
fn job_id_valid(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The `id` rule.
#[derive(Debug)]
pub struct RuleId {
    base: RuleBase,
    seen: HashMap<std::string::String, Pos>,
}

impl RuleId {
    /// Creates a new `id` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "id",
                "Checks for duplication and naming convention of job/step IDs",
            ),
            seen: HashMap::new(),
        }
    }

    fn validate_convention(&mut self, id: Option<&crate::ast::String>, what: &str) {
        let Some(id) = id else {
            return;
        };
        if id.value.is_empty() || id.contains_expression() || job_id_valid(&id.value) {
            return;
        }
        self.base.error_at(
            id.pos,
            format!(
                "invalid {what} ID {:?}. {what} ID must start with a letter or _ and contain only alphanumeric characters, -, or _",
                id.value
            ),
        );
    }
}

impl Default for RuleId {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleId {
    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        self.seen.clear();

        self.validate_convention(Some(&n.id), "job");
        for j in &n.needs {
            self.validate_convention(Some(j), "job");
        }

        Ok(())
    }

    fn visit_job_post(&mut self, _n: &Job) -> Result<(), String> {
        self.seen.clear();
        Ok(())
    }

    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        let Some(id) = &n.id else {
            return Ok(());
        };

        self.validate_convention(Some(id), "step");

        let key = id.value.to_lowercase();
        if let Some(prev) = self.seen.get(&key) {
            self.base.error_at(
                id.pos,
                format!(
                    "step ID {:?} duplicates. previously defined at {}. step ID must be unique within a job. note that step ID is case insensitive",
                    id.value, prev
                ),
            );
            return Ok(());
        }
        self.seen.insert(key, id.pos);
        Ok(())
    }
}

impl Rule for RuleId {
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
    use crate::ast::{Exec, ExecRun};

    fn step_with_id(id: &str) -> Step {
        Step {
            id: Some(crate::ast::String::at(id, Pos::new(5, 7))),
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: "echo".into(),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(6, 9),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(5, 7),
        }
    }

    fn job_with(steps: Vec<Step>) -> Job {
        Job {
            id: "test".into(),
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
            steps,
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

    #[test]
    fn duplicate_step_id() {
        let mut r = RuleId::new();
        let job = job_with(vec![step_with_id("foo"), step_with_id("foo")]);
        r.visit_job_pre(&job).unwrap();
        r.visit_step(&job.steps[0]).unwrap();
        r.visit_step(&job.steps[1]).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("duplicates"));
        assert!(errs[0].message.contains("case insensitive"));
    }

    #[test]
    fn duplicate_step_id_case_insensitive() {
        let mut r = RuleId::new();
        let job = job_with(vec![step_with_id("Foo"), step_with_id("foo")]);
        r.visit_job_pre(&job).unwrap();
        r.visit_step(&job.steps[0]).unwrap();
        r.visit_step(&job.steps[1]).unwrap();
        assert_eq!(r.take_errors().len(), 1);
    }

    #[test]
    fn invalid_job_id_convention() {
        let mut r = RuleId::new();
        let mut job = job_with(vec![]);
        job.id = crate::ast::String::at("1invalid", Pos::new(2, 3));
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("invalid job ID"));
    }

    #[test]
    fn valid_ids_no_errors() {
        let mut r = RuleId::new();
        let job = job_with(vec![step_with_id("foo"), step_with_id("bar")]);
        r.visit_job_pre(&job).unwrap();
        r.visit_step(&job.steps[0]).unwrap();
        r.visit_step(&job.steps[1]).unwrap();
        r.visit_job_post(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn job_id_valid_pattern() {
        assert!(job_id_valid("foo"));
        assert!(job_id_valid("_foo"));
        assert!(job_id_valid("foo-bar_1"));
        assert!(!job_id_valid("1foo"));
        assert!(!job_id_valid("foo bar"));
        assert!(!job_id_valid("foo.bar"));
    }
}
