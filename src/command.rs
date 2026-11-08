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
            Err(ParseError::Help) => {
                self.print_usage();
                return Ok(EXIT_STATUS_SUCCESS_NO_PROBLEM);
            }
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

    /// Prints the usage header and flag descriptions to stderr. Mirrors the original `printUsageHeader`
    /// and the Go `flag` package's `-h` handling.
    fn print_usage(&mut self) {
        let v = version();
        // The docs links embed the release tag when the version looks like a semver tag.
        let b = if regex::Regex::new(r"^\d+\.\d+\.\d+$").is_ok_and(|re| re.is_match(&v)) {
            format!("v{v}")
        } else {
            "main".to_string()
        };
        let _ = write!(
            self.stderr,
            "Usage: actionlint [FLAGS] [FILES...] [-]

  actionlint is a linter for GitHub Actions workflow files.

  To check all YAML files in current repository, just run actionlint without
  arguments. It automatically finds the nearest '.github/workflows' directory:

    $ actionlint

  To check specific files, pass the file paths as arguments:

    $ actionlint file1.yaml file2.yaml

  To check content which is not saved in file yet (e.g. output from some
  command), pass - argument. It reads stdin and checks it as workflow file:

    $ actionlint -

  To serialize errors into JSON, use -format option. It allows to format error
  messages flexibly with Go template syntax.

    $ actionlint -format '{{json .}}'

Documents:

  - List of checks: https://github.com/rhysd/actionlint/tree/{b}/docs/checks.md
  - Usage:          https://github.com/rhysd/actionlint/tree/{b}/docs/usage.md
  - Configuration:  https://github.com/rhysd/actionlint/tree/{b}/docs/config.md

Flags:
"
        );
        let _ = write!(
            self.stderr,
            "  -h, -help\n    \tshow this help message and exit\n"
        );
        let _ = write!(
            self.stderr,
            "  -color\n    \tAlways enable colorful output. This is useful to force colorful outputs\n"
        );
        let _ = write!(
            self.stderr,
            "  -config-file string\n    \tFile path to config file\n"
        );
        let _ = write!(
            self.stderr,
            "  -debug\n    \tEnable debug output (for development)\n"
        );
        let _ = write!(
            self.stderr,
            "  -format string\n    \tCustom template to format error messages in Go template syntax. See the usage documentation for more details\n"
        );
        let _ = write!(
            self.stderr,
            "  -ignore value\n    \tRegular expression to filter errors. The pattern is matched with the error messages. This flag can be used multiple times\n"
        );
        let _ = write!(
            self.stderr,
            "  -init-config\n    \tGenerate default config file at .github/actionlint.yaml in current project\n"
        );
        let _ = write!(self.stderr, "  -no-color\n    \tDisable colorful output\n");
        let _ = write!(
            self.stderr,
            "  -oneline\n    \tUse one line per error for the error messages\n"
        );
        let _ = write!(
            self.stderr,
            "  -pyflakes string\n    \tFile path to the pyflakes command. To disable the integration, set empty string (default \"pyflakes\")\n"
        );
        let _ = write!(
            self.stderr,
            "  -shellcheck string\n    \tFile path to the shellcheck command. To disable the integration, set empty string (default \"shellcheck\")\n"
        );
        let _ = write!(
            self.stderr,
            "  -stdin-filename string\n    \tFile name when reading input from stdin (default \"<stdin>\")\n"
        );
        let _ = write!(self.stderr, "  -verbose\n    \tEnable verbose output\n");
        let _ = write!(
            self.stderr,
            "  -version\n    \tShow version and how this binary was installed\n"
        );
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

        // The `Linter` borrows this command's stdout for its lifetime, so error output keeps
        // flowing to the same stream the process was given.
        let mut linter = crate::linter::Linter::new(&mut *self.stdout, opts).map_err(|e| {
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
            linter.lint_files(&paths, None)
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
