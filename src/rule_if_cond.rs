//! The `if-cond` rule.
//!
//! Checks: Checks for if: conditions which are always true/false. Rust counterpart of the original
//! `rule_if_cond.go`.

use crate::ast::{Job, Pos, Step, String};
use crate::expr_lexer::ExprLexer;
use crate::expr_parser::ExprParser;
use crate::expr_sema::ExprSemanticsChecker;
use crate::rule::{Pass, Rule, RuleBase};

/// The `if-cond` rule.
#[derive(Debug)]
pub struct RuleIfCond {
    base: RuleBase,
}

impl RuleIfCond {
    /// Creates a new `if-cond` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "if-cond",
                "Checks for if: conditions which are always true/false",
            ),
        }
    }

    fn check_if_cond(&mut self, n: Option<&String>) {
        let Some(n) = n else {
            return;
        };
        match (n.value.find("${{"), n.value.find("}}")) {
            (Some(s), Some(e)) => self.check_placeholder(n, s, e),
            _ => self.check_expression(n.pos, &n.value),
        }
    }

    fn check_placeholder(&mut self, n: &String, start: usize, end: usize) {
        // Check number of ${{ }} for conditions like `${{ false }} || ${{ true }}` which are always
        // evaluated to true
        if start > 0 || end + 2 < n.value.len() || n.value.matches("${{").count() > 1 {
            self.base.errorf_at(
                n.pos,
                format_args!(
                    "if: condition {:?} is always evaluated to true because extra characters are around ${{{{ }}}}",
                    n.value
                ),
            );
            return;
        }
        self.check_expression(n.pos, &n.value[start + 3..end]);
    }

    fn check_expression(&mut self, pos: Pos, input: &str) {
        let i = input.trim();
        // `}}` is necessary since the lexer lexes it as the end marker. Each `}}` in the format
        // string is an escaped literal `}`.
        let l = ExprLexer::new(&format!("{i}}}}}"));
        if let Ok(e) = ExprParser::new().parse(l) {
            // Only constant values are reported. Note that the availability of contexts is not
            // checked here because `if:` conditions are evaluated before the job runs.
            if ExprSemanticsChecker::is_constant(&e) {
                self.base.errorf_at(
                    pos,
                    format_args!("constant expression {i:?} in condition. remove the if: section"),
                );
            }
        }
    }
}

impl Default for RuleIfCond {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleIfCond {
    fn visit_step(&mut self, n: &Step) -> Result<(), std::string::String> {
        self.check_if_cond(n.if_condition.as_ref());
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), std::string::String> {
        self.check_if_cond(n.if_condition.as_ref());
        if let Some(s) = &n.snapshot {
            self.check_if_cond(s.if_condition.as_ref());
        }
        Ok(())
    }
}

impl Rule for RuleIfCond {
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
    use crate::ast::{Exec, ExecRun};

    struct Table {
        name: &'static str,
        input: &'static str,
        want: &'static str,
    }

    fn run(t: &Table) {
        let mut r = RuleIfCond::new();
        let n = String::at(t.input, Pos::new(1, 1));
        r.check_if_cond(Some(&n));
        let errs = r.take_errors();
        if t.want.is_empty() {
            assert!(
                errs.is_empty(),
                "{}: no error expected but got {errs:?}",
                t.name
            );
        } else {
            assert_eq!(errs.len(), 1, "{}: exactly one error expected", t.name);
            assert_eq!(errs[0].message, t.want, "{}", t.name);
        }
    }

    fn step_with_if(cond: Option<String>) -> Step {
        Step {
            id: None,
            if_condition: cond,
            name: None,
            exec: Exec::Run(ExecRun {
                run: "echo".into(),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(1, 1),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(1, 1),
        }
    }

    #[test]
    fn name_and_description() {
        let r = RuleIfCond::new();
        assert_eq!(r.name(), "if-cond");
        assert_eq!(
            r.description(),
            "Checks for if: conditions which are always true/false"
        );
    }

    #[test]
    fn constant_false() {
        let t = Table {
            name: "constant false",
            input: "${{ false }}",
            want: "constant expression \"false\" in condition. remove the if: section",
        };
        run(&t);
    }

    #[test]
    fn constant_true() {
        let t = Table {
            name: "constant true",
            input: "${{ true }}",
            want: "constant expression \"true\" in condition. remove the if: section",
        };
        run(&t);
    }

    #[test]
    fn constant_bool_literal() {
        // `!true` and `!null` are constant because their operand is constant.
        for input in ["${{ !true }}", "${{ !null }}"] {
            let trimmed = input
                .trim_start_matches("${{ ")
                .trim_end_matches(" }}")
                .trim();
            let mut r = RuleIfCond::new();
            let n = String::at(input, Pos::new(1, 1));
            r.check_if_cond(Some(&n));
            let errs = r.take_errors();
            assert_eq!(errs.len(), 1, "{input}");
            assert_eq!(
                errs[0].message,
                format!("constant expression {trimmed:?} in condition. remove the if: section"),
                "{input}"
            );
        }
    }

    #[test]
    fn always_true_extra_chars_around_placeholder() {
        // Extra characters around ${{ }} make the condition always true.
        for input in [
            "${{ false }} || ${{ true }}",
            " ${{ false }}",
            "${{ false }} ",
        ] {
            let mut r = RuleIfCond::new();
            let n = String::at(input, Pos::new(1, 1));
            r.check_if_cond(Some(&n));
            let errs = r.take_errors();
            assert_eq!(errs.len(), 1, "{input}");
            assert_eq!(
                errs[0].message,
                format!(
                    "if: condition {input:?} is always evaluated to true because extra characters are around ${{{{ }}}}"
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn real_conditions_no_error() {
        for input in [
            "github.event_name == 'push'",
            "success()",
            // Note: `always()`, `success()`, `cancelled()` and `failure()` are NOT constant in the
            // original (their `IsConstFunc` is unset). `${{ always() }}` therefore produces no error.
            "${{ always() }}",
            "${{ github.ref }}",
            "false || github.ref",
        ] {
            let t = Table {
                name: "real condition",
                input,
                want: "",
            };
            run(&t);
        }
    }

    #[test]
    fn visit_step_and_job() {
        let mut r = RuleIfCond::new();
        let mut step = step_with_if(Some(String::at("${{ false }}", Pos::new(2, 8))));
        r.visit_step(&step).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 2);
        assert_eq!(errs[0].column, 8);

        step.if_condition = None;
        let job = Job {
            id: String::at("test", Pos::new(1, 1)),
            if_condition: Some(String::at("${{ false }}", Pos::new(3, 8))),
            snapshot: None,
            ..create_min_job()
        };
        r.visit_job_pre(&job).unwrap();
        assert_eq!(r.take_errors().len(), 1);
    }

    #[test]
    fn job_snapshot_if_checked() {
        let mut r = RuleIfCond::new();
        let job = Job {
            id: String::at("test", Pos::new(1, 1)),
            if_condition: None,
            snapshot: Some(crate::ast::Snapshot {
                image_name: String::at("img", Pos::new(1, 1)),
                version: None,
                if_condition: Some(String::at("${{ true }}", Pos::new(4, 8))),
            }),
            ..create_min_job()
        };
        r.visit_job_pre(&job).unwrap();
        assert_eq!(r.take_errors().len(), 1);
    }

    fn create_min_job() -> Job {
        Job {
            id: String::at("test", Pos::new(1, 1)),
            name: None,
            needs: vec![],
            runs_on: None,
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
}
