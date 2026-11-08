//! The `glob` rule.
//!
//! Checks: Checks for glob syntax used in branch names, tags, and paths. Rust counterpart of the
//! original `rule_glob.go`. See
//! <https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions#filter-pattern-cheat-sheet>.

use crate::ast::{Event, Job, Workflow};
use crate::glob::{InvalidGlobPattern, validate_path_glob, validate_ref_glob};
use crate::rule::{Pass, Rule, RuleBase};

/// The `glob` rule.
#[derive(Debug)]
pub struct RuleGlob {
    base: RuleBase,
}

impl RuleGlob {
    /// Creates a new `glob` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "glob",
                "Checks for glob syntax used in branch names, tags, and paths",
            ),
        }
    }

    fn check_ref_glob(&mut self, s: &crate::ast::String) {
        // Empty value is already checked by the parser. Avoid duplicate errors.
        if s.value.is_empty() {
            return;
        }
        self.glob_errors(validate_ref_glob(&s.value), s.pos, s.quoted);
    }

    fn check_git_ref_globs(&mut self, filter: Option<&crate::ast::WebhookEventFilter>) {
        let Some(filter) = filter else {
            return;
        };
        for v in &filter.values {
            self.check_ref_glob(v);
        }
    }

    fn check_file_path_globs(&mut self, filter: Option<&crate::ast::WebhookEventFilter>) {
        let Some(filter) = filter else {
            return;
        };
        for v in &filter.values {
            // Empty value is already checked by the parser. Avoid duplicate errors.
            if !v.value.is_empty() {
                self.glob_errors(validate_path_glob(&v.value), v.pos, v.quoted);
            }
        }
    }

    fn glob_errors(&mut self, errs: Vec<InvalidGlobPattern>, pos: crate::ast::Pos, quoted: bool) {
        for err in errs {
            let mut p = pos;
            if quoted {
                p.col += 1;
            }
            if err.column != 0 {
                p.col += err.column - 1;
            }
            self.base.error_at(
                p,
                format!(
                    "{}. note: filter pattern syntax is explained at https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions#filter-pattern-cheat-sheet",
                    err.message
                ),
            );
        }
    }
}

impl Default for RuleGlob {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleGlob {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        for e in &n.on {
            match e {
                Event::Webhook(e) => {
                    self.check_git_ref_globs(e.branches.as_ref());
                    self.check_git_ref_globs(e.branches_ignore.as_ref());
                    self.check_git_ref_globs(e.tags.as_ref());
                    self.check_git_ref_globs(e.tags_ignore.as_ref());
                    self.check_file_path_globs(e.paths.as_ref());
                    self.check_file_path_globs(e.paths_ignore.as_ref());
                }
                Event::ImageVersion(e) => {
                    for v in &e.versions {
                        self.check_ref_glob(v);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        if let Some(snapshot) = &n.snapshot
            && let Some(version) = &snapshot.version
        {
            self.check_ref_glob(version);
        }
        Ok(())
    }
}

impl Rule for RuleGlob {
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
    use crate::ast::{Pos, String, WebhookEvent, WebhookEventFilter};

    fn wf_with_branches(values: Vec<String>) -> Workflow {
        let e = WebhookEvent {
            hook: String::at("push", Pos::new(1, 1)),
            types: vec![],
            branches: Some(WebhookEventFilter {
                name: String::at("branches", Pos::new(1, 1)),
                values,
            }),
            branches_ignore: None,
            tags: None,
            tags_ignore: None,
            paths: None,
            paths_ignore: None,
            workflows: vec![],
            pos: Pos::new(1, 1),
        };
        Workflow {
            name: None,
            run_name: None,
            on: vec![Event::Webhook(e)],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        }
    }

    #[test]
    fn invalid_branch_glob() {
        let mut r = RuleGlob::new();
        let wf = wf_with_branches(vec![String::at("[invalid", Pos::new(2, 5))]);
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("filter pattern syntax"));
    }

    #[test]
    fn valid_branch_glob() {
        let mut r = RuleGlob::new();
        let wf = wf_with_branches(vec![String::at("refs/heads/main", Pos::new(2, 5))]);
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn empty_glob_skipped() {
        let mut r = RuleGlob::new();
        let wf = wf_with_branches(vec![String::at("", Pos::new(2, 5))]);
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
