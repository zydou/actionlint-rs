//! The `shell-name` rule.
//!
//! Checks: Checks for shell names used for scripts in "run:". Rust counterpart of the original
//! `rule_shell_name.go`. See
//! <https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#using-a-specific-shell>.

use crate::ast::{Job, Step, Workflow};
use crate::quotes::sorted_quotes;
use crate::rule::{Pass, Rule, RuleBase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformKind {
    Any,
    MacOrLinux,
    Windows,
}

/// The `shell-name` rule.
#[derive(Debug)]
pub struct RuleShellName {
    base: RuleBase,
    platform: PlatformKind,
}

impl RuleShellName {
    /// Creates a new `shell-name` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "shell-name",
                "Checks for shell names used for scripts in \"run:\"",
            ),
            platform: PlatformKind::Any,
        }
    }

    fn check_shell_name(&mut self, node: Option<&crate::ast::String>) {
        let Some(node) = node else {
            return;
        };

        // Ignore custom shell
        // https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#custom-shell
        if node.value.contains("{0}") {
            return;
        }

        // Ignore dynamic shell name
        if node.contains_expression() {
            return;
        }

        let name = node.value.to_lowercase();
        let available = get_available_shell_names(self.platform);

        if available.contains(&name.as_str()) {
            return; // ok
        }

        let on_platform = match self.platform {
            PlatformKind::Windows => {
                // only when the shell is unavailable on Windows
                if get_available_shell_names(PlatformKind::Any).contains(&name.as_str()) {
                    " on Windows"
                } else {
                    ""
                }
            }
            PlatformKind::MacOrLinux => {
                // only when the shell is unavailable on macOS or Linux
                if get_available_shell_names(PlatformKind::Any).contains(&name.as_str()) {
                    " on macOS or Linux"
                } else {
                    ""
                }
            }
            PlatformKind::Any => "",
        };

        self.base.error_at(
            node.pos,
            format!(
                "shell name {:?} is invalid{}. available names are {}",
                node.value,
                on_platform,
                sorted_quotes(available.to_vec())
            ),
        );
    }

    fn get_platform_from_runner(runner: Option<&crate::ast::Runner>) -> PlatformKind {
        let Some(runner) = runner else {
            return PlatformKind::Any;
        };

        // Note: Labels for self-hosted runners:
        // https://docs.github.com/en/actions/hosting-your-own-runners/using-labels-with-self-hosted-runners

        let mut ret = PlatformKind::Any;
        for label in &runner.labels {
            let k = {
                let l = label.value.to_lowercase();
                if l.starts_with("windows-") || l == "windows" {
                    PlatformKind::Windows
                } else if l.starts_with("macos-")
                    || l.starts_with("ubuntu-")
                    || l == "macos"
                    || l == "linux"
                {
                    PlatformKind::MacOrLinux
                } else {
                    PlatformKind::Any
                }
            };

            if k == PlatformKind::Any {
                continue;
            }
            if ret != PlatformKind::Any && ret != k {
                // Conflicts are reported by runner-label rule so simply ignore here
                return PlatformKind::Any;
            }
            ret = k;
        }

        ret
    }
}

impl Default for RuleShellName {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleShellName {
    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        if let crate::ast::Exec::Run(run) = &n.exec {
            self.check_shell_name(run.shell.as_ref());
        }
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        if n.runs_on.is_none() {
            return Ok(());
        }
        self.platform = Self::get_platform_from_runner(n.runs_on.as_ref());
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
        {
            self.check_shell_name(run.shell.as_ref());
        }
        Ok(())
    }

    fn visit_job_post(&mut self, _n: &Job) -> Result<(), String> {
        self.platform = PlatformKind::Any; // Clear
        Ok(())
    }

    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
        {
            self.check_shell_name(run.shell.as_ref());
        }
        Ok(())
    }
}

impl Rule for RuleShellName {
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

/// The list of available shell names for the given platform. Counterpart of `getAvailableShellNames`.
fn get_available_shell_names(kind: PlatformKind) -> &'static [&'static str] {
    // https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#using-a-specific-shell
    match kind {
        PlatformKind::Any => &["bash", "pwsh", "python", "sh", "cmd", "powershell"],
        PlatformKind::Windows => &["bash", "pwsh", "python", "cmd", "powershell"],
        PlatformKind::MacOrLinux => &["bash", "pwsh", "python", "sh"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Defaults, DefaultsRun, Exec, ExecRun, Pos, String};

    fn run_step(shell: Option<&str>) -> Step {
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: String::at("echo", Pos::new(5, 9)),
                shell: shell.map(|s| String::at(s, Pos::new(5, 16))),
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
    fn valid_shell() {
        let mut r = RuleShellName::new();
        r.visit_step(&run_step(Some("bash"))).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn invalid_shell() {
        let mut r = RuleShellName::new();
        r.visit_step(&run_step(Some("foobar"))).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("shell name \"foobar\" is invalid"));
        assert!(errs[0].message.contains("available names are"));
    }

    #[test]
    fn custom_shell_ignored() {
        let mut r = RuleShellName::new();
        r.visit_step(&run_step(Some("perl {0}"))).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn expression_shell_ignored() {
        let mut r = RuleShellName::new();
        r.visit_step(&run_step(Some("${{ matrix.shell }}")))
            .unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn windows_only_shell_on_linux() {
        let mut r = RuleShellName::new();
        let mut job = Job {
            id: String::at("test", Pos::new(3, 3)),
            name: None,
            needs: vec![],
            runs_on: Some(crate::ast::Runner {
                labels: vec![String::at("ubuntu-latest", Pos::new(4, 12))],
                labels_expr: None,
                group: None,
            }),
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![run_step(Some("cmd"))],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(3, 3),
        };
        r.visit_job_pre(&job).unwrap();
        r.visit_step(&job.steps[0]).unwrap();
        r.visit_job_post(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains(" on macOS or Linux"));

        // On Windows the same shell is valid.
        let mut r2 = RuleShellName::new();
        job.runs_on = Some(crate::ast::Runner {
            labels: vec![String::at("windows-latest", Pos::new(4, 12))],
            labels_expr: None,
            group: None,
        });
        r2.visit_job_pre(&job).unwrap();
        r2.visit_step(&job.steps[0]).unwrap();
        assert!(r2.take_errors().is_empty());
    }

    #[test]
    fn workflow_defaults_shell() {
        let mut r = RuleShellName::new();
        let wf = Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: None,
            defaults: Some(Defaults {
                run: Some(DefaultsRun {
                    shell: Some(String::at("foobar", Pos::new(3, 11))),
                    working_directory: None,
                    pos: Pos::new(3, 9),
                }),
                pos: Pos::new(3, 5),
            }),
            concurrency: None,
            jobs: Default::default(),
        };
        r.visit_workflow_pre(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("foobar"));
    }
}
