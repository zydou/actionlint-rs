//! The `env-var` rule.
//!
//! Checks: Checks for environment variables configuration at "env:". Rust counterpart of the
//! original `rule_env_var.go`.

use crate::ast::{Env, Job, Step, Workflow};
use crate::rule::{Pass, Rule, RuleBase};

/// The `env-var` rule.
#[derive(Debug)]
pub struct RuleEnvVar {
    base: RuleBase,
}

impl RuleEnvVar {
    /// Creates a new `env-var` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "env-var",
                "Checks for environment variables configuration at \"env:\"",
            ),
        }
    }

    fn check_env(&mut self, env: Option<&Env>) {
        let Some(env) = env else {
            return;
        };
        if env.expression.is_some() {
            return;
        }
        for v in env.vars.values() {
            if v.name.contains_expression() {
                continue; // Key name can contain expressions (#312)
            }
            // Note the tab and space characters in this set.
            if v.name.as_str().contains(['&', '=', ' ', '\t']) {
                self.base.error_at(
                    v.name.pos,
                    format!(
                        "environment variable name {:?} is invalid. '&', '=' and spaces should not be contained",
                        v.name.as_str()
                    ),
                );
            }
        }
    }
}

impl Default for RuleEnvVar {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleEnvVar {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        self.check_env(n.env.as_ref());
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        self.check_env(n.env.as_ref());
        if let Some(c) = &n.container {
            self.check_env(c.env.as_ref());
        }
        if let Some(services) = &n.services {
            for s in services.value.values() {
                self.check_env(s.container.env.as_ref());
            }
        }
        Ok(())
    }

    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        self.check_env(n.env.as_ref());
        Ok(())
    }
}

impl Rule for RuleEnvVar {
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
    use crate::ast::{EnvVar, Pos, String};
    use std::collections::BTreeMap;

    fn env_with(name: &str) -> Env {
        let mut vars = BTreeMap::new();
        vars.insert(
            name.to_lowercase(),
            EnvVar {
                name: String::at(name, Pos::new(3, 5)),
                value: String::at("v", Pos::new(3, 5)),
            },
        );
        Env {
            vars,
            expression: None,
        }
    }

    #[test]
    fn invalid_env_var_name() {
        let mut r = RuleEnvVar::new();
        let mut wf = crate::ast::Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: Some(env_with("FOO&BAR")),
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        let _ = &mut wf;
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("is invalid"));
        assert!(errs[0].message.contains("FOO&BAR"));
    }

    #[test]
    fn space_in_name_invalid() {
        let mut r = RuleEnvVar::new();
        let wf = crate::ast::Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: Some(env_with("FOO BAR")),
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        assert_eq!(r.take_errors().len(), 1);
    }

    #[test]
    fn valid_env_var_name() {
        let mut r = RuleEnvVar::new();
        let wf = crate::ast::Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: Some(env_with("VALID_NAME")),
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn expression_key_skipped() {
        let mut r = RuleEnvVar::new();
        let wf = crate::ast::Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: Some(env_with("${{ env.foo }}")),
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
