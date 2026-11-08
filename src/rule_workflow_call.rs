//! The `workflow-call` rule.
//!
//! Checks: Checks for reusable workflow calls. Inputs and outputs of called reusable workflow are
//! checked. Rust counterpart of the original `rule_workflow_call.go`. See
//! <https://docs.github.com/en/actions/learn-github-actions/reusing-workflows>.

use std::sync::Arc;

use crate::ast::{Event, Job, Workflow, WorkflowCall};
use crate::quotes::sorted_quotes;
use crate::reusable_workflow::LocalReusableWorkflowCache;
use crate::rule::{Pass, Rule, RuleBase};

/// The `workflow-call` rule.
#[derive(Debug)]
pub struct RuleWorkflowCall {
    base: RuleBase,
    /// Path to the workflow file being linted, relative to the project root or absolute.
    workflow_path: std::path::PathBuf,
    cache: Arc<LocalReusableWorkflowCache>,
}

impl RuleWorkflowCall {
    /// Creates a new `workflow-call` rule. `workflow_path` is a file path to the workflow which is
    /// relative to a project root directory or an absolute path.
    #[must_use]
    pub fn new(workflow_path: std::path::PathBuf, cache: Arc<LocalReusableWorkflowCache>) -> Self {
        Self {
            base: RuleBase::new(
                "workflow-call",
                "Checks for reusable workflow calls. Inputs and outputs of called reusable workflow are checked",
            ),
            workflow_path,
            cache,
        }
    }

    fn check_workflow_call_uses_local(&mut self, call: &WorkflowCall) {
        let u = &call.uses;
        let m = match self.cache.find_metadata(&u.value) {
            Err(e) => {
                self.base.error_at(u.pos, e);
                return;
            }
            Ok(None) => {
                self.base.debug(format_args!(
                    "Skip workflow call {:?} since no metadata was found",
                    u.value
                ));
                return;
            }
            Ok(Some(m)) => m,
        };

        // Validate inputs
        for (n, i) in &m.inputs {
            if i.required && !call.inputs.contains_key(n) {
                self.base.error_at(
                    u.pos,
                    format!(
                        "input {:?} is required by {:?} reusable workflow",
                        i.name, u.value
                    ),
                );
            }
        }
        for (n, i) in &call.inputs {
            if !m.inputs.contains_key(n) {
                let note = if m.inputs.is_empty() {
                    "no input is defined".to_string()
                } else {
                    let is: Vec<&str> = m.inputs.values().map(|i| i.name.as_str()).collect();
                    if is.len() == 1 {
                        format!("defined input is {:?}", is[0])
                    } else {
                        format!("defined inputs are {}", sorted_quotes(is))
                    }
                };
                self.base.error_at(
                    i.name.pos,
                    format!(
                        "input {:?} is not defined in {:?} reusable workflow. {note}",
                        i.name.value, u.value
                    ),
                );
            }
        }

        // Validate secrets
        if !call.inherit_secrets {
            for (n, s) in &m.secrets {
                if s.required && !call.secrets.contains_key(n) {
                    self.base.error_at(
                        u.pos,
                        format!(
                            "secret {:?} is required by {:?} reusable workflow",
                            s.name, u.value
                        ),
                    );
                }
            }
            for (n, s) in &call.secrets {
                if !m.secrets.contains_key(n) {
                    let note = if m.secrets.is_empty() {
                        "no secret is defined".to_string()
                    } else {
                        let ss: Vec<&str> = m.secrets.values().map(|s| s.name.as_str()).collect();
                        if ss.len() == 1 {
                            format!("defined secret is {:?}", ss[0])
                        } else {
                            format!("defined secrets are {}", sorted_quotes(ss))
                        }
                    };
                    self.base.error_at(
                        s.name.pos,
                        format!(
                            "secret {:?} is not defined in {:?} reusable workflow. {note}",
                            s.name.value, u.value
                        ),
                    );
                }
            }
        }

        self.base
            .debug(format_args!("Validated reusable workflow {:?}", u.value));
    }
}

impl Pass for RuleWorkflowCall {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        for e in &n.on {
            if let Event::WorkflowCall(e) = e {
                // Register this reusable workflow in cache so that it does not need to parse this
                // workflow file again when this workflow is called by other workflows.
                self.cache.write_workflow_call_event(&self.workflow_path, e);
                break;
            }
        }
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        let Some(call) = &n.workflow_call else {
            return Ok(());
        };

        let u = &call.uses;
        if u.value.is_empty() || u.contains_expression() {
            return Ok(());
        }

        if is_workflow_call_uses_local_format(&u.value) {
            self.check_workflow_call_uses_local(call);
            return Ok(());
        }

        if is_workflow_call_uses_repo_format(&u.value) {
            return Ok(());
        }

        // When the spec is an invalid local reusable workflow call, remember it caused an error by
        // caching `None`. This prevents a later lookup from emitting a redundant "could not read
        // reusable workflow file" error, matching the original.
        if u.value.starts_with("./") {
            self.cache.write_cache(&u.value, None);
        }

        self.base.error_at(
            u.pos,
            format!(
                "reusable workflow call {:?} at \"uses\" is not following the format \"owner/repo/path/to/workflow.yml@ref\" nor \"./path/to/workflow.yml\". see https://docs.github.com/en/actions/learn-github-actions/reusing-workflows for more details",
                u.value
            ),
        );
        Ok(())
    }
}

impl Default for RuleWorkflowCall {
    fn default() -> Self {
        let cache = LocalReusableWorkflowCache::new_null(None);
        Self::new(std::path::PathBuf::new(), Arc::new(cache))
    }
}

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

/// Parses `./{path}/{filename}`. Counterpart of `isWorkflowCallUsesLocalFormat`.
fn is_workflow_call_uses_local_format(u: &str) -> bool {
    if !u.starts_with("./") {
        return false;
    }
    let u = &u[2..];

    // Cannot contain a ref
    if let Some(idx) = u.find('@')
        && idx > 0
    {
        return false;
    }

    !u.is_empty()
}

/// Parses `{owner}/{repo}/{path to workflow.yml}@{ref}`. Counterpart of `isWorkflowCallUsesRepoFormat`.
fn is_workflow_call_uses_repo_format(u: &str) -> bool {
    // Repo reference must start with owner
    if u.starts_with('.') {
        return false;
    }

    let Some(idx) = u.find('/') else {
        return false;
    };
    if idx == 0 {
        return false;
    }
    let u = &u[idx + 1..]; // Eat owner

    let Some(idx) = u.find('/') else {
        return false;
    };
    if idx == 0 {
        return false;
    }
    let u = &u[idx + 1..]; // Eat repo

    let Some(idx) = u.find('@') else {
        return false;
    };
    if idx == 0 {
        return false;
    }
    let u = &u[idx + 1..]; // Eat workflow path

    !u.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Pos, String};

    #[test]
    fn name_and_description() {
        let r = RuleWorkflowCall::default();
        assert_eq!(r.name(), "workflow-call");
        assert!(!r.description().is_empty());
    }

    #[test]
    fn local_format() {
        assert!(is_workflow_call_uses_local_format(
            "./.github/workflows/w.yaml"
        ));
        assert!(is_workflow_call_uses_local_format("./w.yaml"));
        assert!(!is_workflow_call_uses_local_format("./w.yaml@v1"));
        assert!(!is_workflow_call_uses_local_format("./"));
        assert!(!is_workflow_call_uses_local_format("owner/repo/w.yaml@v1"));
    }

    #[test]
    fn repo_format() {
        assert!(is_workflow_call_uses_repo_format("owner/repo/w.yaml@v1"));
        assert!(is_workflow_call_uses_repo_format(
            "owner/repo/.github/w.yaml@main"
        ));
        assert!(!is_workflow_call_uses_repo_format("./w.yaml"));
        assert!(!is_workflow_call_uses_repo_format("owner/repo/w.yaml"));
        assert!(!is_workflow_call_uses_repo_format(".owner/repo/w.yaml@v1"));
        assert!(!is_workflow_call_uses_repo_format("owner/w.yaml@v1"));
    }

    #[test]
    fn invalid_uses_format() {
        let cache = Arc::new(LocalReusableWorkflowCache::new_null(None));
        let mut r = RuleWorkflowCall::new(std::path::PathBuf::from("w.yaml"), cache);
        let job = Job {
            id: String::at("test", Pos::new(1, 1)),
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
            workflow_call: Some(WorkflowCall {
                uses: String::at("invalid-uses", Pos::new(4, 10)),
                ..Default::default()
            }),
            snapshot: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("is not following the format"));
    }

    #[test]
    fn expression_uses_skipped() {
        let cache = Arc::new(LocalReusableWorkflowCache::new_null(None));
        let mut r = RuleWorkflowCall::new(std::path::PathBuf::from("w.yaml"), cache);
        let job = Job {
            id: String::at("test", Pos::new(1, 1)),
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
            workflow_call: Some(WorkflowCall {
                uses: String::at("${{ matrix.wf }}", Pos::new(4, 10)),
                ..Default::default()
            }),
            snapshot: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
