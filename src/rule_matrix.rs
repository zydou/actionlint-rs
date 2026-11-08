//! The `matrix` rule.
//!
//! Checks: Checks for matrix combinations in "matrix:". Rust counterpart of the original
//! `rule_matrix.go`. See
//! <https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions#jobsjob_idstrategymatrix>.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Job, Matrix, MatrixCombinations, MatrixRow, RawYAMLValue};
use crate::quotes::sorted_quotes;
use crate::rule::{Pass, Rule, RuleBase};

/// The `matrix` rule.
#[derive(Debug)]
pub struct RuleMatrix {
    base: RuleBase,
}

impl RuleMatrix {
    /// Creates a new `matrix` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("matrix", "Checks for matrix combinations in \"matrix:\""),
        }
    }

    fn check_duplicate_in_row(&mut self, row: &MatrixRow) {
        let mut seen: Vec<&RawYAMLValue> = Vec::with_capacity(row.values.len());
        for v in &row.values {
            let mut ok = true;
            for p in &seen {
                if p.equals(v) {
                    self.base.error_at(
                        v.pos(),
                        format!(
                            "duplicate value {} is found in matrix {:?}. the same value is at {}",
                            v,
                            row.name.value,
                            p.pos()
                        ),
                    );
                    ok = false;
                    break;
                }
            }
            if ok {
                seen.push(v);
            }
        }
    }

    fn check_exclude(&mut self, m: &Matrix) {
        let Some(exclude) = &m.exclude else {
            return;
        };
        if exclude.combinations.is_empty()
            || m.include
                .as_ref()
                .is_some_and(MatrixCombinations::contains_expression)
        {
            return;
        }

        if m.rows.is_empty() && m.include.as_ref().is_none_or(|i| i.combinations.is_empty()) {
            self.base.error_at(
                m.pos,
                "\"exclude\" section exists but no matrix variation exists",
            );
            return;
        }

        let mut rows: BTreeMap<&str, Vec<&RawYAMLValue>> = BTreeMap::new();
        let mut ignored: BTreeSet<&str> = BTreeSet::new();

        for (n, r) in &m.rows {
            if r.expression.is_some() {
                ignored.insert(n.as_str());
                continue;
            }
            rows.insert(n.as_str(), r.values.iter().collect());
        }

        if let Some(include) = &m.include {
            for c in &include.combinations {
                'include: for (n, a) in &c.assigns {
                    if ignored.contains(n.as_str()) {
                        continue;
                    }
                    let row = rows.entry(n.as_str()).or_default();
                    for v in row.iter() {
                        if v.equals(&a.value) {
                            continue 'include;
                        }
                    }
                    row.push(&a.value);
                }
            }
        }

        for c in &exclude.combinations {
            'exclude: for (k, a) in &c.assigns {
                if ignored.contains(k.as_str()) {
                    continue;
                }
                let Some(row) = rows.get(k.as_str()) else {
                    let ss: Vec<&str> = rows.keys().copied().collect();
                    self.base.error_at(
                        a.key.pos,
                        format!(
                            "{:?} in \"exclude\" section does not exist in matrix. available matrix configurations are {}",
                            k,
                            sorted_quotes(ss)
                        ),
                    );
                    continue;
                };

                for v in row {
                    if is_yaml_value_subset(v, &a.value) {
                        continue 'exclude;
                    }
                }

                let ss: Vec<std::string::String> =
                    row.iter().copied().map(RawYAMLValue::to_string).collect();
                self.base.error_at(
                    a.value.pos(),
                    format!(
                        "value {} in \"exclude\" does not match in matrix {:?} combinations. possible values are {}",
                        a.value,
                        k,
                        ss.join(", ") // Note: do not use quotesBuilder
                    ),
                );
            }
        }
    }
}

impl Default for RuleMatrix {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns whether `sub` is a subset (or equal) filter that matches value `v`. Counterpart of
/// `isYAMLValueSubset`.
fn is_yaml_value_subset(v: &RawYAMLValue, sub: &RawYAMLValue) -> bool {
    // When the filter side is dynamically constructed with some expression, it is not possible to
    // statically check if the filter matches the value. To avoid false positives, assume such
    // filter always matches the value. (#414)
    if let RawYAMLValue::String(s) = sub
        && crate::ast::contains_expression(&s.value)
    {
        return true;
    }

    match v {
        // `exclude` filter can match to objects in matrix as subset of them (#249).
        RawYAMLValue::Object(v) => {
            let RawYAMLValue::Object(sub) = sub else {
                return false;
            };
            for (n, s) in &sub.props {
                match v.props.get(n) {
                    None => return false,
                    Some(p) => {
                        if !is_yaml_value_subset(p, s) {
                            return false;
                        }
                    }
                }
            }
            true
        }
        RawYAMLValue::Array(v) => {
            let RawYAMLValue::Array(sub) = sub else {
                return false;
            };
            if v.elems.len() != sub.elems.len() {
                return false;
            }
            v.elems
                .iter()
                .zip(&sub.elems)
                .all(|(a, b)| is_yaml_value_subset(a, b))
        }
        // When some item is constructed with ${{ }} dynamically, give up checking combinations (#261)
        RawYAMLValue::String(v) => {
            if crate::ast::contains_expression(&v.value) {
                return true;
            }
            matches!(sub, RawYAMLValue::String(s) if s.value == v.value)
        }
    }
}

impl Pass for RuleMatrix {
    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        let Some(strategy) = &n.strategy else {
            return Ok(());
        };
        let Some(m) = &strategy.matrix else {
            return Ok(());
        };
        if m.expression.is_some() {
            return Ok(());
        }

        for row in m.rows.values() {
            self.check_duplicate_in_row(row);
        }

        // Note:
        // Any new value can be set in the section as new combination. It can add new value to
        // existing column also.
        //
        // matrix:
        //   os: [ubuntu-latest, macos-latest]
        //   include:
        //     - os: windows-latest
        //       sh: pwsh

        self.check_exclude(m);
        Ok(())
    }
}

impl Rule for RuleMatrix {
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
    use crate::ast::{Pos, RawYAMLString, Strategy, String};

    fn raw_str(s: &str) -> RawYAMLValue {
        RawYAMLValue::String(RawYAMLString {
            value: s.to_string(),
            pos: Pos::new(1, 1),
        })
    }

    fn row(name: &str, values: Vec<RawYAMLValue>) -> MatrixRow {
        MatrixRow {
            name: String::at(name, Pos::new(1, 1)),
            values,
            expression: None,
        }
    }

    fn job_with_matrix(m: Matrix) -> Job {
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
            strategy: Some(Strategy {
                matrix: Some(m),
                fail_fast: None,
                max_parallel: None,
                pos: Pos::new(2, 3),
            }),
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(1, 1),
        }
    }

    #[test]
    fn duplicate_value_in_row() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::from([(
                "os".to_string(),
                row("os", vec![raw_str("linux"), raw_str("linux")]),
            )]),
            include: None,
            exclude: None,
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("duplicate value"));
    }

    #[test]
    fn no_duplicate_ok() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::from([(
                "os".to_string(),
                row("os", vec![raw_str("linux"), raw_str("macos")]),
            )]),
            include: None,
            exclude: None,
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn exclude_no_matrix() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::new(),
            include: None,
            exclude: Some(crate::ast::MatrixCombinations {
                combinations: vec![crate::ast::MatrixCombination {
                    assigns: BTreeMap::new(),
                    expression: None,
                }],
                expression: None,
            }),
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("\"exclude\" section exists but no matrix variation exists")
        );
    }

    #[test]
    fn exclude_unknown_column() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::from([("os".to_string(), row("os", vec![raw_str("linux")]))]),
            include: None,
            exclude: Some(crate::ast::MatrixCombinations {
                combinations: vec![crate::ast::MatrixCombination {
                    assigns: BTreeMap::from([(
                        "unknown".to_string(),
                        crate::ast::MatrixAssign {
                            key: String::at("unknown", Pos::new(3, 5)),
                            value: raw_str("x"),
                        },
                    )]),
                    expression: None,
                }],
                expression: None,
            }),
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("does not exist in matrix. available matrix configurations are")
        );
    }

    #[test]
    fn exclude_value_no_match() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::from([("os".to_string(), row("os", vec![raw_str("linux")]))]),
            include: None,
            exclude: Some(crate::ast::MatrixCombinations {
                combinations: vec![crate::ast::MatrixCombination {
                    assigns: BTreeMap::from([(
                        "os".to_string(),
                        crate::ast::MatrixAssign {
                            key: String::at("os", Pos::new(3, 5)),
                            value: raw_str("windows"),
                        },
                    )]),
                    expression: None,
                }],
                expression: None,
            }),
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("does not match in matrix"));
    }

    #[test]
    fn exclude_value_match_ok() {
        let mut r = RuleMatrix::new();
        let m = Matrix {
            rows: BTreeMap::from([("os".to_string(), row("os", vec![raw_str("linux")]))]),
            include: None,
            exclude: Some(crate::ast::MatrixCombinations {
                combinations: vec![crate::ast::MatrixCombination {
                    assigns: BTreeMap::from([(
                        "os".to_string(),
                        crate::ast::MatrixAssign {
                            key: String::at("os", Pos::new(3, 5)),
                            value: raw_str("linux"),
                        },
                    )]),
                    expression: None,
                }],
                expression: None,
            }),
            expression: None,
            pos: Pos::new(1, 1),
        };
        r.visit_job_pre(&job_with_matrix(m)).unwrap();
        assert!(r.take_errors().is_empty());
    }
}
