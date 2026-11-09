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
    /// Writer for `-verbose` / `-debug` diagnostic output (the original's `opts.LogWriter`).
    /// Kept separate from `stderr` because the linter borrows `stdout` for results, while the log
    /// stream is independent.
    log_writer: Option<Box<dyn Write + Send>>,
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
            log_writer: None,
        }
    }

    /// Sets the writer that `-verbose` / `-debug` diagnostic output is sent to (the original's
    /// `opts.LogWriter`). When unset, no diagnostic output is produced.
    pub fn set_log_writer(&mut self, w: Box<dyn Write + Send>) {
        self.log_writer = Some(w);
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
        // Locate the project by walking up from the current directory, mirroring the original's
        // `GenerateDefaultConfig` (which resolves the project before writing). This requires a Git
        // repository with a `.github/workflows` directory, and writes the config to the project
        // root — so running from a subdirectory targets the root, and a non-project fails without
        // creating anything.
        let cwd = std::env::current_dir().map_err(|_| EXIT_STATUS_FAILURE)?;
        let project = crate::project::find_project(&cwd).map_err(|_| EXIT_STATUS_FAILURE)?;
        let Some(project) = project else {
            let _ = writeln!(
                self.stderr,
                "project is not found. check current project is initialized as Git repository and \".github/workflows\" directory exists"
            );
            return Err(EXIT_STATUS_FAILURE);
        };

        let dir = project.root_dir().join(".github");
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
            // `-verbose` / `-debug` diagnostics go to stderr, mirroring the original's
            // `opts.LogWriter = cmd.Stderr`. Consumed by the `Linter` on construction.
            log_writer: self.log_writer.take(),
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
    /// `-color` was given. Combined with `no_color` to compute `color`.
    color_set: bool,
    /// `-no-color` was given. Always overrides `color`, matching the original.
    no_color: bool,
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

/// Parses a boolean flag value with Go's `strconv.ParseBool` semantics.
///
/// Accepts `1`/`t`/`T`/`TRUE`/`true`/`True` and `0`/`f`/`F`/`FALSE`/`false`/`False`. Returns
/// `None` for any other value, which the caller reports as an invalid boolean flag (exit code 2).
fn parse_go_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

/// The result of attempting to parse a single flag from the argument list.
enum OneFlag {
    /// A flag was consumed; keep parsing.
    Seen,
    /// Parsing stops here. The remaining arguments (including the one that triggered the stop,
    /// except a consumed `--` terminator) become positional.
    Stop,
}

/// Parses one flag from the front of `it`, mirroring Go's `flag.FlagSet.parseOne`.
///
/// Returns [`OneFlag::Seen`] when a flag was recognized and consumed. Returns
/// [`OneFlag::Stop`] when a non-flag argument or `--` terminator is reached, in which case the
/// remaining arguments (including the stopping one, unless `consumed` is `true`) become positional.
/// Returns `Err` for an undefined flag, a missing flag value, or an invalid boolean value, matching
/// the Go `flag` package's error messages (which the original reports with exit code 2).
fn parse_one(
    it: &mut std::slice::Iter<'_, OsString>,
    cli: &mut Cli,
) -> Result<OneFlag, ParseError> {
    let Some(s) = it.as_slice().first() else {
        return Ok(OneFlag::Stop);
    };
    let s = s.to_string_lossy().into_owned();

    // A lone `-` or any argument not starting with `-` is not a flag: parsing stops and this
    // argument stays in the positional list (Go leaves it in `f.args`).
    if s.len() < 2 || !s.starts_with('-') {
        return Ok(OneFlag::Stop);
    }

    let rest = &s[1..];
    if let Some(after) = rest.strip_prefix('-') {
        // `--` (exactly two dashes) terminates flag parsing and is itself consumed.
        if after.is_empty() {
            it.next();
            return Ok(OneFlag::Stop);
        }
    }

    let mut name = rest.to_string();
    let mut inline_value: Option<String> = None;
    if let Some(eq) = name.find('=') {
        inline_value = Some(name[eq + 1..].to_string());
        name.truncate(eq);
    }
    if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
        return Err(ParseError::Invalid(format!("bad flag syntax: {s}")));
    }

    // Consume the flag token now that we know it is a flag.
    it.next();

    let is_bool = matches!(
        name.as_str(),
        "version" | "init-config" | "verbose" | "debug" | "oneline" | "no-color" | "color"
    );

    if name == "h" || name == "help" {
        return Err(ParseError::Help);
    }

    // For a boolean flag the inline value is optional and must be a valid bool; without one the
    // flag is set to true. For a string flag a missing value falls back to the next argument.
    let value = if is_bool {
        match inline_value {
            Some(v) => match parse_go_bool(&v) {
                Some(b) => b.to_string(),
                None => {
                    return Err(ParseError::Invalid(format!(
                        "invalid boolean value {v:?} for -{name}"
                    )));
                }
            },
            None => "true".to_string(),
        }
    } else if let Some(v) = inline_value {
        v
    } else {
        match it.next() {
            Some(v) => v.to_string_lossy().into_owned(),
            None => {
                return Err(ParseError::Invalid(format!(
                    "flag needs an argument: -{name}"
                )));
            }
        }
    };

    match name.as_str() {
        "version" => cli.version = value == "true",
        "init-config" => cli.init_config = value == "true",
        "verbose" => cli.verbose = value == "true",
        "debug" => cli.debug = value == "true",
        "oneline" => cli.oneline = value == "true",
        "no-color" => cli.no_color = value == "true",
        "color" => cli.color_set = value == "true",
        "ignore" => cli.ignore.push(value),
        "shellcheck" => cli.shellcheck = value,
        "pyflakes" => cli.pyflakes = value,
        "config-file" => cli.config_file = value,
        "format" => cli.format = value,
        "stdin-filename" => cli.stdin_filename = value,
        _ => {
            return Err(ParseError::Invalid(format!(
                "flag provided but not defined: -{name}"
            )));
        }
    }
    Ok(OneFlag::Seen)
}

/// Parses command line arguments into a [`Cli`] struct, replicating Go's `flag` package semantics.
///
/// Parsing stops at the first non-flag argument or at a `--` terminator; everything after that is
/// treated as a positional file argument (and is not interpreted as a flag). Boolean flags accept
/// an optional `=value` and string flags take the next argument when no `=value` is given, exactly
/// like the original. `-no-color` always overrides `-color` regardless of their relative order,
/// matching the original's color handling.
fn parse_args(args: &[OsString]) -> Result<Cli, ParseError> {
    let mut cli = Cli {
        args: Vec::new(),
        version: false,
        init_config: false,
        verbose: false,
        debug: false,
        oneline: false,
        color: None,
        color_set: false,
        no_color: false,
        shellcheck: "shellcheck".to_string(),
        pyflakes: "pyflakes".to_string(),
        ignore: Vec::new(),
        config_file: String::new(),
        format: String::new(),
        stdin_filename: "<stdin>".to_string(),
    };

    let mut it = args[1..].iter(); // Skip program name
    // Each `parse_one` consumes exactly one flag; stop at the first non-flag (or `--`), leaving the
    // iterator positioned at the remaining positional arguments.
    while let OneFlag::Seen = parse_one(&mut it, &mut cli)? {}
    cli.args
        .extend(it.map(|a| a.to_string_lossy().into_owned()));

    // `-no-color` wins over `-color` no matter which appears first, mirroring the original where
    // the `noColor` check runs after the `color` check.
    cli.color = if cli.no_color {
        Some(false)
    } else if cli.color_set {
        Some(true)
    } else {
        None
    };

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
    fn stops_at_first_positional() {
        // Go's flag package stops parsing at the first non-flag argument, so `-version` here is a
        // file name, not a flag.
        let args: Vec<OsString> = ["actionlint", "foo.yaml", "-version"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(
            cli.args,
            vec!["foo.yaml".to_string(), "-version".to_string()]
        );
        assert!(!cli.version);
    }

    #[test]
    fn dashdash_terminates_parsing() {
        // `--` is consumed and everything after it becomes positional.
        let args: Vec<OsString> = ["actionlint", "--", "-version"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.args, vec!["-version".to_string()]);
        assert!(!cli.version);
    }

    #[test]
    fn bool_flag_inline_value() {
        // Boolean flags accept an `=value` suffix, parsed with strconv.ParseBool semantics.
        let mk = |a: &str| parse_args(&[OsString::from("actionlint"), OsString::from(a)]).unwrap();
        assert!(!mk("-color=false").color_set);
        assert!(mk("-color=true").color_set);
        assert!(mk("-color=1").color_set);
        assert!(mk("-color=t").color_set);
        assert!(!mk("-oneline=0").oneline);
        for bad in ["-color=notabool", "-version=x", "-verbose=2"] {
            let args: Vec<OsString> = ["actionlint", bad].iter().map(OsString::from).collect();
            assert!(
                matches!(parse_args(&args), Err(ParseError::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn no_color_overrides_color() {
        // `-no-color` always wins over `-color`, regardless of the order they appear in.
        let mk = |a: &[&str]| {
            let mut v = vec![OsString::from("actionlint")];
            v.extend(a.iter().map(OsString::from));
            parse_args(&v).unwrap().color
        };
        assert_eq!(mk(&["-no-color", "-color"]), Some(false));
        assert_eq!(mk(&["-color", "-no-color"]), Some(false));
        assert_eq!(mk(&["-color"]), Some(true));
        assert_eq!(mk(&["-no-color"]), Some(false));
        assert_eq!(mk(&[]), None);
    }

    #[test]
    fn string_flag_takes_next_arg() {
        // A string flag consumes the next argument even if it looks like a flag, matching Go.
        let args: Vec<OsString> = ["actionlint", "-format", "-color"]
            .iter()
            .map(OsString::from)
            .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.format, "-color");
        assert!(!cli.color_set);
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
