//! The `runner-label` rule.
//!
//! Checks: Checks for GitHub-hosted and preset self-hosted runner labels in "runs-on:". Rust
//! counterpart of the original `rule_runner_label.go`. See:
//! - <https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>
//! - <https://docs.github.com/en/actions/hosting-your-own-runners/using-self-hosted-runners-in-a-workflow>

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::ast::{Matrix, RawYAMLValue, Runner, String};
use crate::glob::path_match;
use crate::quotes::quotes_all;
use crate::rule::{Pass, Rule, RuleBase};

// Bit flags describing which GitHub-hosted runner OS images a label is compatible with.
const COMPAT_INVALID: u32 = 0;
const COMPAT_UBUNTU_2204: u32 = 1 << 1;
const COMPAT_UBUNTU_2404: u32 = 1 << 2;
const COMPAT_MACOS_140: u32 = 1 << 3;
const COMPAT_MACOS_140L: u32 = 1 << 4;
const COMPAT_MACOS_140XL: u32 = 1 << 5;
const COMPAT_MACOS_150: u32 = 1 << 6;
const COMPAT_MACOS_150INTEL: u32 = 1 << 7;
const COMPAT_MACOS_150L: u32 = 1 << 8;
const COMPAT_MACOS_150XL: u32 = 1 << 9;
const COMPAT_MACOS_260: u32 = 1 << 10;
const COMPAT_MACOS_260INTEL: u32 = 1 << 11;
const COMPAT_MACOS_260L: u32 = 1 << 12;
const COMPAT_MACOS_260XL: u32 = 1 << 13;
const COMPAT_WINDOWS_2022: u32 = 1 << 14;
const COMPAT_WINDOWS_2025: u32 = 1 << 15;
const COMPAT_WINDOWS_2025_VS2026: u32 = 1 << 16;
const COMPAT_WINDOWS_11_ARM: u32 = 1 << 17;

/// GitHub-hosted runner labels.
/// <https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>
const ALL_GITHUB_HOSTED_RUNNER_LABELS: &[&str] = &[
    "windows-latest",
    "windows-latest-8-cores",
    "windows-2025",
    "windows-2025-vs2026",
    "windows-2022",
    "windows-11-arm",
    "ubuntu-slim",
    "ubuntu-latest",
    "ubuntu-latest-4-cores",
    "ubuntu-latest-8-cores",
    "ubuntu-latest-16-cores",
    "ubuntu-24.04",
    "ubuntu-24.04-arm",
    "ubuntu-22.04",
    "ubuntu-22.04-arm",
    "macos-latest",
    "macos-latest-xlarge",
    "macos-latest-large",
    "macos-26-intel",
    "macos-26-xlarge",
    "macos-26-large",
    "macos-26",
    "macos-15-intel",
    "macos-15-xlarge",
    "macos-15-large",
    "macos-15",
    "macos-14-xlarge",
    "macos-14-large",
    "macos-14",
];

/// Preset self-hosted runner OS labels.
const SELF_HOSTED_RUNNER_PRESET_OS_LABELS: &[&str] = &["linux", "macos", "windows"];

/// Preset self-hosted runner labels other than the OS labels.
const SELF_HOSTED_RUNNER_PRESET_OTHER_LABELS: &[&str] = &["self-hosted", "x64", "arm", "arm64"];

/// The map from a label name (lower cased) to the set of runner OS images it is compatible with.
fn default_runner_os_compats() -> &'static BTreeMap<&'static str, u32> {
    static MAP: OnceLock<BTreeMap<&'static str, u32>> = OnceLock::new();
    MAP.get_or_init(|| {
        BTreeMap::from([
            ("ubuntu-slim", COMPAT_UBUNTU_2404),
            ("ubuntu-latest", COMPAT_UBUNTU_2404),
            ("ubuntu-latest-4-cores", COMPAT_UBUNTU_2404),
            ("ubuntu-latest-8-cores", COMPAT_UBUNTU_2404),
            ("ubuntu-latest-16-cores", COMPAT_UBUNTU_2404),
            ("ubuntu-24.04", COMPAT_UBUNTU_2404),
            ("ubuntu-24.04-arm", COMPAT_UBUNTU_2404),
            ("ubuntu-22.04", COMPAT_UBUNTU_2204),
            ("ubuntu-22.04-arm", COMPAT_UBUNTU_2204),
            ("macos-latest-xlarge", COMPAT_MACOS_150XL),
            ("macos-latest-large", COMPAT_MACOS_150L),
            ("macos-latest", COMPAT_MACOS_150),
            ("macos-26-intel", COMPAT_MACOS_260INTEL),
            ("macos-26-xlarge", COMPAT_MACOS_260XL),
            ("macos-26-large", COMPAT_MACOS_260L),
            ("macos-26", COMPAT_MACOS_260),
            ("macos-15-intel", COMPAT_MACOS_150INTEL),
            ("macos-15-xlarge", COMPAT_MACOS_150XL),
            ("macos-15-large", COMPAT_MACOS_150L),
            ("macos-15", COMPAT_MACOS_150),
            ("macos-14-xlarge", COMPAT_MACOS_140XL),
            ("macos-14-large", COMPAT_MACOS_140L),
            ("macos-14", COMPAT_MACOS_140),
            ("windows-latest", COMPAT_WINDOWS_2022),
            ("windows-latest-8-cores", COMPAT_WINDOWS_2022),
            ("windows-2025", COMPAT_WINDOWS_2025),
            ("windows-2025-vs2026", COMPAT_WINDOWS_2025_VS2026),
            ("windows-2022", COMPAT_WINDOWS_2022),
            ("windows-11-arm", COMPAT_WINDOWS_11_ARM),
            // Note: "linux" does not always indicate Ubuntu. It might be Fedora or Arch or ...
            ("linux", COMPAT_UBUNTU_2404 | COMPAT_UBUNTU_2204),
            (
                "macos",
                COMPAT_MACOS_260
                    | COMPAT_MACOS_260INTEL
                    | COMPAT_MACOS_260L
                    | COMPAT_MACOS_260XL
                    | COMPAT_MACOS_150
                    | COMPAT_MACOS_150INTEL
                    | COMPAT_MACOS_150L
                    | COMPAT_MACOS_150XL
                    | COMPAT_MACOS_140
                    | COMPAT_MACOS_140L
                    | COMPAT_MACOS_140XL,
            ),
            (
                "windows",
                COMPAT_WINDOWS_2025_VS2026
                    | COMPAT_WINDOWS_2025
                    | COMPAT_WINDOWS_2022
                    | COMPAT_WINDOWS_11_ARM,
            ),
        ])
    })
}

/// The `runner-label` rule.
#[derive(Debug)]
pub struct RuleRunnerLabel {
    base: RuleBase,
    // Note: Using only one compatibility integer is enough to check compatibility. But we remember
    // all past compatibility values here for better error message. If accumulating all
    // compatibility values into one integer, we can no longer know what labels are conflicting.
    compats: Option<BTreeMap<u32, String>>,
}

impl RuleRunnerLabel {
    /// Creates a new `runner-label` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "runner-label",
                "Checks for GitHub-hosted and preset self-hosted runner labels in \"runs-on:\"",
            ),
            compats: None,
        }
    }

    fn get_known_labels(&self) -> &[std::string::String] {
        match self.base.config() {
            Some(c) => &c.self_hosted_runner_labels,
            None => &[],
        }
    }

    fn visit_job_pre_impl(&mut self, runner: Option<&Runner>, matrix: Option<&Matrix>) {
        let Some(runner) = runner else {
            return;
        };

        if runner.labels.len() == 1 {
            self.check_label(&runner.labels[0], matrix);
            return;
        }

        self.compats = Some(BTreeMap::new());
        if let Some(expr) = &runner.labels_expr {
            self.check_label_and_conflict(expr, matrix);
        } else {
            for label in &runner.labels {
                self.check_label_and_conflict(label, matrix);
            }
        }
        self.compats = None; // reset
    }

    fn check_label_and_conflict(&mut self, l: &String, m: Option<&Matrix>) {
        if l.contains_expression() {
            let ss = Self::try_to_get_labels_in_matrix(l, m);
            let mut cs = Vec::with_capacity(ss.len());
            for s in &ss {
                let comp = self.verify_runner_label(s);
                cs.push(comp);
            }
            self.check_combi_compat(cs, &ss);
            return;
        }

        let comp = self.verify_runner_label(l);
        self.check_compat(comp, l);
    }

    fn check_label(&mut self, l: &String, m: Option<&Matrix>) {
        if l.contains_expression() {
            let ss = Self::try_to_get_labels_in_matrix(l, m);
            for s in &ss {
                self.verify_runner_label(s);
            }
            return;
        }

        self.verify_runner_label(l);
    }

    fn verify_runner_label(&mut self, label: &String) -> u32 {
        let l = label.value.as_str();
        if let Some(c) = default_runner_os_compats().get(l.to_lowercase().as_str()) {
            return *c;
        }

        for p in SELF_HOSTED_RUNNER_PRESET_OTHER_LABELS {
            if l.eq_ignore_ascii_case(p) {
                return COMPAT_INVALID;
            }
        }

        let known = self.get_known_labels().to_vec();
        for k in &known {
            match path_match(k, l) {
                Err(e) => {
                    self.base.error_at(
                        label.pos,
                        format!(
                            "label pattern {k:?} is an invalid glob. kindly check list of labels in actionlint.yaml config file: {e}"
                        ),
                    );
                    return COMPAT_INVALID;
                }
                Ok(true) => return COMPAT_INVALID,
                Ok(false) => {}
            }
        }

        self.base.error_at(
            label.pos,
            format!(
                "label {:?} is unknown. available labels are {}. if it is a custom label for self-hosted runner, set list of labels in actionlint.yaml config file",
                label.value,
                quotes_all(&[
                    ALL_GITHUB_HOSTED_RUNNER_LABELS,
                    SELF_HOSTED_RUNNER_PRESET_OTHER_LABELS,
                    SELF_HOSTED_RUNNER_PRESET_OS_LABELS,
                    &known.iter().map(std::string::String::as_str).collect::<Vec<_>>(),
                ]),
            ),
        );

        COMPAT_INVALID
    }

    fn try_to_get_labels_in_matrix(label: &String, m: Option<&Matrix>) -> Vec<String> {
        let Some(m) = m else {
            return Vec::new();
        };

        // Only when the form of "${{...}}", evaluate the expression
        if !label.is_expression_assigned() {
            return Vec::new();
        }

        let l = label.value.trim();
        let Ok((expr, _)) = crate::expr_parser::parse_expression(&l[3..]) else {
            return Vec::new();
        };

        let crate::expr_ast::ExprNodeKind::ObjectDeref(deref) = expr.as_ref() else {
            return Vec::new();
        };
        let Some(recv) = &deref.receiver else {
            return Vec::new();
        };
        let crate::expr_ast::ExprNodeKind::Variable(v) = recv.as_ref() else {
            return Vec::new();
        };
        if v.name != "matrix" {
            return Vec::new();
        }

        let prop = &deref.property;
        let mut labels: Vec<String> = Vec::new();

        if let Some(row) = m.rows.get(prop) {
            for val in &row.values {
                if let RawYAMLValue::String(s) = val
                    && !crate::ast::contains_expression(&s.value)
                {
                    labels.push(String::at(s.value.as_str(), s.pos));
                }
            }
        }

        if let Some(include) = &m.include {
            for combi in &include.combinations {
                if let Some(assign) = combi.assigns.get(prop)
                    && let RawYAMLValue::String(s) = &assign.value
                    && !crate::ast::contains_expression(&s.value)
                {
                    labels.push(String::at(s.value.as_str(), s.pos));
                }
            }
        }

        labels
    }

    fn check_conflict(&mut self, comp: u32, label: &String) -> bool {
        let Some(compats) = &self.compats else {
            return true;
        };
        for (c, l) in compats {
            if c & comp == 0 {
                self.base.error_at(
                    label.pos,
                    format!(
                        "label {:?} conflicts with label {:?} defined at {}. note: to run your job on each workers, use matrix",
                        label.value, l.value, l.pos
                    ),
                );
                return false;
            }
        }
        true
    }

    fn check_compat(&mut self, comp: u32, label: &String) {
        if comp == COMPAT_INVALID || !self.check_conflict(comp, label) {
            return;
        }
        if let Some(compats) = &mut self.compats {
            compats.entry(comp).or_insert_with(|| label.clone());
        }
    }

    fn check_combi_compat(&mut self, mut comps: Vec<u32>, labels: &[String]) {
        for i in 0..comps.len() {
            if comps[i] != COMPAT_INVALID && !self.check_conflict(comps[i], &labels[i]) {
                // Overwrite the compatibility value with COMPAT_INVALID at conflicted label not to
                // register the label to `self.compats`.
                comps[i] = COMPAT_INVALID;
            }
        }
        for i in 0..comps.len() {
            if comps[i] != COMPAT_INVALID
                && let Some(compats) = &mut self.compats
            {
                compats.entry(comps[i]).or_insert_with(|| labels[i].clone());
            }
        }
    }
}

impl Default for RuleRunnerLabel {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleRunnerLabel {
    fn visit_job_pre(&mut self, n: &crate::ast::Job) -> Result<(), std::string::String> {
        let matrix = n.strategy.as_ref().and_then(|s| s.matrix.as_ref());
        self.visit_job_pre_impl(n.runs_on.as_ref(), matrix);
        Ok(())
    }
}

impl Rule for RuleRunnerLabel {
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
    use crate::ast::{Job, Pos};

    fn job_with_runner(labels: Vec<String>, labels_expr: Option<String>) -> Job {
        Job {
            id: String::at("test", Pos::new(1, 1)),
            name: None,
            needs: vec![],
            runs_on: Some(Runner {
                labels,
                labels_expr,
                group: None,
            }),
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(1, 1),
        }
    }

    fn label(s: &str) -> String {
        String::at(s, Pos::new(4, 12))
    }

    #[test]
    fn github_hosted_label_ok() {
        let mut r = RuleRunnerLabel::new();
        let job = job_with_runner(vec![label("ubuntu-latest")], None);
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn unknown_label() {
        let mut r = RuleRunnerLabel::new();
        let job = job_with_runner(vec![label("foobar")], None);
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("label \"foobar\" is unknown"));
    }

    #[test]
    fn preset_self_hosted_labels_ok() {
        let mut r = RuleRunnerLabel::new();
        let job = job_with_runner(
            vec![label("self-hosted"), label("linux"), label("x64")],
            None,
        );
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn conflicting_labels() {
        let mut r = RuleRunnerLabel::new();
        let job = job_with_runner(vec![label("ubuntu-latest"), label("macos-latest")], None);
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("conflicts with label"));
    }

    #[test]
    fn config_known_label_pattern() {
        let mut r = RuleRunnerLabel::new();
        let cfg = crate::config::Config {
            self_hosted_runner_labels: vec!["gpu-*".to_string()],
            ..crate::config::Config::default()
        };
        r.set_config(cfg);
        let job = job_with_runner(vec![label("gpu-big")], None);
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn invalid_glob_in_config() {
        let mut r = RuleRunnerLabel::new();
        let cfg = crate::config::Config {
            self_hosted_runner_labels: vec!["[invalid".to_string()],
            ..crate::config::Config::default()
        };
        r.set_config(cfg);
        let job = job_with_runner(vec![label("foobar")], None);
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("is an invalid glob"));
    }

    #[test]
    fn multiple_same_labels_no_conflict() {
        let mut r = RuleRunnerLabel::new();
        let job = job_with_runner(vec![label("ubuntu-latest"), label("ubuntu-latest")], None);
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
