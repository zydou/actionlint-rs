//! The linter that orchestrates parsing and rule checking of workflow files.
//!
//! Rust counterpart of the original `linter.go`. The [`Linter`] manages the entire linting
//! lifecycle: it locates the project, reads workflow files, parses them, runs the rules through a
//! [`Visitor`](crate::pass::Visitor), filters the resulting errors, and writes them to the output.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::config::{Config, IgnorePatterns};
use crate::error::{Error, ErrorFormatter};

/// Options controlling the behavior of the [`Linter`].
///
/// This mirrors the original `LinterOptions`. It is not `Clone`/`Debug` because it owns the
/// `-verbose`/`-debug` log writer (a trait object), matching the original's `io.Writer` field.
#[derive(Default)]
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
    /// Writer for `-verbose` / `-debug` diagnostic output. The command wires this to stderr. When
    /// `None` (or the level is `None`) no diagnostic output is produced. Counterpart of the
    /// original's `LogWriter`.
    pub log_writer: Option<Box<dyn std::io::Write + Send>>,
}

/// Lints GitHub Actions workflow files.
///
/// The lifetime `'a` borrows the writer that errors are printed to, so the caller keeps ownership
/// of the stream (mirroring the original `Linter` holding an `io.Writer`).
pub struct Linter<'a> {
    out: &'a mut dyn std::io::Write,
    /// Writer for `-verbose` / `-debug` diagnostic output. The command wires this to stderr.
    log_out: Option<Box<dyn std::io::Write + Send>>,
    /// Verbosity of diagnostic output. `None`-equivalent is [`LogLevel::None`].
    log_level: LogLevel,
    opts: LinterOptions,
    /// The name used for stdin input in error messages.
    stdin_name: String,
    /// Compiled `--ignore` patterns. Errors whose messages match any pattern are dropped.
    ignore_pats: IgnorePatterns,
    /// Config loaded from the `-config-file` option. Takes priority over the repository config.
    default_config: Option<Config>,
    /// Template formatter for the `--format` option. When `None`, errors are pretty-printed.
    err_fmt: Option<ErrorFormatter>,
}

/// Verbosity of `-verbose` / `-debug` diagnostic output. Counterpart of the original `LogLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// No diagnostic output.
    None,
    /// `-verbose`: high-level progress lines.
    Verbose,
    /// `-debug`: all output including per-rule and per-file detail.
    Debug,
}

impl<'a> Linter<'a> {
    /// Creates a new `Linter` writing errors to `out` and configured with `opts`.
    ///
    /// Returns an error when an `--ignore` pattern is not a valid regular expression, the
    /// `-config-file` cannot be read/parsed, or the `--format` template is invalid.
    pub fn new(out: &'a mut dyn std::io::Write, mut opts: LinterOptions) -> Result<Self, String> {
        // Decide whether to colorize using the original's exact behavior and force it onto the
        // `colored` crate. We must not let `colored` consult the environment itself: it honors
        // `CLICOLOR_FORCE`/`CLICOLOR` (which Go's `fatih/color` ignores) and orders the checks
        // differently. Emulating `fatih/color` + the `Linter` color option keeps the two builds
        // byte-identical:
        //
        // - `color.New` (run at package init for every color used in `error.go`) captures
        //   `NO_COLOR != ""` into a per-Color `noColor = true`. `isNoColorSet` checks that pointer
        //   BEFORE the package default, so once `NO_COLOR` is set no `-color` flag can re-enable
        //   color — it is a hard veto.
        // - `-no-color` (`Never`) and `-color` (`Always`) set the package-level `color.NoColor`,
        //   which only applies when the per-Color pointer is unset (i.e. `NO_COLOR` absent).
        // - With no flag (`Auto`) the package default `NO_COLOR != "" || TERM == "dumb" || stdout
        //   is not a TTY` applies; since `NO_COLOR` was already handled as a veto, only `TERM` and
        //   the TTY check remain.
        let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let colorize = if no_color_env {
            false
        } else {
            match opts.color {
                Some(c) => c,
                None => {
                    std::env::var("TERM").as_deref() != Ok("dumb")
                        && std::io::stdout().is_terminal()
                }
            }
        };
        colored::control::set_override(colorize);

        let stdin_name = if opts.stdin_file_name.is_empty() {
            "<stdin>".to_string()
        } else {
            opts.stdin_file_name.clone()
        };

        let mut pats = Vec::with_capacity(opts.ignore_patterns.len());
        for s in &opts.ignore_patterns {
            let r = regex::Regex::new(s)
                .map_err(|e| format!("invalid regular expression for ignore pattern {s:?}: {e}"))?;
            pats.push(r);
        }

        let default_config = if opts.config_file.is_empty() {
            None
        } else {
            Some(crate::config::read_config_file(Path::new(
                &opts.config_file,
            ))?)
        };

        let err_fmt = if opts.format.is_empty() {
            None
        } else {
            Some(ErrorFormatter::new(&opts.format)?)
        };

        let log_level = if opts.debug {
            LogLevel::Debug
        } else if opts.verbose {
            LogLevel::Verbose
        } else {
            LogLevel::None
        };
        let log_out = opts.log_writer.take();
        let log_level = if log_out.is_some() {
            log_level
        } else {
            LogLevel::None
        };

        Ok(Self {
            out,
            log_out,
            log_level,
            opts,
            stdin_name,
            ignore_pats: IgnorePatterns(pats),
            default_config,
            err_fmt,
        })
    }

    /// Writes a `-verbose` progress line (`verbose: ...`) to the log writer when the log level is
    /// [`LogLevel::Verbose`] or higher. Counterpart of the original `Linter.log`. No-op when
    /// neither `-verbose` nor `-debug` was given.
    fn log(&mut self, args: std::fmt::Arguments<'_>) {
        if self.log_level < LogLevel::Verbose {
            return;
        }
        if let Some(w) = self.log_out.as_mut() {
            let _ = writeln!(w, "verbose: {args}");
        }
    }

    /// Writes a `-debug` line (`[Linter] ...`) to the log writer when the log level is
    /// [`LogLevel::Debug`]. Counterpart of the original `Linter.debug`.
    fn debug(&mut self, args: std::fmt::Arguments<'_>) {
        if self.log_level < LogLevel::Debug {
            return;
        }
        if let Some(w) = self.log_out.as_mut() {
            let _ = writeln!(w, "[Linter] {args}");
        }
    }

    /// Returns the writer that per-rule debug output should be sent to, or `None` when the log
    /// level is below [`LogLevel::Debug`]. Counterpart of the original `Linter.debugWriter`.
    #[must_use]
    pub fn debug_writer(&mut self) -> Option<&mut dyn std::io::Write> {
        if self.log_level < LogLevel::Debug {
            return None;
        }
        self.log_out
            .as_mut()
            .map(|w| &mut **w as &mut dyn std::io::Write)
    }

    /// Creates a new `Linter` like [`Linter::new`], but with an explicit default config that takes
    /// priority over both the `-config-file` option and the repository config file. This mirrors the
    /// original tests' `linter.defaultConfig = &Config{}` override and lets callers force a specific
    /// config (e.g. an empty one) regardless of the files on disk.
    ///
    /// Returns an error under the same conditions as [`Linter::new`].
    pub fn new_with_config(
        out: &'a mut dyn std::io::Write,
        opts: LinterOptions,
        default_config: Option<Config>,
    ) -> Result<Self, String> {
        let mut linter = Self::new(out, opts)?;
        linter.default_config = default_config;
        Ok(linter)
    }

    /// Lints the workflow content read from stdin.
    pub fn lint_stdin(&mut self, content: &str) -> Result<Vec<Error>, String> {
        let name = self.stdin_name.clone();
        self.log(format_args!("Reading the input from stdin"));
        self.lint(&name, content.as_bytes(), None)
    }

    /// Lints all YAML workflow files under `.github/workflows` of the project containing `dir`.
    /// When `dir` is empty, the current working directory is used.
    pub fn lint_repository(&mut self, dir: &str) -> Result<Vec<Error>, String> {
        let start = if dir.is_empty() {
            std::env::current_dir().map_err(|e| e.to_string())?
        } else {
            PathBuf::from(dir)
        };
        self.log(format_args!(
            "Linting all workflow files in repository: {}",
            start.display()
        ));
        let project = crate::project::find_project(&start)?.ok_or_else(|| {
            format!(
                "no project was found in any parent directories of {}",
                start.display()
            )
        })?;
        self.log(format_args!(
            "Detected project: {}",
            project.root_dir().display()
        ));
        let workflows = project.workflows_dir();
        self.lint_dir(&workflows, Some(project))
    }

    /// Lints all YAML workflow files in the given directory, recursing into subdirectories.
    ///
    /// Mirrors the original `LintDir`, which walks the directory tree with `filepath.Walk` and
    /// collects every `.yml` / `.yaml` file, so nested workflow files are checked too.
    pub fn lint_dir(
        &mut self,
        dir: &Path,
        project: Option<crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        let mut files = Vec::new();
        collect_workflow_files(dir, &mut files)
            .map_err(|e| format!("could not read files in {}: {e}", dir.display()))?;
        files.sort();
        self.log(format_args!("Collected {} YAML files", files.len()));
        if files.is_empty() {
            return Err(format!("no YAML file was found in {}", dir.display()));
        }
        self.lint_files(&files, project)
    }

    /// Lints the given YAML workflow files, printing each file's errors as they are checked.
    ///
    /// Mirrors the original `LintFiles`: every file is read up front, and if any read fails the
    /// error is returned before anything is linted or printed (so stdout stays empty on failure).
    pub fn lint_files(
        &mut self,
        paths: &[PathBuf],
        project: Option<crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        let n = paths.len();
        self.log(format_args!("Linting {n} files"));

        // Phase 1: resolve each file's project and read its content. Any failure aborts before any
        // output is produced, matching the original's `errgroup.Wait()` after all reads.
        let mut contents: Vec<(PathBuf, String, Option<crate::project::Project>)> =
            Vec::with_capacity(paths.len());
        for p in paths {
            let project = match &project {
                Some(pr) => crate::project::find_project(p)?.or(Some(pr.clone())),
                None => crate::project::find_project(p)?,
            };
            let content =
                std::fs::read(p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
            // Use the path relative to the current directory when possible, matching the original's
            // `filepath.Rel(cwd, path)`. Error messages and `paths` config matching then use this
            // relative path, so cwd-relative glob patterns apply regardless of how the file was
            // passed (absolute path or discovered without arguments).
            let rel = relativize_to_cwd(p);
            contents.push((
                PathBuf::from(rel),
                String::from_utf8_lossy(&content).into_owned(),
                project,
            ));
        }

        // Phase 2: lint and print. All reads succeeded, so output flows normally.
        let mut all = Vec::new();
        // In `--format` mode all errors are rendered through a single template invocation, so the
        // template fields are accumulated across files and printed once at the end (matching the
        // original `LintFiles`).
        let mut fields = Vec::new();

        for (p, content, project) in &contents {
            let path_str = p.to_string_lossy().into_owned();
            let errs = self.check(&path_str, content.as_bytes(), project.as_ref())?;

            if self.err_fmt.is_some() {
                fields.extend(errs.iter().map(|e| e.template_fields(content.as_bytes())));
            } else {
                let src: &[u8] = if self.opts.oneline {
                    &[]
                } else {
                    content.as_bytes()
                };
                for e in &errs {
                    e.pretty_print(&mut *self.out, src);
                }
            }
            all.extend(errs);
        }

        self.log(format_args!("Found {} errors in {n} files", all.len()));

        if let Some(fmt) = &self.err_fmt {
            fmt.print(&mut *self.out, &fields)?;
        }

        Ok(all)
    }

    /// Lints the given workflow content. The `path` is used as the file path in error messages.
    /// Errors are printed to the output writer and also returned.
    ///
    /// When `project` is `None`, it is detected from the path.
    pub fn lint(
        &mut self,
        path: &str,
        content: &[u8],
        project: Option<crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        // The original only resolves a project when the path is a real, existing file: `Lint` guards
        // project detection with `path != "<stdin>"` AND `os.Stat(path)` succeeding. `LintStdin`
        // passes a nil project, so a workflow piped on stdin is never attributed to the surrounding
        // git project (which would otherwise enable project-relative rules such as `workflow-call`
        // resolving local reusable workflows). A custom `-stdin-filename` that does not name a real
        // file is likewise not attributed to a project, matching the original.
        let project = match project {
            Some(p) => Some(p),
            None if path != "<stdin>" && Path::new(path).exists() => {
                crate::project::find_project(Path::new(path))?
            }
            None => None,
        };

        let errs = self.check(path, content, project.as_ref())?;

        if let Some(fmt) = &self.err_fmt {
            let fields: Vec<_> = errs.iter().map(|e| e.template_fields(content)).collect();
            fmt.print(&mut *self.out, &fields)?;
        } else {
            let src = if self.opts.oneline { &[][..] } else { content };
            for e in &errs {
                e.pretty_print(&mut *self.out, src);
            }
        }

        Ok(errs)
    }

    /// The core of the linting pipeline: parse the content, run the rules, filter the resulting
    /// errors, and normalize them (populate the file path, sort, dedup). Does not print.
    ///
    /// This is the counterpart of the `check` method in the original `Linter`.
    fn check(
        &mut self,
        path: &str,
        content: &[u8],
        project: Option<&crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        // `-config-file` has higher priority than the repository config file.
        let config = match &self.default_config {
            Some(c) => Some(c.clone()),
            None => project.and_then(|p| p.config().cloned()),
        };
        self.log(format_args!("Linting {path}"));
        if let Some(p) = project {
            self.log(format_args!("Using project at {}", p.root_dir().display()));
        }
        match &config {
            Some(c) => self.debug(format_args!("Config: {c:?}")),
            None => self.debug(format_args!("No config was found")),
        }

        let (workflow, mut all) = crate::parse::parse(content);

        if let Some(w) = workflow {
            let errs = crate::linter::run_rules(
                &w,
                path,
                project,
                config.as_ref(),
                &self.opts,
                self.err_fmt.as_ref(),
            )?;
            all.extend(errs);
        }

        all = self.filter_errors(all, config.as_ref(), path);

        for err in &mut all {
            err.filepath = path.to_string();
        }

        all.sort_by(crate::error::compare_errors);
        all.dedup_by(|a, b| crate::error::equals_errors(a, b));
        Ok(all)
    }

    /// Drops errors matched by the `--ignore` option or the `ignore` key of a matching `paths`
    /// entry in the config file. Counterpart of `filterErrors`.
    fn filter_errors(
        &self,
        mut errs: Vec<Error>,
        config: Option<&Config>,
        path: &str,
    ) -> Vec<Error> {
        let path_cfgs = config.map(|c| c.path_configs(path)).unwrap_or_default();
        if self.ignore_pats.0.is_empty() && path_cfgs.is_empty() {
            return errs;
        }

        errs.retain(|e| {
            if self.ignore_pats.matches(&e.message) {
                return false;
            }
            !path_cfgs.iter().any(|pc| pc.ignore.matches(&e.message))
        });
        errs
    }
}

/// Recursively collects every `.yml` / `.yaml` file under `dir`, mirroring the original's
/// `filepath.Walk` in `LintDir`. Only regular files with a workflow extension are collected;
/// subdirectories are traversed so nested workflow files are checked too.
fn collect_workflow_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_workflow_files(&path, out)?;
        } else if file_type.is_file()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yml" | "yaml")
            )
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Computes `path` relative to the current working directory, matching the original's use of
/// `filepath.Rel(cwd, path)`. Already-relative paths are resolved against the current directory
/// first (as `filepath.Rel` receives an absolute `cwd` and an absolute or relative target). When
/// no relative form can be computed the original-form `path` is returned. This keeps error
/// messages and `paths` config matching aligned with the original, which reports and matches
/// against cwd-relative paths — including `../` upward traversal when the target lives outside
/// the current directory.
fn relativize_to_cwd(path: &Path) -> String {
    let display = path.to_string_lossy().into_owned();
    let Ok(cwd) = std::env::current_dir() else {
        return display;
    };
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    match filepath_rel(&cwd, &abs) {
        Some(rel) => rel,
        None => display,
    }
}

/// Faithful port of Go's `filepath.Rel(basepath, targpath)` for absolute Unix paths.
///
/// Returns the lexical path of `targpath` relative to `basepath`, using `..` to traverse up out of
/// `basepath` when the target is not a descendant. Returns `None` only when a component cannot be
/// made relative (not reachable for two absolute paths). Mirrors `filepath.Rel`, which cleans both
/// inputs (so trailing separators are ignored and `.` components are skipped) before stripping the
/// common prefix.
fn filepath_rel(basepath: &Path, targpath: &Path) -> Option<String> {
    let base: Vec<_> = basepath.components().collect();
    let targ: Vec<_> = targpath.components().collect();

    // Strip the common prefix.
    let mut common = 0;
    while common < base.len()
        && common < targ.len()
        && base[common].as_os_str() == targ[common].as_os_str()
    {
        common += 1;
    }

    // `..` for every remaining base component, then the remaining target components.
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for _ in common..base.len() {
        parts.push("..".into());
    }
    for c in &targ[common..] {
        parts.push(c.as_os_str().to_os_string());
    }

    if parts.is_empty() {
        return Some(".".to_string());
    }
    // `Path::push` on the first element absorbs a preceding `/`, keeping the result relative.
    let mut rel = PathBuf::new();
    for p in &parts {
        rel.push(p);
    }
    Some(rel.to_string_lossy().into_owned())
}

/// Runs all rules against the parsed workflow, returning the errors found by the rules (not
/// including syntax errors). Each rule is given the resolved config via [`Rule::set_config`].
///
/// This is the counterpart of the `check` method's rule-running portion in the original `Linter`.
pub fn run_rules(
    workflow: &crate::ast::Workflow,
    path: &str,
    project: Option<&crate::project::Project>,
    config: Option<&Config>,
    opts: &LinterOptions,
    err_fmt: Option<&ErrorFormatter>,
) -> Result<Vec<Error>, String> {
    let mut visitor = crate::pass::Visitor::new();
    let mut rules: Vec<Box<dyn crate::rule::Rule>> = Vec::new();

    // The `workflow-call` rule shares a reusable workflow cache with the linter so that a local
    // reusable workflow is parsed at most once across the whole lint run.
    let reusable_cache =
        std::sync::Arc::new(crate::reusable_workflow::LocalReusableWorkflowCache::new(
            project.cloned(),
            PathBuf::from(path),
            None,
        ));

    // The `action` rule shares a local action metadata cache so that a local action is read and
    // parsed at most once across the whole lint run.
    let actions_cache = std::sync::Arc::new(crate::action_metadata::LocalActionsCache::new(
        project.map(|p| p.root_dir().to_path_buf()),
    ));

    macro_rules! push {
        ($r:expr) => {
            rules.push(Box::new($r));
        };
    }

    push!(crate::rule_matrix::RuleMatrix::new());
    push!(crate::rule_credentials::RuleCredentials::new());
    push!(crate::rule_shell_name::RuleShellName::new());
    push!(crate::rule_runner_label::RuleRunnerLabel::new());
    push!(crate::rule_events::RuleEvents::new());
    push!(crate::rule_job_needs::RuleJobNeeds::new());
    push!(crate::rule_action::RuleAction::new(actions_cache.clone()));
    push!(crate::rule_env_var::RuleEnvVar::new());
    push!(crate::rule_id::RuleId::new());
    push!(crate::rule_glob::RuleGlob::new());
    push!(crate::rule_permissions::RulePermissions::new());
    push!(crate::rule_workflow_call::RuleWorkflowCall::new(
        PathBuf::from(path),
        reusable_cache.clone(),
    ));
    push!(crate::rule_expression::RuleExpression::new(
        actions_cache,
        reusable_cache,
    ));
    push!(crate::rule_deprecated_commands::RuleDeprecatedCommands::new());
    push!(crate::rule_if_cond::RuleIfCond::new());

    if !opts.shellcheck.is_empty()
        && let Ok(r) = crate::rule_shellcheck::RuleShellcheck::new(&opts.shellcheck)
    {
        rules.push(Box::new(r));
    }
    if !opts.pyflakes.is_empty()
        && let Ok(r) = crate::rule_pyflakes::RulePyflakes::new(&opts.pyflakes)
    {
        rules.push(Box::new(r));
    }

    // Populate each rule's config (self-hosted runner labels, config variables, ...).
    if let Some(cfg) = config {
        for r in &mut rules {
            r.set_config(cfg.clone());
        }
    }

    for r in &mut rules {
        visitor.add_pass_box(r.as_mut());
    }

    visitor.visit(workflow)?;

    let mut all = Vec::new();
    for r in &mut rules {
        all.append(&mut r.take_errors());
    }

    // Register every rule that ran with the error formatter so the `allKinds` format helper exposes
    // the full set of rule kinds (including the external `pyflakes` / `shellcheck` rules when they
    // were enabled). This mirrors the original's `RegisterRule` loop after `Visit`.
    if let Some(f) = err_fmt {
        for r in &rules {
            f.register_rule(r.as_ref());
        }
    }

    Ok(all)
}
