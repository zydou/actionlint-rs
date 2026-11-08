//! The `permissions` rule.
//!
//! Checks: Checks for permissions configuration in "permissions:". Permission names and permission
//! scopes are checked. Rust counterpart of the original `rule_permissions.go`. See
//! <https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#defining-access-for-the-github_token-scopes>.

use std::collections::BTreeMap;

use crate::ast::{Job, Permissions, Workflow};
use crate::quotes::{quotes, sorted_quotes};
use crate::rule::{Pass, Rule, RuleBase};

/// The map from permission scope name to the list of available permission values for that scope.
fn all_permission_scopes() -> &'static BTreeMap<&'static str, &'static [&'static str]> {
    static MAP: std::sync::OnceLock<BTreeMap<&'static str, &'static [&'static str]>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        BTreeMap::from([
            ("actions", &["read", "write", "none"][..]),
            ("artifact-metadata", &["read", "write", "none"][..]),
            ("attestations", &["read", "write", "none"][..]),
            ("checks", &["read", "write", "none"][..]),
            ("contents", &["read", "write", "none"][..]),
            ("deployments", &["read", "write", "none"][..]),
            ("discussions", &["read", "write", "none"][..]),
            ("id-token", &["write", "none"][..]),
            ("issues", &["read", "write", "none"][..]),
            ("models", &["read", "none"][..]),
            ("packages", &["read", "write", "none"][..]),
            ("pages", &["read", "write", "none"][..]),
            ("pull-requests", &["read", "write", "none"][..]),
            ("repository-projects", &["read", "write", "none"][..]),
            ("security-events", &["read", "write", "none"][..]),
            ("statuses", &["read", "write", "none"][..]),
        ])
    })
}

/// The `permissions` rule.
#[derive(Debug)]
pub struct RulePermissions {
    base: RuleBase,
}

impl RulePermissions {
    /// Creates a new `permissions` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "permissions",
                "Checks for permissions configuration in \"permissions:\". Permission names and permission scopes are checked",
            ),
        }
    }

    fn check_permissions(&mut self, p: Option<&Permissions>) {
        let Some(p) = p else {
            return;
        };

        if let Some(all) = &p.all {
            match all.value.as_str() {
                "write-all" | "read-all" => {}
                _ => {
                    self.base.error_at(
                        all.pos,
                        format!(
                            "{:?} is invalid for permission for all the scopes. available values are \"read-all\", \"write-all\" or {{}}",
                            all.value
                        ),
                    );
                }
            }
            return;
        }

        for scope in p.scopes.values() {
            let n = scope.name.as_str(); // Permission names are case-sensitive
            let Some(s) = all_permission_scopes().get(n) else {
                let ss: Vec<&str> = all_permission_scopes().keys().copied().collect();
                self.base.error_at(
                    scope.name.pos,
                    format!(
                        "unknown permission scope {:?}. all available permission scopes are {}",
                        n,
                        sorted_quotes(ss)
                    ),
                );
                continue;
            };

            if !s.contains(&scope.value.as_str()) {
                self.base.error_at(
                    scope.value.pos,
                    format!(
                        "{:?} is invalid as permission of scope {:?}. available values are {}",
                        scope.value.as_str(),
                        n,
                        quotes(s)
                    ),
                );
            }
        }
    }
}

impl Default for RulePermissions {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RulePermissions {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        self.check_permissions(n.permissions.as_ref());
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        self.check_permissions(n.permissions.as_ref());
        Ok(())
    }
}

impl Rule for RulePermissions {
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
    use crate::ast::{PermissionScope, Pos, String};
    use std::collections::BTreeMap;

    fn perms_all(value: &str) -> Permissions {
        Permissions {
            all: Some(String::at(value, Pos::new(2, 3))),
            scopes: BTreeMap::new(),
            pos: Pos::new(2, 3),
        }
    }

    fn perms_scopes(scopes: Vec<(&str, &str)>) -> Permissions {
        let mut m = BTreeMap::new();
        for (name, value) in scopes {
            m.insert(
                name.to_string(),
                PermissionScope {
                    name: String::at(name, Pos::new(3, 5)),
                    value: String::at(value, Pos::new(3, 5)),
                },
            );
        }
        Permissions {
            all: None,
            scopes: m,
            pos: Pos::new(2, 3),
        }
    }

    #[test]
    fn invalid_all_value() {
        let mut r = RulePermissions::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: Some(perms_all("invalid")),
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("is invalid for permission for all the scopes")
        );
    }

    #[test]
    fn valid_all_value() {
        let mut r = RulePermissions::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: Some(perms_all("read-all")),
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn unknown_scope() {
        let mut r = RulePermissions::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: Some(perms_scopes(vec![("unknown-scope", "read")])),
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unknown permission scope"));
    }

    #[test]
    fn invalid_scope_value() {
        let mut r = RulePermissions::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: Some(perms_scopes(vec![("contents", "invalid")])),
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("is invalid as permission of scope")
        );
    }

    #[test]
    fn valid_scope() {
        let mut r = RulePermissions::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: Some(perms_scopes(vec![("contents", "read")])),
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
