//! The linter that orchestrates parsing and rule checking of workflow files.
//!
//! Rust counterpart of the original `linter.go`. The [`Linter`] manages the entire linting
//! lifecycle: it locates the project, reads workflow files, parses them, runs the rules through a
//! [`Visitor`](crate::pass::Visitor), and collects the resulting errors.

use std::path::{Path, PathBuf};

use crate::error::Error;

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
#[derive(Debug)]
pub struct Linter {
    #[allow(dead_code)]
    opts: LinterOptions,
    /// The name used for stdin input in error messages.
    stdin_name: String,
}

impl Linter {
    /// Creates a new `Linter` writing errors to `out` and configured with `opts`.
    pub fn new(out: Box<dyn std::io::Write>, opts: LinterOptions) -> Result<Self, String> {
        let stdin_name = if opts.stdin_file_name.is_empty() {
            "<stdin>".to_string()
        } else {
            opts.stdin_file_name.clone()
        };
        let _ = out;
        Ok(Self { opts, stdin_name })
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

    /// Lints all YAML workflow files in the given directory recursively.
    pub fn lint_dir(
        &mut self,
        dir: &Path,
        _project: Option<crate::project::Project>,
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
        self.lint_files(&files)
    }

    /// Lints the given YAML workflow files.
    pub fn lint_files(&mut self, paths: &[PathBuf]) -> Result<Vec<Error>, String> {
        let mut all = Vec::new();
        for p in paths {
            let project = crate::project::find_project(p)?;
            let content =
                std::fs::read(p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
            let path_str = p.to_string_lossy().into_owned();
            let mut errs = self.lint(&path_str, &content, project)?;
            all.append(&mut errs);
        }
        all.sort_by(crate::error::compare_errors);
        all.dedup_by(|a, b| crate::error::equals_errors(a, b));
        Ok(all)
    }

    /// Lints the given workflow content. The `path` is used as the file path in error messages.
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

        let (workflow, mut all) = crate::parse::parse(content);

        if let Some(w) = workflow {
            let errs = crate::linter::run_rules(&w, path, project.as_ref(), &self.opts)?;
            all.extend(errs);
        }

        for err in &mut all {
            err.filepath = path.to_string();
        }

        all.sort_by(crate::error::compare_errors);
        all.dedup_by(|a, b| crate::error::equals_errors(a, b));
        Ok(all)
    }
}

/// Runs all rules against the parsed workflow, returning the errors found by the rules (not
/// including syntax errors).
///
/// This is the counterpart of the `check` method in the original `Linter`.
pub fn run_rules(
    workflow: &crate::ast::Workflow,
    _path: &str,
    _project: Option<&crate::project::Project>,
    opts: &LinterOptions,
) -> Result<Vec<Error>, String> {
    let mut visitor = crate::pass::Visitor::new();
    let mut rules: Vec<Box<dyn crate::rule::Rule>> = Vec::new();

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
    push!(crate::rule_action::RuleAction::new());
    push!(crate::rule_env_var::RuleEnvVar::new());
    push!(crate::rule_id::RuleId::new());
    push!(crate::rule_glob::RuleGlob::new());
    push!(crate::rule_permissions::RulePermissions::new());
    push!(crate::rule_workflow_call::RuleWorkflowCall::new());
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
