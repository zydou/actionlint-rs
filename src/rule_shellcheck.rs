//! The `shellcheck` rule.
//!
//! Integrates the external `shellcheck` command to check `run:` shell scripts. Rust counterpart of
//! the original `rule_shellcheck.go`. See <https://github.com/koalaman/shellcheck>.

use serde::Deserialize;

use crate::ast::{Exec, ExecRun, Job, Step, Workflow};
use crate::process::{ConcurrentProcess, resolve_external_command};
use crate::rule::{Pass, Rule, RuleBase};

/// A single issue reported by `shellcheck` in its JSON output format.
#[derive(Debug, Deserialize)]
struct ShellcheckError {
    line: usize,
    column: usize,
    level: String,
    code: i64,
    message: String,
}

/// The `shellcheck` rule.
#[derive(Debug)]
pub struct RuleShellcheck {
    base: RuleBase,
    executable: String,
    proc: ConcurrentProcess,
    workflow_shell: String,
    job_shell: String,
    runner_shell: String,
}

impl RuleShellcheck {
    /// Creates a new `shellcheck` rule using the given executable. The executable argument can be a
    /// command name or a relative/absolute file path. When the given executable is not found in the
    /// system, it returns an error.
    pub fn new(executable: &str) -> Result<Self, String> {
        resolve_external_command(executable)?;
        Ok(Self {
            base: RuleBase::new(
                "shellcheck",
                "Checks for shell script sources in \"run:\" using shellcheck",
            ),
            executable: executable.to_string(),
            proc: ConcurrentProcess::new(1),
            workflow_shell: String::new(),
            job_shell: String::new(),
            runner_shell: String::new(),
        })
    }

    fn get_shell_name(&self, exec: &ExecRun) -> String {
        if let Some(s) = &exec.shell {
            return s.value.clone();
        }
        if !self.job_shell.is_empty() {
            return self.job_shell.clone();
        }
        if !self.workflow_shell.is_empty() {
            return self.workflow_shell.clone();
        }
        if !self.runner_shell.is_empty() {
            return self.runner_shell.clone();
        }
        // Note: Default shell on Windows is pwsh so this value is not always correct.
        // Note: When bash is not found, GitHub-hosted runner fallbacks to sh.
        "bash".to_string()
    }

    /// Runs `shellcheck` on `src` and reports any issues as lint errors.
    ///
    /// A failure to execute `shellcheck` or to parse its JSON output is a **fatal** error (returned
    /// as `Err`), mirroring the original where the run callback returns an error that propagates up
    /// through the visitor and aborts the lint run (exit status 3) rather than being reported as a
    /// per-position lint error.
    fn run_shellcheck(
        &mut self,
        src: &str,
        shell: &str,
        pos: crate::ast::Pos,
    ) -> Result<(), String> {
        let sh = if shell == "bash" || shell == "sh" {
            shell.to_string()
        } else if shell.starts_with("bash ") {
            "bash".to_string()
        } else if shell.starts_with("sh ") {
            "sh".to_string()
        } else {
            return Ok(()); // Skip checking this shell script since shellcheck doesn't support it
        };

        let src = sanitize_expressions_in_script(src);
        self.base.debug(format_args!(
            "{pos}: Run shellcheck for {sh} script:\n{src}"
        ));

        // Reasons to exclude the rules:
        //
        // - SC1091: File not found. Scripts are for CI environment.
        // - SC2194: The word is constant. Happens by replacing ${{ }} with underscores.
        // - SC2050: The expression is constant. Happens at `if` condition by replacing ${{ }} (#45).
        // - SC2153: Same as SC2154.
        // - SC2154: The var is referenced but not assigned. `run:` can refer `env:` variables (#53).
        // - SC2157: Argument to -z is always false due to literal strings (#113).
        // - SC2043: Loop detected as only running once when target is a placeholder (#355).
        let args: Vec<String> = [
            "--norc",
            "-f",
            "json",
            "-x",
            "--shell",
            &sh,
            "-e",
            "SC1091,SC2194,SC2050,SC2153,SC2154,SC2157,SC2043",
            "-",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

        // Use the same options to run the shell process described in the document.
        // https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#using-a-specific-shell
        let setup = if sh == "bash" {
            "set -eo pipefail"
        } else {
            "set -e"
        };
        let script = format!("{setup}\n{src}\n");

        let stdout = match self.proc.run(&self.executable, &args, &script, false) {
            Ok(o) => o,
            Err(e) => {
                self.base.debug(format_args!(
                    "Command {} {args:?} failed: {e}",
                    self.executable
                ));
                return Err(format!(
                    "`{} {}` did not run successfully while checking script at {pos}: {e}",
                    self.executable,
                    args.join(" ")
                ));
            }
        };

        let errs: Vec<ShellcheckError> = serde_json::from_slice(&stdout).map_err(|e| {
            format!(
                "could not parse JSON output from shellcheck: {e}: stdout={:?}",
                String::from_utf8_lossy(&stdout)
            )
        })?;
        if errs.is_empty() {
            return Ok(());
        }

        // It's better to show the source location in the script as the position of the error, but
        // it's not easy. YAML has multiple block styles with '|', '>', '|+', '>+', '|-', '>-'. Some
        // of them remove indentation and/or blank lines. So restoring the source position in a block
        // string is not possible. Instead, actionlint shows the position of 'run:' as the position of
        // the error, and separately shows the location in the script reported by shellcheck.
        for err in errs {
            // Consider the first line is setup for running the shell which was implicitly added.
            let line = err.line - 1;
            // Trim a trailing period to align with the style of the other error messages.
            let msg = err.message.strip_suffix('.').unwrap_or(&err.message);
            self.base.error_at(
                pos,
                format!(
                    "shellcheck reported issue in this script: SC{}:{}:{}:{}: {msg}",
                    err.code, err.level, line, err.column
                ),
            );
        }
        Ok(())
    }
}

/// Replaces `${{ ... }}` with underscores like `__________`.
///
/// Note: replacing with spaces sometimes causes a syntax error. For example,
///
/// ```sh
/// if ${{ contains(xs, s) }}; then
///   echo 'hello'
/// fi
/// ```
///
/// Counterpart of `sanitizeExpressionsInScript`.
pub fn sanitize_expressions_in_script(src: &str) -> String {
    let mut b = String::new();
    let mut rest = src;
    loop {
        let Some(s) = rest.find("${{") else {
            if b.is_empty() {
                return rest.to_string();
            }
            b.push_str(rest);
            return b;
        };

        let Some(e) = rest[s..].find("}}") else {
            if b.is_empty() {
                return rest.to_string();
            }
            b.push_str(rest);
            return b;
        };
        let e = e + s + 2; // 2 is the offset for len("}}")

        // Note: If ${{ ... }} includes a newline, the line and column reported by shellcheck will
        // be shifted.
        b.push_str(&rest[..s]);
        for _ in 0..(e - s) {
            b.push('_');
        }

        rest = &rest[e..];
    }
}

impl Pass for RuleShellcheck {
    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        let Exec::Run(run) = &n.exec else {
            return Ok(());
        };
        let shell = self.get_shell_name(run);
        self.run_shellcheck(&run.run.value, &shell, run.run_pos)
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
            && let Some(s) = &run.shell
        {
            self.job_shell = s.value.clone();
        }

        if let Some(r) = &n.runs_on {
            for label in &r.labels {
                let l = label.value.to_lowercase();
                // Default shell on Windows is PowerShell.
                // https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#using-a-specific-shell
                if l == "windows" || l.starts_with("windows-") {
                    self.runner_shell = "pwsh".to_string();
                    break;
                }
            }
        }

        Ok(())
    }

    fn visit_job_post(&mut self, _n: &Job) -> Result<(), String> {
        self.job_shell.clear();
        self.runner_shell.clear();
        Ok(())
    }

    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
            && let Some(s) = &run.shell
        {
            self.workflow_shell = s.value.clone();
        }
        Ok(())
    }

    fn visit_workflow_post(&mut self, _n: &Workflow) -> Result<(), String> {
        self.workflow_shell.clear();
        Ok(())
    }
}

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
    use crate::ast::{Pos, String};

    fn run_step(run: &str, shell: Option<&str>) -> Step {
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: String::at(run, Pos::new(5, 9)),
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
    fn missing_executable_errors() {
        assert!(RuleShellcheck::new("this-does-not-exist-xyz").is_err());
    }

    #[test]
    fn sanitize_no_expression() {
        assert_eq!(sanitize_expressions_in_script("echo hello"), "echo hello");
    }

    #[test]
    fn sanitize_single_expression() {
        assert_eq!(
            sanitize_expressions_in_script("echo ${{ matrix.os }}"),
            "echo ________________"
        );
    }

    #[test]
    fn sanitize_expression_in_if() {
        // The whole `${{ ... }}` including `${{` and `}}` is replaced with underscores.
        let src = "if ${{ contains(xs, s) }}; then\n  echo hi\nfi";
        let out = sanitize_expressions_in_script(src);
        assert!(out.contains("if __"));
        assert!(!out.contains("${{"));
    }

    #[test]
    fn sanitize_unterminated_expression() {
        // No closing `}}` — return as-is.
        assert_eq!(
            sanitize_expressions_in_script("echo ${{ foo"),
            "echo ${{ foo"
        );
    }

    #[test]
    fn unsupported_shell_skipped() {
        // Without a real shellcheck binary this only verifies the shell-name gate. We use `cat` as a
        // stand-in executable so `new` succeeds, but the JSON parse of its output will fail and be
        // reported rather than crashing. Instead just check that a python shell is skipped silently
        // (no error, no panic) by running with a script under `python` shell.
        let mut r = RuleShellcheck::new("cat").unwrap();
        let s = run_step("print('hi')", Some("python"));
        r.visit_step(&s).unwrap();
        // `python` shell is not bash/sh so it is skipped; no errors collected.
        assert!(r.take_errors().is_empty());
    }
}
