//! The `pyflakes` rule.
//!
//! Integrates the external `pyflakes` command to check Python scripts at `run:` when
//! `shell: python` is configured. Rust counterpart of the original `rule_pyflakes.go`. See
//! <https://github.com/PyCQA/pyflakes>.

use crate::ast::{Exec, ExecRun, Job, Step, Workflow};
use crate::process::{ConcurrentProcess, resolve_external_command};
use crate::rule::{Pass, Rule, RuleBase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellIsPythonKind {
    Unspecified,
    Python,
    NotPython,
}

fn get_shell_is_python_kind(shell: Option<&crate::ast::String>) -> ShellIsPythonKind {
    let Some(shell) = shell else {
        return ShellIsPythonKind::Unspecified;
    };
    if shell.value == "python" || shell.value.starts_with("python ") {
        ShellIsPythonKind::Python
    } else {
        ShellIsPythonKind::NotPython
    }
}

/// The `pyflakes` rule.
#[derive(Debug)]
pub struct RulePyflakes {
    base: RuleBase,
    executable: String,
    proc: ConcurrentProcess,
    workflow_shell_is_python: ShellIsPythonKind,
    job_shell_is_python: ShellIsPythonKind,
}

impl RulePyflakes {
    /// Creates a new `pyflakes` rule using the given executable. The executable argument can be a
    /// command name or a relative/absolute file path. When the given executable is not found in the
    /// system, it returns an error.
    pub fn new(executable: &str) -> Result<Self, String> {
        resolve_external_command(executable)?;
        // Combine output because pyflakes outputs lint errors to stdout and outputs syntax errors to
        // stderr. (#411)
        Ok(Self {
            base: RuleBase::new(
                "pyflakes",
                "Checks for Python script when \"shell: python\" is configured using Pyflakes",
            ),
            executable: executable.to_string(),
            proc: ConcurrentProcess::new(1),
            workflow_shell_is_python: ShellIsPythonKind::Unspecified,
            job_shell_is_python: ShellIsPythonKind::Unspecified,
        })
    }

    fn is_python_shell(&self, r: &ExecRun) -> bool {
        let k = get_shell_is_python_kind(r.shell.as_ref());
        if k != ShellIsPythonKind::Unspecified {
            return k == ShellIsPythonKind::Python;
        }

        if self.job_shell_is_python != ShellIsPythonKind::Unspecified {
            return self.job_shell_is_python == ShellIsPythonKind::Python;
        }

        self.workflow_shell_is_python == ShellIsPythonKind::Python
    }

    fn run_pyflakes(&mut self, src: &str, pos: crate::ast::Pos) {
        // Defined in rule_shellcheck.go.
        let src = crate::rule_shellcheck::sanitize_expressions_in_script(src);
        self.base.debug(format_args!(
            "{pos}: Running {} for Python script:\n{src}",
            self.executable
        ));

        let stdout = match self.proc.run(&self.executable, &[], &src, true) {
            Ok(o) => o,
            Err(e) => {
                self.base
                    .debug(format_args!("Command {} failed: {e}", self.executable));
                self.base.error_at(
                    pos,
                    format!(
                        "`{}` did not run successfully while checking script at {pos}: {e}",
                        self.executable
                    ),
                );
                return;
            }
        };
        if stdout.is_empty() {
            return;
        }

        let mut rest: &[u8] = &stdout;
        while !rest.is_empty() {
            match self.parse_next_error(rest, pos) {
                Ok(Some(next)) => rest = next,
                Ok(None) => break,
                Err(e) => {
                    self.base.error_at(pos, e);
                    return;
                }
            }
        }
    }

    /// Parses the next error message from the pyflakes output. Returns `Ok(None)` when no more error
    /// messages are found. Counterpart of `parseNextError`.
    fn parse_next_error<'a>(
        &mut self,
        stdout: &'a [u8],
        pos: crate::ast::Pos,
    ) -> Result<Option<&'a [u8]>, String> {
        let mut b = stdout;

        // Search the start of the error message.
        let Some(idx) = find_subslice(b, b"<stdin>:") else {
            // Syntax errors from pyflakes consist of multiple lines. Skip subsequent lines. (#411)
            //
            // ```text
            // <stdin>:1:7: unexpected EOF while parsing
            // print(
            //       ^
            // ```
            return Ok(None);
        };
        b = &b[idx + b"<stdin>:".len()..];

        let Some(idx) = b.iter().position(|&c| c == b'\n') else {
            return Err(format!(
                "error message from pyflakes does not end with \\n nor \\r\\n while checking script at {pos}. output: {:?}",
                String::from_utf8_lossy(stdout)
            ));
        };

        let mut msg = &b[..idx];
        if let Some(i) = msg.len().checked_sub(1)
            && msg[i] == b'\r'
        {
            msg = &msg[..i];
        }
        let rest = &b[idx + 1..];

        let msg = String::from_utf8_lossy(msg).into_owned();
        self.base.error_at(
            pos,
            format!("pyflakes reported issue in this script: {msg}"),
        );

        Ok(Some(rest))
    }
}

/// Finds the first occurrence of `needle` in `haystack`, returning its byte index.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

impl Pass for RulePyflakes {
    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
        {
            self.job_shell_is_python = get_shell_is_python_kind(run.shell.as_ref());
        }
        Ok(())
    }

    fn visit_job_post(&mut self, _n: &Job) -> Result<(), String> {
        self.job_shell_is_python = ShellIsPythonKind::Unspecified; // reset
        Ok(())
    }

    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), String> {
        if let Some(d) = &n.defaults
            && let Some(run) = &d.run
        {
            self.workflow_shell_is_python = get_shell_is_python_kind(run.shell.as_ref());
        }
        Ok(())
    }

    fn visit_workflow_post(&mut self, _n: &Workflow) -> Result<(), String> {
        self.workflow_shell_is_python = ShellIsPythonKind::Unspecified; // reset
        Ok(())
    }

    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        let Exec::Run(run) = &n.exec else {
            return Ok(());
        };

        if !self.is_python_shell(run) {
            return Ok(());
        }

        self.run_pyflakes(&run.run.value, run.run_pos);
        Ok(())
    }
}

impl Rule for RulePyflakes {
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
        assert!(RulePyflakes::new("this-does-not-exist-xyz").is_err());
    }

    #[test]
    fn python_shell_detected() {
        assert_eq!(
            get_shell_is_python_kind(Some(&String::at("python", Pos::new(1, 1)))),
            ShellIsPythonKind::Python
        );
        assert_eq!(
            get_shell_is_python_kind(Some(&String::at("python -v", Pos::new(1, 1)))),
            ShellIsPythonKind::Python
        );
        assert_eq!(
            get_shell_is_python_kind(Some(&String::at("bash", Pos::new(1, 1)))),
            ShellIsPythonKind::NotPython
        );
        assert_eq!(
            get_shell_is_python_kind(None),
            ShellIsPythonKind::Unspecified
        );
    }

    #[test]
    fn non_python_shell_skipped() {
        let mut r = RulePyflakes::new("cat").unwrap();
        let s = run_step("print('hi')", Some("bash"));
        r.visit_step(&s).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn find_subslice_basic() {
        assert_eq!(find_subslice(b"hello <stdin>:1:2", b"<stdin>:"), Some(6));
        assert_eq!(find_subslice(b"no match", b"<stdin>:"), None);
        assert_eq!(find_subslice(b"", b"<stdin>:"), None);
    }
}
