//! The `credentials` rule.
//!
//! Checks: Checks for credentials in "services:" configuration. Rust counterpart of the original
//! `rule_credentials.go`.

use crate::ast::{Container, Job};
use crate::rule::{Pass, Rule, RuleBase};

/// The `credentials` rule.
#[derive(Debug)]
pub struct RuleCredentials {
    base: RuleBase,
}

impl RuleCredentials {
    /// Creates a new `credentials` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "credentials",
                "Checks for credentials in \"services:\" configuration",
            ),
        }
    }

    fn check_container(&mut self, where_: &str, n: &Container) {
        let Some(credentials) = &n.credentials else {
            return;
        };
        let Some(p) = &credentials.password else {
            return;
        };

        if !p.is_expression_assigned() {
            self.base.error_at(
                p.pos,
                format!(
                    "\"password\" section in {where_} should be specified via secrets. do not put password value directly"
                ),
            );
        }
    }
}

impl Default for RuleCredentials {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleCredentials {
    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        if let Some(c) = &n.container {
            self.check_container("\"container\" section", c);
        }
        if let Some(services) = &n.services {
            for s in services.value.values() {
                self.check_container(&format!("{:?} service", s.name.as_str()), &s.container);
            }
        }
        Ok(())
    }
}

impl Rule for RuleCredentials {
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
    use crate::ast::{Credentials, Exec, ExecRun, Pos, Service, Services, Step, String};
    use std::collections::BTreeMap;

    fn job_with_credentials(password: Option<String>) -> Job {
        let credentials = password.map(|pw| Credentials {
            username: None,
            password: Some(pw),
            expression: None,
            pos: Pos::new(1, 1),
        });
        let mut services = BTreeMap::new();
        services.insert(
            "svc".to_string(),
            Service {
                name: String::at("svc", Pos::new(1, 1)),
                container: Container {
                    image: String::at("image", Pos::new(1, 1)),
                    credentials,
                    env: None,
                    ports: vec![],
                    volumes: vec![],
                    options: None,
                    command: None,
                    entrypoint: None,
                    pos: Pos::new(1, 1),
                },
            },
        );
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
            steps: vec![],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: Some(Services {
                value: services,
                expression: None,
                pos: Pos::new(1, 1),
            }),
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(1, 1),
        }
    }

    fn dummy_step() -> Step {
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
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

    #[test]
    fn direct_password_reported() {
        let mut r = RuleCredentials::new();
        let job = job_with_credentials(Some(String::at("plainpassword", Pos::new(2, 3))));
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("should be specified via secrets"));
        assert_eq!(errs[0].line, 2);
        assert_eq!(errs[0].column, 3);
    }

    #[test]
    fn expression_password_ok() {
        let mut r = RuleCredentials::new();
        let job = job_with_credentials(Some(String::at(
            "${{ secrets.MY_PASSWORD }}",
            Pos::new(2, 3),
        )));
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn no_password_ok() {
        let mut r = RuleCredentials::new();
        let job = job_with_credentials(None);
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn visit_step_noop() {
        let mut r = RuleCredentials::new();
        r.visit_step(&dummy_step()).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
