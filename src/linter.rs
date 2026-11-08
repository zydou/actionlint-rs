//! The linter that orchestrates parsing and rule checking of workflow files.
//!
//! Rust counterpart of the original `linter.go`. The [`Linter`] manages the entire linting
//! lifecycle: it locates the project, reads workflow files, parses them, runs the rules through a
//! [`Visitor`](crate::pass::Visitor), filters the resulting errors, and writes them to the output.

use std::path::{Path, PathBuf};

use crate::config::{Config, IgnorePatterns};
use crate::error::{Error, ErrorFormatter};

/// Options controlling the behavior of the [`Linter`].
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

/// Lints GitHub Actions workflow files.
///
/// The lifetime `'a` borrows the writer that errors are printed to, so the caller keeps ownership
/// of the stream (mirroring the original `Linter` holding an `io.Writer`).
pub struct Linter<'a> {
    out: &'a mut dyn std::io::Write,
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

impl<'a> Linter<'a> {
    /// Creates a new `Linter` writing errors to `out` and configured with `opts`.
    ///
    /// Returns an error when an `--ignore` pattern is not a valid regular expression, the
    /// `-config-file` cannot be read/parsed, or the `--format` template is invalid.
    pub fn new(out: &'a mut dyn std::io::Write, opts: LinterOptions) -> Result<Self, String> {
        // Honor the explicit color choice; leave the `colored` crate to auto-detect from the
        // environment (`NO_COLOR`, tty) when no option is given.
        match opts.color {
            Some(false) => colored::control::set_override(false),
            Some(true) => colored::control::set_override(true),
            None => colored::control::unset_override(),
        }

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

        Ok(Self {
            out,
            opts,
            stdin_name,
            ignore_pats: IgnorePatterns(pats),
            default_config,
            err_fmt,
        })
    }

    /// Lints the workflow content read from stdin.
    pub fn lint_stdin(&mut self, content: &str) -> Result<Vec<Error>, String> {
        let name = self.stdin_name.clone();
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
        let project = crate::project::find_project(&start)?.ok_or_else(|| {
            format!(
                "no project was found in any parent directories of {}",
                start.display()
            )
        })?;
        let workflows = project.workflows_dir();
        self.lint_dir(&workflows, Some(project))
    }

    /// Lints all YAML workflow files in the given directory.
    pub fn lint_dir(
        &mut self,
        dir: &Path,
        project: Option<crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("could not read files in {}: {e}", dir.display()))?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
            })
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(format!("no YAML file was found in {}", dir.display()));
        }
        self.lint_files(&files, project)
    }

    /// Lints the given YAML workflow files, printing each file's errors as they are checked.
    pub fn lint_files(
        &mut self,
        paths: &[PathBuf],
        project: Option<crate::project::Project>,
    ) -> Result<Vec<Error>, String> {
        let mut all = Vec::new();
        // In `--format` mode all errors are rendered through a single template invocation, so the
        // template fields are accumulated across files and printed once at the end (matching the
        // original `LintFiles`).
        let mut fields = Vec::new();

        for p in paths {
            let project = match &project {
                Some(pr) => crate::project::find_project(p)?.or(Some(pr.clone())),
                None => crate::project::find_project(p)?,
            };
            let content =
                std::fs::read(p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
            let path_str = p.to_string_lossy().into_owned();
            let errs = self.check(&path_str, &content, project.as_ref())?;

            if self.err_fmt.is_some() {
                fields.extend(errs.iter().map(|e| e.template_fields(&content)));
            } else {
                let src = if self.opts.oneline { &[][..] } else { &content[..] };
                for e in &errs {
                    e.pretty_print(&mut *self.out, src);
                }
            }
            all.extend(errs);
        }

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
        let project = match project {
            Some(p) => Some(p),
            None => crate::project::find_project(Path::new(path))?,
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

        let (workflow, mut all) = crate::parse::parse(content);

        if let Some(w) = workflow {
            let errs = crate::linter::run_rules(&w, path, project, config.as_ref(), &self.opts)?;
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
            !path_cfgs
                .iter()
                .any(|pc| pc.ignore.matches(&e.message))
        });
        errs
    }
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
    push!(crate::rule_action::RuleAction::new(actions_cache));
    push!(crate::rule_env_var::RuleEnvVar::new());
    push!(crate::rule_id::RuleId::new());
    push!(crate::rule_glob::RuleGlob::new());
    push!(crate::rule_permissions::RulePermissions::new());
    push!(crate::rule_workflow_call::RuleWorkflowCall::new(
        PathBuf::from(path),
        reusable_cache,
    ));
    push!(crate::rule_expression::RuleExpression::new());
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
    Ok(all)
}
