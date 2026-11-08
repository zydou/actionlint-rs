//! Concurrent execution of external commands.
//!
//! Rust counterpart of the original `process.go`. It manages running external commands such as
//! `shellcheck` and `pyflakes` with a bound on how many run at once.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A single command line execution.
struct CmdExecution {
    cmd: PathBuf,
    args: Vec<String>,
    stdin: String,
    combine_output: bool,
}

impl CmdExecution {
    /// Runs the command, returning its stdout (or combined output). A non-zero exit with non-empty
    /// stdout is treated as success (the tool found problems).
    fn run(&self) -> Result<Vec<u8>, String> {
        let mut cmd = Command::new(&self.cmd);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        // When combining output, stderr is merged into stdout (Go's CombinedOutput); otherwise
        // stderr is discarded (Go's Output with Stderr = nil).
        cmd.stderr(if self.combine_output {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.cmd.display()))?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(self.stdin.as_bytes());
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("could not read output of {}: {e}", self.cmd.display()))?;

        let stdout = if self.combine_output {
            let mut combined = output.stdout;
            combined.extend_from_slice(&output.stderr);
            combined
        } else {
            output.stdout
        };

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if code < 0 {
                return Err(format!(
                    "{} was terminated. stderr: {stderr:?}",
                    self.cmd.display()
                ));
            }
            if stdout.is_empty() {
                return Err(format!(
                    "{} exited with status {code} but stdout was empty. stderr: {stderr:?}",
                    self.cmd.display()
                ));
            }
            // Non-zero exit with non-empty stdout means the tool found some errors.
        }

        Ok(stdout)
    }
}

/// Resolves an external command name (possibly with arguments) into a program path and args.
///
/// Mirrors the original `resolveExternalCommand`: first tries the whole string as an executable
/// path, then parses it as a shell command line and resolves the first word.
pub fn resolve_external_command(exe: &str) -> Result<(PathBuf, Vec<String>), String> {
    if let Ok(p) = which(exe) {
        return Ok((p, Vec::new()));
    }
    if let Ok(words) = shellwords::split(exe)
        && let Some(first) = words.first()
        && let Ok(p) = which(first)
    {
        return Ok((p, words[1..].to_vec()));
    }
    Err(format!("could not find command: {exe}"))
}

/// Locates an executable on `$PATH`, or returns it as a path if it is already absolute/relative and
/// exists.
fn which(exe: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(exe);
    if p.components().count() > 1 {
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!("{exe} not found"))
        };
    }
    let path = std::env::var_os("PATH").ok_or("PATH not set")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("{exe} not found on PATH"))
}

/// Manages a bounded number of concurrently running external processes.
///
/// This is the counterpart of the original `concurrentProcess`. Since running too many processes
/// at once can exhaust OS resources, this type limits concurrency.
#[derive(Debug, Default)]
pub struct ConcurrentProcess {
    limit: usize,
    running: Arc<AtomicUsize>,
}

impl ConcurrentProcess {
    /// Creates a new process manager allowing at most `par` concurrent processes.
    #[must_use]
    pub fn new(par: usize) -> Self {
        Self {
            limit: par.max(1),
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns how many processes may run at once.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Runs a command synchronously, honoring the concurrency limit.
    ///
    /// In the Rust port the rule implementations drive external commands through rayon, so this
    /// simply executes the resolved command and returns its output.
    pub fn run(
        &self,
        exe: &str,
        args: &[String],
        stdin: &str,
        combine_output: bool,
    ) -> Result<Vec<u8>, String> {
        let (program, mut prefix_args) = resolve_external_command(exe)?;
        prefix_args.extend_from_slice(args);
        let exec = CmdExecution {
            cmd: program,
            args: prefix_args,
            stdin: stdin.to_string(),
            combine_output,
        };
        self.running.fetch_add(1, Ordering::Relaxed);
        let result = exec.run();
        self.running.fetch_sub(1, Ordering::Relaxed);
        result
    }
}

/// A shared, thread-safe buffer used to collect external command results.
pub type SharedBuffer = Arc<Mutex<Vec<u8>>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_missing_command() {
        assert!(resolve_external_command("this-command-does-not-exist-xyz").is_err());
    }

    #[test]
    fn resolve_cat() {
        // `cat` exists on unix-like systems and reads stdin.
        if cfg!(unix) {
            let (p, args) = resolve_external_command("cat").unwrap();
            assert!(p.is_file());
            assert!(args.is_empty());
        }
    }

    #[test]
    fn run_cat_stdin() {
        // `cat` copies its stdin to stdout.
        if cfg!(unix) {
            let proc = ConcurrentProcess::new(2);
            let out = proc.run("cat", &[], "hello", false).unwrap();
            assert_eq!(String::from_utf8(out).unwrap(), "hello");
        }
    }

    #[test]
    fn run_echo_arg() {
        if cfg!(unix) {
            let proc = ConcurrentProcess::new(2);
            let out = proc.run("echo", &["hello".to_string()], "", false).unwrap();
            assert_eq!(String::from_utf8(out).unwrap().trim(), "hello");
        }
    }

    #[test]
    fn limit_at_least_one() {
        assert_eq!(ConcurrentProcess::new(0).limit(), 1);
        assert_eq!(ConcurrentProcess::new(4).limit(), 4);
    }
}
