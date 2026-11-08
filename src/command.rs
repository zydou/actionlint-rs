//! The `actionlint` command line interface.
//!
//! Rust counterpart of the original `command.go`. It parses command line flags and drives the
//! [`crate::linter::Linter`].

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;

/// The exit status when the command ran successfully with no problem found.
pub const EXIT_STATUS_SUCCESS_NO_PROBLEM: u8 = 0;
/// The exit status when the command ran successfully with some problem found.
pub const EXIT_STATUS_SUCCESS_PROBLEM_FOUND: u8 = 1;
/// The exit status when parsing command line options failed.
pub const EXIT_STATUS_INVALID_COMMAND_OPTION: u8 = 2;
/// The exit status when the command stopped due to some fatal error while checking workflows.
pub const EXIT_STATUS_FAILURE: u8 = 3;

/// Returns the version string of the actionlint binary.
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Options controlling the behavior of the linter, gathered from command line flags.
///
/// This mirrors the original `LinterOptions`.
#[derive(Debug, Clone, Default)]
pub struct LinterOptions {
    /// Enable verbose log output.
    pub verbose: bool,
    /// Enable debug log output.
    pub debug: bool,
    /// Colorize error output: `None` means auto, `Some(true)` always, `Some(false)` never.
    pub color: Option<bool>,
    /// Emit one line per error.
    pub oneline: bool,
    /// Executable for running the `shellcheck` external command. Empty disables the integration.
    pub shellcheck: String,
    /// Executable for running the `pyflakes` external command. Empty disables the integration.
    pub pyflakes: String,
    /// Regular expressions to filter errors by matching their messages.
    pub ignore_patterns: Vec<String>,
    /// Path to a config file. Empty means actionlint tries to read `.github/actionlint.yaml`.
    pub config_file: String,
    /// Custom Go template to format error messages.
    pub format: String,
    /// File name used when reading input from stdin.
    pub stdin_file_name: String,
}

/// Represents the entire actionlint command. The given handles are used for input/output.
pub struct Command {
    stdout: Box<dyn Write>,
    stderr: Box<dyn Write>,
    stdin: Box<dyn Read>,
}

impl Command {
    /// Creates a new `Command` reading from `stdin` and writing to `stdout`/`stderr`.
    pub fn new<R, O, E>(stdin: R, stdout: O, stderr: E) -> Self
    where
        R: Read + 'static,
        O: Write + 'static,
        E: Write + 'static,
    {
        Self {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            stdin: Box::new(stdin),
        }
    }

    /// Runs the actionlint command with the given arguments (including the program name) and
    /// returns the exit status.
    pub fn main(&mut self, args: Vec<OsString>) -> u8 {
        match self.run(args) {
            Ok(status) => status,
            Err(status) => status,
        }
    }

    fn run(&mut self, args: Vec<OsString>) -> Result<u8, u8> {
        let cli = match parse_args(&args) {
            Ok(cli) => cli,
            Err(ParseError::Help) => return Ok(EXIT_STATUS_SUCCESS_NO_PROBLEM),
            Err(ParseError::Invalid(msg)) => {
                let _ = writeln!(self.stderr, "{msg}");
                return Err(EXIT_STATUS_INVALID_COMMAND_OPTION);
            }
        };

        if cli.version {
            let _ = writeln!(
                self.stdout,
                "{}\ninstalled by building from source\nbuilt with rustc for {}",
                version(),
                std::env::consts::OS
            );
            return Ok(EXIT_STATUS_SUCCESS_NO_PROBLEM);
        }

        if cli.init_config {
            return self.init_config();
        }

        self.run_linter(&cli)
    }

    fn init_config(&mut self) -> Result<u8, u8> {
        let cwd = std::env::current_dir().map_err(|_| EXIT_STATUS_FAILURE)?;
        let dir = cwd.join(".github");
        for f in ["actionlint.yaml", "actionlint.yml"] {
            let p = dir.join(f);
            if p.exists() {
                let _ = writeln!(self.stderr, "config file already exists at {}", p.display());
                return Err(EXIT_STATUS_FAILURE);
            }
        }
        let p = dir.join("actionlint.yaml");
        if let Err(e) = crate::config::write_default_config_file(&p) {
            let _ = writeln!(self.stderr, "{e}");
            return Err(EXIT_STATUS_FAILURE);
        }
        let _ = writeln!(self.stdout, "Config file was generated at {}", p.display());
        Ok(EXIT_STATUS_SUCCESS_NO_PROBLEM)
    }

    fn run_linter(&mut self, cli: &Cli) -> Result<u8, u8> {
        let opts = crate::linter::LinterOptions {
            verbose: cli.verbose,
            debug: cli.debug,
            color: cli.color,
            oneline: cli.oneline,
            shellcheck: cli.shellcheck.clone(),
            pyflakes: cli.pyflakes.clone(),
            ignore_patterns: cli.ignore.clone(),
            config_file: cli.config_file.clone(),
            format: cli.format.clone(),
            stdin_file_name: cli.stdin_filename.clone(),
        };

        let mut linter =
            crate::linter::Linter::new(Box::new(std::io::sink()), opts).map_err(|e| {
                let _ = writeln!(self.stderr, "{e}");
                EXIT_STATUS_FAILURE
            })?;

        let result = if cli.args.len() == 1 && cli.args[0] == "-" {
            let mut buf = String::new();
            if let Err(e) = self.stdin.read_to_string(&mut buf) {
                let _ = writeln!(self.stderr, "could not read stdin: {e}");
                return Err(EXIT_STATUS_FAILURE);
            }
            linter.lint_stdin(&buf)
        } else if cli.args.is_empty() {
            linter.lint_repository("")
        } else {
            let paths: Vec<PathBuf> = cli.args.iter().map(PathBuf::from).collect();
            linter.lint_files(&paths)
        };

        match result {
            Ok(errs) => {
                if errs.is_empty() {
                    Ok(EXIT_STATUS_SUCCESS_NO_PROBLEM)
                } else {
                    Ok(EXIT_STATUS_SUCCESS_PROBLEM_FOUND)
                }
            }
            Err(e) => {
                let _ = writeln!(self.stderr, "{e}");
                Err(EXIT_STATUS_FAILURE)
            }
        }
    }
}

struct Cli {
    args: Vec<String>,
    version: bool,
    init_config: bool,
    verbose: bool,
    debug: bool,
    oneline: bool,
    color: Option<bool>,
    shellcheck: String,
    pyflakes: String,
    ignore: Vec<String>,
    config_file: String,
    format: String,
    stdin_filename: String,
}

#[derive(Debug)]
enum ParseError {
    Help,
    Invalid(String),
}

/// Parses command line arguments into a [`Cli`] struct.
fn parse_args(args: &[OsString]) -> Result<Cli, ParseError> {
    let mut cli = Cli {
        args: Vec::new(),
        version: false,
        init_config: false,
        verbose: false,
        debug: false,
        oneline: false,
        color: None,
        shellcheck: "shellcheck".to_string(),
        pyflakes: "pyflakes".to_string(),
        ignore: Vec::new(),
        config_file: String::new(),
        format: String::new(),
        stdin_filename: "<stdin>".to_string(),
    };

    let mut it = args.iter().skip(1); // Skip program name
    while let Some(arg) = it.next() {
        let arg = arg.to_string_lossy();
        if !arg.starts_with('-') {
            cli.args.push(arg.into_owned());
            continue;
        }
        if arg == "-" {
            cli.args.push("-".to_string());
            continue;
        }

        let (name, inline_value) = match arg.split_once('=') {
            Some((n, v)) => (n.trim_start_matches('-').to_string(), Some(v.to_string())),
            None => (arg.trim_start_matches('-').to_string(), None),
        };

        let mut value = || -> Result<String, ParseError> {
            if let Some(v) = &inline_value {
                return Ok(v.clone());
            }
            it.next()
                .map(|v| v.to_string_lossy().into_owned())
                .ok_or_else(|| ParseError::Invalid(format!("flag needs an argument: -{name}")))
        };

        match name.as_str() {
            "h" | "help" => return Err(ParseError::Help),
            "version" => cli.version = true,
            "init-config" => cli.init_config = true,
            "verbose" => cli.verbose = true,
            "debug" => cli.debug = true,
            "oneline" => cli.oneline = true,
            "no-color" => cli.color = Some(false),
            "color" => cli.color = Some(true),
            "ignore" => cli.ignore.push(value()?),
            "shellcheck" => cli.shellcheck = value()?,
            "pyflakes" => cli.pyflakes = value()?,
            "config-file" => cli.config_file = value()?,
            "format" => cli.format = value()?,
            "stdin-filename" => cli.stdin_filename = value()?,
            _ => {
                return Err(ParseError::Invalid(format!(
                    "provided flag is not defined: -{name}"
                )));
            }
        }
    }

    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_flags() {
        let args: Vec<OsString> = ["actionlint", "-verbose", "-oneline", "foo.yaml"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert!(cli.verbose);
        assert!(cli.oneline);
        assert_eq!(cli.args, vec!["foo.yaml".to_string()]);
    }

    #[test]
    fn parse_ignore_repeatable() {
        let args: Vec<OsString> = ["actionlint", "-ignore", "a", "-ignore", "b"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.ignore, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn parse_inline_value() {
        let args: Vec<OsString> = ["actionlint", "-format={{json .}}"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.format, "{{json .}}");
    }

    #[test]
    fn parse_stdin() {
        let args: Vec<OsString> = ["actionlint", "-"].iter().map(OsString::from).collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.args, vec!["-".to_string()]);
    }

    #[test]
    fn parse_help() {
        let args: Vec<OsString> = ["actionlint", "-h"].iter().map(OsString::from).collect();
        assert!(matches!(parse_args(&args), Err(ParseError::Help)));
    }

    #[test]
    fn parse_unknown_flag() {
        let args: Vec<OsString> = ["actionlint", "-unknown"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(matches!(parse_args(&args), Err(ParseError::Invalid(_))));
    }

    #[test]
    fn color_flags() {
        let a: Vec<OsString> = ["actionlint", "-color"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse_args(&a).unwrap().color, Some(true));
        let b: Vec<OsString> = ["actionlint", "-no-color"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse_args(&b).unwrap().color, Some(false));
    }

    #[test]
    fn default_shellcheck() {
        let args: Vec<OsString> = ["actionlint"].iter().map(OsString::from).collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.shellcheck, "shellcheck");
        assert_eq!(cli.stdin_filename, "<stdin>");
    }
}
