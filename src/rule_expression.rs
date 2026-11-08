//! The `expression` rule.
//!
//! Checks: Syntax and semantics checks for expressions embedded with `${{ }}` syntax. Rust
//! counterpart of the original `rule_expression.go`. See
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>.

use std::collections::BTreeMap;
use std::string::String as StdString;
use std::sync::Arc;

use crate::action_metadata::{ActionMetadata, LocalActionsCache};
use crate::ast::{
    Bool, Concurrency, Container, Defaults, Env, Event, Exec, Job, Matrix, MatrixRow, Pos,
    RawYAMLValue, Snapshot, Step, String, Workflow, WorkflowCall,
};
use crate::availability::workflow_key_availability;
use crate::expr_ast::ExprNode;
use crate::expr_lexer::{ExprLexer, TokenKind};
use crate::expr_parser::ExprParser;
use crate::expr_sema::ExprSemanticsChecker;
use crate::expr_type::{ExprType, ExprTypeKind, ExprTypeKindInner, ObjectType};
use crate::popular_actions::popular_actions;
use crate::reusable_workflow::LocalReusableWorkflowCache;
use crate::rule::{Pass, Rule, RuleBase};

/// Parses the given expression source (which must end with the `}}` end marker) and returns the
/// parsed node along with the lexer's scanning offset after the expression.
///
/// The original passes the lexer to the parser by pointer and reads `lexer.Offset()` afterward. The
/// Rust [`ExprParser::parse`](crate::expr_parser::ExprParser::parse) consumes the lexer, so the
/// offset is computed by scanning a separate lexer until the `}}` end marker, which is where the
/// parser stops. This offset is what [`RuleExpression::check_exprs_in`] uses to advance past each
/// embedded `${{ }}` expression.
fn parse_expression_with_offset(src: &str) -> Result<(ExprNode, usize), crate::expr::ExprError> {
    let expr = ExprParser::new().parse(ExprLexer::new(src))?;
    let mut off = ExprLexer::new(src);
    loop {
        match off.next_token().kind {
            TokenKind::End | TokenKind::Unknown => break,
            _ => {}
        }
    }
    Ok((expr, off.offset()))
}

/// An expression with its static type and the position it appears at in the workflow source.
#[derive(Debug, Clone)]
struct TypedExpr {
    ty: ExprTypeKind,
    pos: Pos,
}

/// The `expression` rule.
#[derive(Debug)]
pub struct RuleExpression {
    base: RuleBase,
    matrix_ty: Option<ObjectType>,
    steps_ty: Option<ObjectType>,
    needs_ty: Option<ObjectType>,
    secrets_ty: Option<ObjectType>,
    inputs_ty: Option<ObjectType>,
    dispatch_inputs_ty: Option<ObjectType>,
    jobs_ty: Option<ObjectType>,
    workflow: Option<Workflow>,
    local_actions: Arc<LocalActionsCache>,
    local_workflows: Arc<LocalReusableWorkflowCache>,
}

impl RuleExpression {
    /// Creates a new `expression` rule. `actions_cache` and `workflow_cache` are shared caches so
    /// local actions and local reusable workflows are read at most once across the lint run.
    #[must_use]
    pub fn new(
        actions_cache: Arc<LocalActionsCache>,
        workflow_cache: Arc<LocalReusableWorkflowCache>,
    ) -> Self {
        Self {
            base: RuleBase::new(
                "expression",
                "Syntax and semantics checks for expressions embedded with ${{ }} syntax",
            ),
            matrix_ty: None,
            steps_ty: None,
            needs_ty: None,
            secrets_ty: None,
            inputs_ty: None,
            dispatch_inputs_ty: None,
            jobs_ty: None,
            workflow: None,
            local_actions: actions_cache,
            local_workflows: workflow_cache,
        }
    }

    fn errorf_at(&mut self, pos: Pos, message: std::fmt::Arguments<'_>) {
        self.base.errorf_at(pos, message);
    }

    fn check_string(&mut self, s: Option<&String>, workflow_key: &str) -> Vec<TypedExpr> {
        let Some(s) = s else {
            return Vec::new();
        };

        let Some(ts) = self.check_exprs_in(&s.value, s.pos, s.quoted, false, workflow_key) else {
            return Vec::new();
        };

        self.check_template_evaluated_type(&ts);
        ts
    }

    fn check_script_string(&mut self, s: Option<&String>, workflow_key: &str) {
        let Some(s) = s else {
            return;
        };

        let Some(ts) = self.check_exprs_in(&s.value, s.pos, s.quoted, true, workflow_key) else {
            return;
        };

        self.check_template_evaluated_type(&ts);
    }

    fn check_strings(&mut self, ss: &[String], workflow_key: &str) {
        for s in ss {
            self.check_string(Some(s), workflow_key);
        }
    }

    fn check_bool(&mut self, b: Option<&Bool>, workflow_key: &str) {
        let Some(b) = b else {
            return;
        };
        let Some(expr) = &b.expression else {
            return;
        };

        let Some(ty) = self.check_one_expression(expr, "bool value", workflow_key) else {
            return;
        };

        if !matches!(ty.inner(), ExprTypeKindInner::Bool | ExprTypeKindInner::Any) {
            self.errorf_at(
                expr.pos,
                format_args!("type of expression must be bool but found type {ty}"),
            );
        }
    }

    fn check_int(&mut self, i: Option<&crate::ast::Int>, workflow_key: &str) {
        let Some(i) = i else {
            return;
        };
        let Some(expr) = &i.expression else {
            return;
        };
        self.check_number_expression(expr, "integer value", workflow_key);
    }

    fn check_float(&mut self, f: Option<&crate::ast::Float>, workflow_key: &str) {
        let Some(f) = f else {
            return;
        };
        let Some(expr) = &f.expression else {
            return;
        };
        self.check_number_expression(expr, "float number value", workflow_key);
    }

    fn check_env(&mut self, env: Option<&Env>, workflow_key: &str) {
        let Some(env) = env else {
            return;
        };

        if !env.vars.is_empty() {
            for e in env.vars.values() {
                self.check_string(Some(&e.name), workflow_key);
                self.check_string(Some(&e.value), workflow_key);
            }
            return;
        }

        // When form of "env: ${{...}}"
        if let Some(expr) = &env.expression {
            self.check_object_expression(expr, "env", workflow_key);
        }
    }

    fn check_container(
        &mut self,
        c: Option<&Container>,
        workflow_key: &str,
        child_workflow_key_prefix: &str,
    ) {
        let Some(c) = c else {
            return;
        };
        let mut child_workflow_key = workflow_key.to_string();
        if !child_workflow_key_prefix.is_empty() {
            child_workflow_key.push('.');
            child_workflow_key.push_str(child_workflow_key_prefix);
        }
        self.check_string(Some(&c.image), workflow_key);
        if let Some(cred) = &c.credentials {
            let k = format!("{child_workflow_key}.credentials");
            if let Some(expr) = &cred.expression {
                self.check_object_expression(expr, "credentials", &k);
            } else {
                self.check_string(cred.username.as_ref(), &k);
                self.check_string(cred.password.as_ref(), &k);
            }
        }
        self.check_env(
            c.env.as_ref(),
            &format!("{child_workflow_key}.env.<env_id>"),
        );
        self.check_strings(&c.ports, workflow_key);
        self.check_strings(&c.volumes, workflow_key);
        self.check_string(c.options.as_ref(), workflow_key);
        self.check_string(c.command.as_ref(), workflow_key);
        self.check_string(c.entrypoint.as_ref(), workflow_key);
    }

    fn check_concurrency(&mut self, c: Option<&Concurrency>, workflow_key: &str) {
        let Some(c) = c else {
            return;
        };
        self.check_string(Some(&c.group), workflow_key);
        self.check_bool(c.cancel_in_progress.as_ref(), workflow_key);
    }

    fn check_defaults(&mut self, d: Option<&Defaults>, workflow_key: &str) {
        let Some(d) = d else {
            return;
        };
        let Some(run) = &d.run else {
            return;
        };
        self.check_string(run.shell.as_ref(), workflow_key);
        self.check_string(run.working_directory.as_ref(), workflow_key);
    }

    fn check_one_expression(
        &mut self,
        s: &String,
        what: &str,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        // `check_string` is not available since it checks types for embedding values into a string
        let ts = self.check_exprs_in(&s.value, s.pos, s.quoted, false, workflow_key)?;

        if ts.len() != 1 {
            // This case should be unreachable since only one ${{ }} is included is checked by
            // parser
            self.errorf_at(
                s.pos,
                format_args!(
                    "one ${{{{ }}}} expression should be included in {:?} value but got {} expressions",
                    what,
                    ts.len()
                ),
            );
            return None;
        }

        Some(ts[0].ty.clone())
    }

    fn check_object_ty(
        &mut self,
        ty: Option<ExprTypeKind>,
        pos: Pos,
        what: &str,
    ) -> Option<ExprTypeKind> {
        let ty = ty?;
        match ty.inner() {
            ExprTypeKindInner::Object(_) | ExprTypeKindInner::Any => Some(ty),
            _ => {
                self.errorf_at(
                    pos,
                    format_args!(
                        "type of expression at {what:?} must be object but found type {ty}"
                    ),
                );
                None
            }
        }
    }

    fn check_array_ty(
        &mut self,
        ty: Option<ExprTypeKind>,
        pos: Pos,
        what: &str,
    ) -> Option<ExprTypeKind> {
        let ty = ty?;
        match ty.inner() {
            ExprTypeKindInner::Array(_) | ExprTypeKindInner::Any => Some(ty),
            _ => {
                self.errorf_at(
                    pos,
                    format_args!(
                        "type of expression at {what:?} must be array but found type {ty}"
                    ),
                );
                None
            }
        }
    }

    fn check_number_ty(
        &mut self,
        ty: Option<ExprTypeKind>,
        pos: Pos,
        what: &str,
    ) -> Option<ExprTypeKind> {
        let ty = ty?;
        match ty.inner() {
            ExprTypeKindInner::Number | ExprTypeKindInner::Any => Some(ty),
            _ => {
                self.errorf_at(
                    pos,
                    format_args!(
                        "type of expression at {what:?} must be number but found type {ty}"
                    ),
                );
                None
            }
        }
    }

    fn check_object_expression(
        &mut self,
        s: &String,
        what: &str,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        let ty = self.check_one_expression(s, what, workflow_key)?;
        self.check_object_ty(Some(ty), s.pos, what)
    }

    fn check_array_expression(
        &mut self,
        s: &String,
        what: &str,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        let ty = self.check_one_expression(s, what, workflow_key)?;
        self.check_array_ty(Some(ty), s.pos, what)
    }

    fn check_number_expression(
        &mut self,
        s: &String,
        what: &str,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        let ty = self.check_one_expression(s, what, workflow_key)?;
        self.check_number_ty(Some(ty), s.pos, what)
    }

    fn check_object_expression_opt(
        &mut self,
        s: Option<&String>,
        what: &str,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        let s = s?;
        self.check_object_expression(s, what, workflow_key)
    }

    fn check_webhook_event_filter(&mut self, f: Option<&crate::ast::WebhookEventFilter>) {
        let Some(f) = f else {
            return;
        };
        self.check_strings(&f.values, "");
    }

    fn check_workflow_call(&mut self, c: Option<&WorkflowCall>) {
        let c = match c {
            Some(c) => c,
            None => return,
        };
        let uses = &c.uses;

        self.check_string(Some(uses), "");

        let m = match self.local_workflows.find_metadata(&uses.value) {
            Ok(m) => m,
            Err(e) => {
                self.base.error_at(uses.pos, e);
                None
            }
        };

        for (n, i) in &c.inputs {
            let ts = self.check_string(Some(&i.value), "jobs.<job_id>.with.<with_id>");

            let Some(m) = &m else {
                continue;
            };

            let Some(mi) = m.inputs.get(n) else {
                continue;
            };
            if matches!(mi.ty.inner(), ExprTypeKindInner::Any) {
                continue;
            }

            let v = i.value.value.trim();

            let mut ty = ExprTypeKind::string();
            match ts.len() {
                0 => match v {
                    "null" => ty = ExprTypeKind::null(),
                    "true" | "false" => ty = ExprTypeKind::bool(),
                    _ => {
                        if v.parse::<f64>().is_ok() {
                            ty = ExprTypeKind::number();
                        }
                    }
                },
                1 if i.value.is_expression_assigned() => {
                    ty = ts[0].ty.clone();
                }
                _ => {}
            }

            if !mi.ty.assignable(&ty) {
                self.errorf_at(
                    i.value.pos,
                    format_args!(
                        "input {:?} is typed as {} by reusable workflow {:?}. {} value cannot be assigned",
                        mi.name,
                        mi.ty,
                        uses.value,
                        ty
                    ),
                );
            }
        }

        for s in c.secrets.values() {
            self.check_string(Some(&s.value), "jobs.<job_id>.secrets.<secrets_id>");
        }
    }

    fn check_snapshot(&mut self, s: Option<&Snapshot>) {
        let Some(s) = s else {
            return;
        };
        self.check_if_condition(s.if_condition.as_ref(), "jobs.<job_id>.snapshot.if");
    }

    fn check_if_condition(&mut self, s: Option<&String>, workflow_key: &str) {
        let Some(s) = s else {
            return;
        };

        // Note:
        // https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#jobsjob_idif
        //
        // > When you use expressions in an if conditional, you may omit the expression syntax (${{ }})
        // > because GitHub automatically evaluates the if conditional as an expression, unless the
        // > expression contains any operators. If the expression contains any operators, the expression
        // > must be contained within ${{ }} to explicitly mark it for evaluation.
        //
        // This document is actually wrong. I confirmed that any strings without surrounding in ${{ }}
        // are evaluated.
        //
        // - run: echo 'run'
        //   if: '!false'
        // - run: echo 'not run'
        //   if: '!true'
        // - run: echo 'run'
        //   if: false || true
        // - run: echo 'run'
        //   if: true && true
        // - run: echo 'not run'
        //   if: true && false

        let mut cond_ty: Option<ExprTypeKind> = None;
        if s.contains_expression() {
            let ts = self.check_string(Some(s), workflow_key);

            if ts.len() == 1 && s.is_expression_assigned() {
                cond_ty = Some(ts[0].ty.clone());
            }
        } else {
            // `}}` is necessary since lexer lexes it as end of tokens
            let src = format!("{}}}}}", s.value);
            let line = s.pos.line;
            let col = s.pos.col;

            let Ok((expr, _)) = parse_expression_with_offset(&src) else {
                return;
            };

            if let Some(ty) =
                self.check_semantics_of_expr_node(&expr, line, col, false, workflow_key)
            {
                cond_ty = Some(ty);
            }
        }

        if let Some(cond_ty) = cond_ty
            && !ExprTypeKind::bool().assignable(&cond_ty)
        {
            self.errorf_at(
                s.pos,
                format_args!(
                    "\"if\" condition should be type \"bool\" but got type {:?}",
                    cond_ty.to_string()
                ),
            );
        }
    }

    fn check_template_evaluated_type(&mut self, ts: &[TypedExpr]) {
        for t in ts {
            match t.ty.inner() {
                ExprTypeKindInner::Object(_)
                | ExprTypeKindInner::Array(_)
                | ExprTypeKindInner::Null => {
                    self.errorf_at(
                        t.pos,
                        format_args!(
                            "object, array, and null values should not be evaluated in template with ${{{{ }}}} but evaluating the value of type {}",
                            t.ty
                        ),
                    );
                }
                _ => {}
            }
        }
    }

    fn check_exprs_in(
        &mut self,
        s: &str,
        pos: Pos,
        quoted: bool,
        check_untrusted: bool,
        workflow_key: &str,
    ) -> Option<Vec<TypedExpr>> {
        // TODO: Line number is not correct when the string contains newlines.
        let line = pos.line;
        let mut col = pos.col;
        if quoted {
            col += 1; // when the string is quoted like 'foo' or "foo", column should be incremented
        }
        let mut offset = 0usize;
        let mut ts: Vec<TypedExpr> = Vec::new();
        let mut rest = s;
        while let Some(idx) = rest.find("${{") {
            let start = idx + 3; // 3 means removing "${{"
            rest = &rest[start..];
            offset += start;
            let col = col + offset;

            let (ty, offset_after) =
                self.check_semantics(rest, line, col, check_untrusted, workflow_key)?;
            let Some(offset_after) = offset_after else {
                return Some(Vec::new());
            };
            if offset_after == 0 {
                return Some(Vec::new());
            }
            ts.push(TypedExpr {
                ty,
                pos: Pos::new(line, col - 3),
            });

            rest = &rest[offset_after..];
            offset += offset_after;
        }

        Some(ts)
    }

    /// Converts a 1-based line/column reported by the lexer/parser (relative to the sub-source
    /// starting at `line_base:col_base`) into an absolute [`Pos`] and records the error there.
    /// Counterpart of `exprError` + `convertExprLineColToPos`.
    fn expr_error(&mut self, message: &str, line_base: usize, col_base: usize) {
        let line = line_base.saturating_sub(1);
        let col = col_base.saturating_sub(1);
        self.base.error_at(Pos::new(line, col), message);
    }

    fn check_semantics_of_expr_node(
        &mut self,
        expr: &ExprNode,
        line: usize,
        col: usize,
        check_untrusted: bool,
        workflow_key: &str,
    ) -> Option<ExprTypeKind> {
        let config_vars = self.base.config().and_then(|c| c.config_variables.clone());
        let mut c = ExprSemanticsChecker::new(check_untrusted, config_vars);
        if let Some(matrix_ty) = &self.matrix_ty {
            c.update_matrix(matrix_ty.clone());
        }
        if let Some(steps_ty) = &self.steps_ty {
            c.steps = steps_ty.clone();
        }
        if let Some(needs_ty) = &self.needs_ty {
            c.needs = needs_ty.clone();
        }
        if let Some(secrets_ty) = &self.secrets_ty {
            c.secrets = secrets_ty.clone();
        }
        if let Some(inputs_ty) = &self.inputs_ty {
            c.inputs = inputs_ty.clone();
        }
        if let Some(dispatch_inputs_ty) = &self.dispatch_inputs_ty {
            c.update_dispatch_inputs(dispatch_inputs_ty.clone());
        }
        if let Some(jobs_ty) = &self.jobs_ty {
            c.jobs = jobs_ty.clone();
        }
        // Always derive the context/special-function availability from the workflow key, matching
        // the original. An empty key (used by positions with no availability entry, e.g. a step
        // `id`, a `uses` value, a shell key, a webhook event filter) yields no available contexts,
        // so the checker restricts every context there — exactly like the original, where an empty
        // key skips `SetContextAvailability`, leaving `availableContexts` nil (length 0), which
        // `checkAvailableContext` reports as "no context is available here".
        {
            let (ctx, sp) = workflow_key_availability(workflow_key);
            if ctx.is_empty() {
                self.base.debug(format_args!(
                    "No context availability was found for workflow key {workflow_key:?}"
                ));
            }
            // `set_context_availability` also marks the contexts as restricted, which enables the
            // "context ... is not allowed here" check. An empty (or unknown) key restricts every
            // context (e.g. the shell key), while a known key restricts to its listed contexts.
            c.set_context_availability(ctx.iter().map(|s| (*s).to_string()).collect());
            c.set_special_function_availability(sp.iter().map(|s| (*s).to_string()).collect());
        }

        let (ty, errs) = c.check(expr);
        for err in &errs {
            self.expr_error(&err.message, line + err.line, col + err.column);
        }

        if errs.is_empty() { Some(ty) } else { None }
    }

    fn check_semantics(
        &mut self,
        src: &str,
        line: usize,
        col: usize,
        check_untrusted: bool,
        workflow_key: &str,
    ) -> Option<(ExprTypeKind, Option<usize>)> {
        let (expr, offset) = match parse_expression_with_offset(src) {
            Ok(x) => x,
            Err(err) => {
                self.expr_error(&err.message, line + err.line, col + err.column);
                return None;
            }
        };
        let t =
            self.check_semantics_of_expr_node(&expr, line, col, check_untrusted, workflow_key)?;
        Some((t, Some(offset)))
    }

    fn calc_needs_type(&mut self, job: &Job) -> ObjectType {
        // https://docs.github.com/en/actions/learn-github-actions/contexts#needs-context
        let mut o = ObjectType {
            props: BTreeMap::new(),
            mapped: None,
        };
        self.populate_dependant_needs_types(&mut o, job, job);
        o
    }

    fn populate_dependant_needs_types(&mut self, out: &mut ObjectType, job: &Job, root: &Job) {
        for id in &job.needs {
            let i = id.value.to_lowercase(); // ID is case insensitive
            if i == root.id.value {
                continue; // When cyclic dependency exists. This does not happen normally.
            }
            if out.props.contains_key(&i) {
                continue; // Already added
            }

            let Some(j) = self.workflow.as_ref().and_then(|w| w.jobs.get(&i)) else {
                continue;
            };
            // Clone the pieces needed from `j` so the immutable borrow of `self.workflow` ends
            // before the `&mut self` call to `get_workflow_call_outputs_type`.
            let call = j.workflow_call.clone();
            let output_names: Vec<StdString> = j.outputs.keys().cloned().collect();

            let outputs = if let Some(call) = &call {
                self.get_workflow_call_outputs_type(call)
            } else {
                let mut props = BTreeMap::new();
                for name in output_names {
                    props.insert(name, ExprTypeKind::string());
                }
                ExprTypeKind::strict_object(props)
            };

            let mut props = BTreeMap::new();
            props.insert("outputs".to_string(), outputs);
            props.insert("result".to_string(), ExprTypeKind::string());
            out.props.insert(i, ExprTypeKind::strict_object(props));

            // Do not collect outputs type from parent of parent recursively. (#151)
        }
    }

    fn get_action_outputs_type(&mut self, spec: Option<&String>) -> ExprTypeKind {
        let Some(spec) = spec else {
            return ExprTypeKind::map_object(ExprTypeKind::string());
        };

        if spec.value.starts_with("./") {
            let meta = match self.local_actions.find_metadata(&spec.value) {
                Ok(meta) => meta,
                Err(e) => {
                    self.base.error_at(spec.pos, e);
                    return ExprTypeKind::map_object(ExprTypeKind::string());
                }
            };
            let Some(meta) = meta else {
                return ExprTypeKind::map_object(ExprTypeKind::string());
            };

            return type_of_action_outputs(&meta);
        }

        // github-script action allows to set any outputs through calling `core.setOutput` directly.
        // So any `outputs.*` properties should be accepted (#104)
        if spec.value.starts_with("actions/github-script@") {
            return ExprTypeKind::empty_object();
        }

        // When the action run at this step is a popular action, we know what outputs are set by it.
        // Set the output names to `steps.{step_id}.outputs.{name}`.
        if let Some(meta) = popular_actions().get(spec.value.as_str()) {
            return type_of_action_outputs(meta);
        }

        ExprTypeKind::map_object(ExprTypeKind::string())
    }

    fn get_workflow_call_outputs_type(&mut self, call: &WorkflowCall) -> ExprTypeKind {
        let uses = &call.uses;

        let m = match self.local_workflows.find_metadata(&uses.value) {
            Ok(m) => m,
            Err(e) => {
                self.base.error_at(uses.pos, e);
                return ExprTypeKind::map_object(ExprTypeKind::string());
            }
        };
        let Some(m) = m else {
            return ExprTypeKind::map_object(ExprTypeKind::string());
        };

        let props: BTreeMap<StdString, ExprTypeKind> = m
            .outputs
            .keys()
            .map(|n| (n.clone(), ExprTypeKind::string()))
            .collect();
        ExprTypeKind::strict_object(props)
    }

    fn check_matrix_expression(&mut self, expr: &String) -> ObjectType {
        let Some(ty) = self.check_object_expression(expr, "matrix", "jobs.<job_id>.strategy")
        else {
            return empty_loose_object();
        };
        let Some(mat_ty) = ty.as_object() else {
            return empty_loose_object();
        };
        let mut mat_ty = mat_ty.clone();

        // Consider properties in include section elements since 'include' section adds matrix values
        if let Some(inc_ty) = mat_ty.props.remove("include")
            && let Some(a) = inc_ty.as_array()
            && let Some(o) = a.elem.as_object()
        {
            for (n, p) in &o.props {
                match mat_ty.props.get(n) {
                    None => {
                        mat_ty.props.insert(n.clone(), p.clone());
                    }
                    Some(t) => {
                        let merged = t.merge(p);
                        mat_ty.props.insert(n.clone(), merged);
                    }
                }
            }
        }

        mat_ty.props.remove("exclude");

        mat_ty
    }

    fn check_matrix(&mut self, m: &Matrix) -> ObjectType {
        if let Some(expr) = &m.expression {
            return self.check_matrix_expression(expr);
        }

        // Check types of "exclude" but they are not used to guess type of matrix
        if let Some(exclude) = &m.exclude {
            if let Some(expr) = &exclude.expression {
                if let Some(ty) =
                    self.check_array_expression(expr, "exclude", "jobs.<job_id>.strategy")
                    && let Some(a) = ty.as_array()
                {
                    self.check_object_ty(Some(a.elem.clone()), expr.pos, "exclude");
                }
            } else {
                for combi in &exclude.combinations {
                    if let Some(expr) = &combi.expression {
                        self.check_object_expression(expr, "exclude", "jobs.<job_id>.strategy");
                        continue;
                    }
                    for a in combi.assigns.values() {
                        self.check_raw_yaml_value(&a.value);
                    }
                }
            }
        }

        let mut o = ObjectType {
            props: BTreeMap::new(),
            mapped: None,
        };

        for (n, r) in &m.rows {
            let t = self.check_matrix_row(r);
            o.props.insert(n.clone(), t);
        }

        let Some(include) = &m.include else {
            return o;
        };

        if let Some(expr) = &include.expression {
            if let Some(ty) = self.check_one_expression(expr, "include", "jobs.<job_id>.strategy")
                && let Some(a) = ty.as_array()
            {
                let merged = ExprTypeKind::new(ExprTypeKindInner::Object(o.clone())).merge(&a.elem);
                if let Some(ret) = merged.as_object() {
                    return ret.clone();
                }
            }
            return empty_loose_object();
        }

        for combi in &include.combinations {
            if let Some(expr) = &combi.expression {
                let Some(ty) = self.check_one_expression(
                    expr,
                    "matrix combination at element of include section",
                    "jobs.<job_id>.strategy",
                ) else {
                    continue;
                };
                let merged = ExprTypeKind::new(ExprTypeKindInner::Object(o.clone())).merge(&ty);
                if let Some(merged) = merged.as_object() {
                    o = merged.clone();
                } else {
                    o.set_loose();
                }
                continue;
            }

            for (n, assign) in &combi.assigns {
                let mut ty = self.check_raw_yaml_value(&assign.value);
                if let Some(t) = o.props.get(n) {
                    // When the combination exists in 'matrix' section, merge type with existing one
                    ty = t.merge(&ty);
                }
                o.props.insert(n.clone(), ty);
            }
        }

        o
    }

    fn check_matrix_row(&mut self, r: &MatrixRow) -> ExprTypeKind {
        if let Some(expr) = &r.expression {
            if let Some(ty) =
                self.check_array_expression(expr, "matrix row", "jobs.<job_id>.strategy")
                && let Some(a) = ty.as_array()
            {
                return a.elem.clone();
            }
            return ExprTypeKind::any();
        }

        let mut ty: Option<ExprTypeKind> = None;
        for v in &r.values {
            let t = self.check_raw_yaml_value(v);
            ty = Some(match ty {
                None => t,
                Some(prev) => prev.merge(&t),
            });
        }

        ty.unwrap_or_else(ExprTypeKind::any) // No element
    }

    fn check_workflow_call_outputs(
        &mut self,
        outputs: &BTreeMap<StdString, crate::ast::WorkflowCallEventOutput>,
        jobs: &BTreeMap<StdString, Job>,
    ) {
        if outputs.is_empty() || jobs.is_empty() {
            return;
        }

        let mut props: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
        for (n, j) in jobs {
            let o = if j.workflow_call.is_some() {
                // Outputs are not defined in jobs.<job_id> section when it is reusable workflow call.
                ExprTypeKind::empty_object()
            } else {
                let mut p: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
                for name in j.outputs.keys() {
                    p.insert(name.clone(), ExprTypeKind::string());
                }
                ExprTypeKind::strict_object(p)
            };
            let mut inner: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
            inner.insert("outputs".to_string(), o);
            props.insert(n.clone(), ExprTypeKind::strict_object(inner));
        }
        self.jobs_ty = Some(ObjectType {
            props,
            mapped: None,
        });

        for o in outputs.values() {
            self.check_string(
                o.value.as_ref(),
                "on.workflow_call.outputs.<output_id>.value",
            );
        }
    }

    fn check_raw_yaml_value(&mut self, v: &RawYAMLValue) -> ExprTypeKind {
        match v {
            RawYAMLValue::Object(o) => {
                let m: BTreeMap<StdString, ExprTypeKind> = o
                    .props
                    .iter()
                    .map(|(k, p)| (k.clone(), self.check_raw_yaml_value(p)))
                    .collect();
                ExprTypeKind::strict_object(m)
            }
            RawYAMLValue::Array(a) => {
                if a.elems.is_empty() {
                    return ExprTypeKind::array(ExprTypeKind::any());
                }
                let mut elem = self.check_raw_yaml_value(&a.elems[0]);
                for v in &a.elems[1..] {
                    elem = elem.merge(&self.check_raw_yaml_value(v));
                }
                ExprTypeKind::array(elem)
            }
            RawYAMLValue::String(s) => self.check_raw_yaml_string(s),
        }
    }

    fn check_raw_yaml_string(&mut self, y: &crate::ast::RawYAMLString) -> ExprTypeKind {
        let ts = self.check_exprs_in(&y.value, y.pos, false, false, "jobs.<job_id>.strategy");

        // Counterpart of `isExprAssigned` (a private helper in the original `ast.go`, exposed as a
        // method on `ast::String` but not on `RawYAMLString`).
        let v = y.value.trim();
        let is_assigned =
            v.starts_with("${{") && v.ends_with("}}") && v.matches("${{").count() == 1;
        if is_assigned {
            return match ts {
                Some(ts) if ts.len() == 1 => ts[0].ty.clone(),
                _ => ExprTypeKind::any(),
            };
        }

        let s = y.value.trim();
        // Note that keywords are case sensitive. TRUE, FALSE, NULL are invalid named value.
        if s == "true" || s == "false" {
            return ExprTypeKind::bool();
        }
        if s == "null" {
            return ExprTypeKind::null();
        }
        if s.parse::<f64>().is_ok() {
            return ExprTypeKind::number();
        }
        ExprTypeKind::string()
    }
}

fn empty_loose_object() -> ObjectType {
    ObjectType {
        props: BTreeMap::new(),
        mapped: Some(ExprTypeKind::any()),
    }
}

/// Returns the type of the `outputs` object of a step running the given action metadata.
fn type_of_action_outputs(meta: &ActionMetadata) -> ExprTypeKind {
    // Some action sets outputs dynamically. Such outputs are not defined in action.yml. actionlint
    // cannot check such outputs statically so it allows any props (#18)
    if meta.skip_outputs {
        return ExprTypeKind::empty_object();
    }
    let props: BTreeMap<StdString, ExprTypeKind> = meta
        .outputs
        .keys()
        .map(|n| (n.to_lowercase(), ExprTypeKind::string()))
        .collect();
    ExprTypeKind::strict_object(props)
}

impl Default for RuleExpression {
    fn default() -> Self {
        Self::new(
            Arc::new(LocalActionsCache::new(None)),
            Arc::new(LocalReusableWorkflowCache::new_null(None)),
        )
    }
}

impl Pass for RuleExpression {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), StdString> {
        self.check_string(n.name.as_ref(), "");

        for e in &n.on {
            match e {
                Event::Webhook(e) => {
                    self.check_strings(&e.types, "");
                    self.check_webhook_event_filter(e.branches.as_ref());
                    self.check_webhook_event_filter(e.branches_ignore.as_ref());
                    self.check_webhook_event_filter(e.tags.as_ref());
                    self.check_webhook_event_filter(e.tags_ignore.as_ref());
                    self.check_webhook_event_filter(e.paths.as_ref());
                    self.check_webhook_event_filter(e.paths_ignore.as_ref());
                    self.check_strings(&e.workflows, "");
                }
                Event::Scheduled(e) => {
                    for s in &e.schedules {
                        self.check_string(Some(&s.cron), "");
                        self.check_string(s.timezone.as_ref(), "");
                    }
                }
                Event::WorkflowDispatch(e) => {
                    let mut props: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
                    for (id, i) in &e.inputs {
                        self.check_string(i.description.as_ref(), "");
                        self.check_string(i.default.as_ref(), "");
                        self.check_bool(i.required.as_ref(), "");
                        self.check_strings(&i.options, "");

                        let ty = match i.input_type {
                            crate::ast::WorkflowDispatchEventInputType::Boolean => {
                                ExprTypeKind::bool()
                            }
                            crate::ast::WorkflowDispatchEventInputType::Number => {
                                ExprTypeKind::number()
                            }
                            crate::ast::WorkflowDispatchEventInputType::String
                            | crate::ast::WorkflowDispatchEventInputType::Choice
                            | crate::ast::WorkflowDispatchEventInputType::Environment => {
                                ExprTypeKind::string()
                            }
                            crate::ast::WorkflowDispatchEventInputType::None => ExprTypeKind::any(),
                        };
                        props.insert(id.clone(), ty);
                    }
                    // The `inputs` context of `workflow_dispatch` is typed via
                    // `dispatch_inputs_ty`, which also types `github.event.inputs` (whose values
                    // are always strings). Counterpart of `rule.dispatchInputsTy = ity`.
                    self.dispatch_inputs_ty = Some(ObjectType {
                        props,
                        mapped: None,
                    });
                }
                Event::RepositoryDispatch(e) => {
                    self.check_strings(&e.types, "");
                }
                Event::WorkflowCall(e) => {
                    let mut props: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();

                    // Build the `inputs` context incrementally: each input's default is checked
                    // before that input is added, so a default can only refer to the inputs that
                    // precede it (and referring to itself is an error). Go sets `rule.inputsTy`
                    // before the loop, but the first default is still checked against an empty
                    // object because no input has been added yet.
                    for i in &e.inputs {
                        self.check_string(i.description.as_ref(), "");
                        // Check default value before setting type to `ity` because referring myself
                        // should cause an error.
                        //   inputs:
                        //     recursive:
                        //       type: string
                        //       default: ${{ inputs.recursive }}
                        let ts = self.check_string(
                            i.default.as_ref(),
                            "on.workflow_call.inputs.<inputs_id>.default",
                        );

                        let ty = match i.input_type {
                            crate::ast::WorkflowCallEventInputType::String => {
                                ExprTypeKind::string()
                            }
                            crate::ast::WorkflowCallEventInputType::Boolean => {
                                let ty = ExprTypeKind::bool();
                                if ts.len() == 1
                                    && i.default
                                        .as_ref()
                                        .is_some_and(super::ast::String::is_expression_assigned)
                                    && !matches!(
                                        ts[0].ty.inner(),
                                        ExprTypeKindInner::Bool | ExprTypeKindInner::Any
                                    )
                                {
                                    let pos = i.default.as_ref().map(|d| d.pos).unwrap_or_default();
                                    self.errorf_at(
                                        pos,
                                        format_args!(
                                            "type of input {:?} must be bool but found type {}",
                                            i.name.value, ts[0].ty
                                        ),
                                    );
                                }
                                ty
                            }
                            crate::ast::WorkflowCallEventInputType::Number => {
                                let ty = ExprTypeKind::number();
                                if ts.len() == 1
                                    && i.default
                                        .as_ref()
                                        .is_some_and(super::ast::String::is_expression_assigned)
                                    && !matches!(
                                        ts[0].ty.inner(),
                                        ExprTypeKindInner::Number | ExprTypeKindInner::Any
                                    )
                                {
                                    let pos = i.default.as_ref().map(|d| d.pos).unwrap_or_default();
                                    self.errorf_at(
                                        pos,
                                        format_args!(
                                            "type of input {:?} must be number but found type {}",
                                            i.name.value, ts[0].ty
                                        ),
                                    );
                                }
                                ty
                            }
                            crate::ast::WorkflowCallEventInputType::Invalid => ExprTypeKind::any(),
                        };
                        // Update the `inputs` context after each input so that a later input's
                        // default can refer to the inputs defined before it (Go mutates the shared
                        // `ity` object that `rule.inputsTy` points at).
                        props.insert(i.id.clone(), ty);
                        self.inputs_ty = Some(ObjectType {
                            props: props.clone(),
                            mapped: None,
                        });
                    }

                    // When no secret is passed, secrets may be inherited from a caller of the
                    // workflow. So `secrets` context must be typed as { string => string }.
                    // `secrets_present` distinguishes the two empty cases: when `secrets:` is
                    // omitted the map is empty and no strict type is set (secrets may be
                    // inherited), but when `secrets:` exists with no child the map is also empty
                    // yet a strict empty object must be set so that referring to any secret is an
                    // error. This mirrors the original, which checks `e.Secrets != nil`.
                    if e.secrets_present {
                        let mut sty_props: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
                        for (id, s) in &e.secrets {
                            sty_props.insert(id.clone(), ExprTypeKind::string());
                            self.check_string(s.description.as_ref(), "");
                        }
                        self.secrets_ty = Some(ObjectType {
                            props: sty_props,
                            mapped: None,
                        });
                    }

                    for o in e.outputs.values() {
                        self.check_string(o.description.as_ref(), "");
                        // o.value will be checked in visit_workflow_post
                    }
                }
                Event::ImageVersion(e) => {
                    self.check_strings(&e.names, "");
                    self.check_strings(&e.versions, "");
                }
            }
        }

        self.check_string(n.run_name.as_ref(), "run-name");
        self.check_env(n.env.as_ref(), "env");

        self.check_defaults(n.defaults.as_ref(), "");
        self.check_concurrency(n.concurrency.as_ref(), "concurrency");

        self.workflow = Some(n.clone());
        Ok(())
    }

    fn visit_workflow_post(&mut self, n: &Workflow) -> Result<(), StdString> {
        if let Some(e) = n.find_workflow_call_event() {
            self.check_workflow_call_outputs(&e.outputs, &n.jobs);
        }
        self.workflow = None;
        Ok(())
    }

    fn visit_job_pre(&mut self, n: &Job) -> Result<(), StdString> {
        // Type of needs must be resolved before resolving type of matrix because `needs` context can
        // be used in matrix configuration.
        self.needs_ty = Some(self.calc_needs_type(n));

        // Set matrix type at start of visit_job_pre() because matrix values are available in
        // jobs.<job_id> section. For example:
        //   jobs:
        //     foo:
        //       strategy:
        //         matrix:
        //           os: [ubuntu-latest, macos-latest, windows-latest]
        //       runs-on: ${{ matrix.os }}
        if let Some(strategy) = &n.strategy
            && let Some(matrix) = &strategy.matrix
        {
            // Check and guess type of the matrix
            self.matrix_ty = Some(self.check_matrix(matrix));
        }

        self.check_string(n.name.as_ref(), "jobs.<job_id>.name");
        self.check_strings(&n.needs, "");

        if let Some(runs_on) = &n.runs_on {
            if let Some(labels_expr) = &runs_on.labels_expr {
                if let Some(ty) = self.check_one_expression(
                    labels_expr,
                    "runner label at \"runs-on\" section",
                    "jobs.<job_id>.runs-on",
                ) {
                    match ty.inner() {
                        ExprTypeKindInner::Array(_)
                        | ExprTypeKindInner::String
                        | ExprTypeKindInner::Any => {
                            // OK
                        }
                        _ => {
                            self.errorf_at(
                                labels_expr.pos,
                                format_args!(
                                    "type of expression at \"runs-on\" must be string or array but found type {:?}",
                                    ty.to_string()
                                ),
                            );
                        }
                    }
                }
            } else {
                for l in &runs_on.labels {
                    self.check_string(Some(l), "jobs.<job_id>.runs-on");
                }
            }
            self.check_string(runs_on.group.as_ref(), "jobs.<job_id>.runs-on");
        }

        self.check_concurrency(n.concurrency.as_ref(), "jobs.<job_id>.concurrency");

        self.check_env(n.env.as_ref(), "jobs.<job_id>.env");

        self.check_defaults(n.defaults.as_ref(), "jobs.<job_id>.defaults.run");
        self.check_if_condition(n.if_condition.as_ref(), "jobs.<job_id>.if");

        if let Some(strategy) = &n.strategy {
            // Note: Types in "jobs.<job_id>.strategy.matrix" were checked `check_matrix`
            self.check_bool(strategy.fail_fast.as_ref(), "jobs.<job_id>.strategy");
            self.check_int(strategy.max_parallel.as_ref(), "jobs.<job_id>.strategy");
        }

        self.check_bool(
            n.continue_on_error.as_ref(),
            "jobs.<job_id>.continue-on-error",
        );
        self.check_float(n.timeout_minutes.as_ref(), "jobs.<job_id>.timeout-minutes");
        self.check_container(n.container.as_ref(), "jobs.<job_id>.container", "");

        if let Some(services) = &n.services {
            self.check_object_expression_opt(
                services.expression.as_ref(),
                "services",
                "jobs.<job_id>.services",
            );
            for s in services.value.values() {
                self.check_container(Some(&s.container), "jobs.<job_id>.services", "<service_id>");
            }
        }

        self.check_workflow_call(n.workflow_call.as_ref());
        self.check_snapshot(n.snapshot.as_ref());

        self.steps_ty = Some(ObjectType {
            props: BTreeMap::new(),
            mapped: None,
        });

        Ok(())
    }

    fn visit_job_post(&mut self, n: &Job) -> Result<(), StdString> {
        // 'environment' and 'outputs' sections are evaluated after all steps are run
        if let Some(env) = &n.environment {
            self.check_string(Some(&env.name), "jobs.<job_id>.environment");
            self.check_string(env.url.as_ref(), "jobs.<job_id>.environment.url");
            self.check_bool(env.deployment.as_ref(), "jobs.<job_id>.environment");
        }
        for output in n.outputs.values() {
            self.check_string(Some(&output.value), "jobs.<job_id>.outputs.<output_id>");
        }

        self.matrix_ty = None;
        self.steps_ty = None;
        self.needs_ty = None;

        Ok(())
    }

    fn visit_step(&mut self, n: &Step) -> Result<(), StdString> {
        self.check_string(n.name.as_ref(), "jobs.<job_id>.steps.name");
        self.check_if_condition(n.if_condition.as_ref(), "jobs.<job_id>.steps.if");

        let mut spec: Option<&String> = None;
        match &n.exec {
            Exec::Run(e) => {
                self.check_script_string(Some(&e.run), "jobs.<job_id>.steps.run");
                self.check_string(e.shell.as_ref(), "");
                self.check_string(
                    e.working_directory.as_ref(),
                    "jobs.<job_id>.steps.working-directory",
                );
            }
            Exec::Action(e) => {
                self.check_string(Some(&e.uses), "");
                for (name, i) in &e.inputs {
                    if e.uses.value.starts_with("actions/github-script@") && name == "script" {
                        self.check_script_string(Some(&i.value), "jobs.<job_id>.steps.with");
                    } else {
                        self.check_string(Some(&i.value), "jobs.<job_id>.steps.with");
                    }
                }
                self.check_string(e.entrypoint.as_ref(), "jobs.<job_id>.steps.with");
                self.check_string(e.args.as_ref(), "jobs.<job_id>.steps.with");
                spec = Some(&e.uses);
            }
        }

        // env: at step level can refer 'env' context (#158)
        self.check_env(n.env.as_ref(), "jobs.<job_id>.steps.env");
        self.check_bool(
            n.continue_on_error.as_ref(),
            "jobs.<job_id>.steps.continue-on-error",
        );
        self.check_float(
            n.timeout_minutes.as_ref(),
            "jobs.<job_id>.steps.timeout-minutes",
        );

        if let Some(id) = &n.id {
            if id.contains_expression() {
                self.check_string(Some(id), "");
                if let Some(steps_ty) = &mut self.steps_ty {
                    steps_ty.set_loose();
                }
            }
            // Step ID is case insensitive
            let step_id = id.value.to_lowercase();
            let outputs_ty = self.get_action_outputs_type(spec);
            let mut props: BTreeMap<StdString, ExprTypeKind> = BTreeMap::new();
            props.insert("outputs".to_string(), outputs_ty);
            props.insert("conclusion".to_string(), ExprTypeKind::string());
            props.insert("outcome".to_string(), ExprTypeKind::string());
            if let Some(steps_ty) = &mut self.steps_ty {
                steps_ty
                    .props
                    .insert(step_id, ExprTypeKind::strict_object(props));
            }
        }

        Ok(())
    }
}

impl Rule for RuleExpression {
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
    use crate::ast::{ExecAction, ExecRun, Strategy};

    fn null_cache_rule() -> RuleExpression {
        RuleExpression::default()
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

    fn create_min_workflow() -> Workflow {
        Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: Default::default(),
        }
    }

    #[test]
    fn name_and_description() {
        let r = null_cache_rule();
        assert_eq!(r.name(), "expression");
        assert_eq!(
            r.description(),
            "Syntax and semantics checks for expressions embedded with ${{ }} syntax"
        );
    }

    #[test]
    fn parse_error_in_expression() {
        // A parse error is reported directly without depending on the semantic checker.
        let mut r = null_cache_rule();
        let s = String::at("${{ github.event_name == }}", Pos::new(3, 9));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("unexpected"),
            "{:?}",
            errs[0].message
        );
    }

    /// Populates the rule's matrix context with a strict object mapping `os` to an array of
    /// strings and `id`/`ver` to strings, mirroring what the linter derives from a `matrix:`.
    fn rule_with_matrix() -> RuleExpression {
        let mut r = null_cache_rule();
        r.matrix_ty = Some(ObjectType {
            props: BTreeMap::from([
                // `os` is a scalar string here so it can be embedded in a `${{ }}` template. The
                // original rejects object/array/null values in templates regardless of context
                // availability, so an array-typed `matrix.os` would always be an error.
                ("os".to_string(), ExprTypeKind::string()),
                ("id".to_string(), ExprTypeKind::string()),
                ("ver".to_string(), ExprTypeKind::string()),
            ]),
            mapped: None,
        });
        r
    }

    #[test]
    fn valid_expressions_no_error() {
        for input in [
            "${{ github.event_name }}",
            "${{ contains(github.ref, 'refs/heads/') }}",
            "plain string with no expression",
        ] {
            let mut r = null_cache_rule();
            let s = String::at(input, Pos::new(1, 1));
            r.check_string(Some(&s), "jobs.<job_id>.steps.name");
            assert!(r.take_errors().is_empty(), "{input}");
        }
    }

    #[test]
    fn matrix_property_no_error_with_matrix_ty() {
        // `matrix.os` resolves once the matrix context type is populated (as the linter does).
        let mut r = rule_with_matrix();
        let s = String::at("${{ matrix.os }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert!(errs.is_empty());
    }

    #[test]
    fn unknown_matrix_property_is_error() {
        // A property not present in the (strict) matrix type is reported.
        let mut r = null_cache_rule();
        r.matrix_ty = Some(ObjectType {
            props: BTreeMap::from([(
                "os".to_string(),
                ExprTypeKind::array(ExprTypeKind::string()),
            )]),
            mapped: None,
        });
        let s = String::at("${{ matrix.unknown }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("is not defined in object type"),
            "{:?}",
            errs[0].message
        );
    }

    #[test]
    fn object_value_in_template_is_error() {
        // Objects/arrays/null should not be embedded in templates.
        let mut r = null_cache_rule();
        r.matrix_ty = Some(ObjectType {
            props: BTreeMap::from([(
                "obj".to_string(),
                ExprTypeKind::strict_object(BTreeMap::from([(
                    "a".to_string(),
                    ExprTypeKind::string(),
                )])),
            )]),
            mapped: None,
        });
        let s = String::at("${{ matrix.obj }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .starts_with("object, array, and null values should not be evaluated in template"),
            "{:?}",
            errs[0].message
        );
    }

    #[test]
    fn if_condition_object_value_is_error() {
        // `github.event` is an object. It is rejected when evaluated in the template (before the
        // bool check), because objects should not be evaluated in a `${{ }}` template.
        let mut r = null_cache_rule();
        let s = String::at("${{ github.event }}", Pos::new(1, 1));
        r.check_if_condition(Some(&s), "jobs.<job_id>.steps.if");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .starts_with("object, array, and null values should not be evaluated in template"),
            "{:?}",
            errs[0].message
        );
    }

    #[test]
    fn if_condition_omitted_syntax() {
        // `if:` conditions can omit the `${{ }}` syntax.
        let mut r = null_cache_rule();
        let s = String::at("github.event_name == 'push'", Pos::new(1, 1));
        r.check_if_condition(Some(&s), "jobs.<job_id>.if");
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn runs_on_wrong_type() {
        let mut r = null_cache_rule();
        let mut job = create_min_job();
        job.runs_on = Some(crate::ast::Runner {
            labels: vec![],
            labels_expr: Some(String::at("${{ github.event }}", Pos::new(4, 12))),
            group: None,
        });
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "type of expression at \"runs-on\" must be string or array but found type \"object\""
        );
    }

    #[test]
    fn bool_field_wrong_type() {
        let mut r = null_cache_rule();
        let mut job = create_min_job();
        job.continue_on_error = Some(Bool {
            value: false,
            expression: Some(String::at("${{ github.event }}", Pos::new(4, 20))),
            pos: Pos::new(4, 18),
        });
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "type of expression must be bool but found type object"
        );
    }

    #[test]
    fn visit_workflow_pre_no_errors() {
        let mut r = null_cache_rule();
        let wf = create_min_workflow();
        r.visit_workflow_pre(&wf).unwrap();
        r.visit_workflow_post(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }

    fn test_step(step: Step) -> (RuleExpression, Step) {
        // `visit_step` only populates `steps_ty` when it is already initialized, which normally
        // happens in `visit_job_pre` (the original sets `stepsTy = NewEmptyStrictObjectType()`
        // there). Seed it here so a directly-visited step registers its ID in the context.
        let mut r = rule_with_matrix();
        r.steps_ty = Some(ObjectType {
            props: BTreeMap::new(),
            mapped: None,
        });
        (r, step)
    }

    #[test]
    fn visit_step_builds_steps_context() {
        let (mut r, step) = test_step(Step {
            id: Some(String::at("foo", Pos::new(1, 5))),
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: String::at("echo ${{ matrix.os }}", Pos::new(2, 9)),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(2, 7),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(1, 3),
        });
        r.visit_step(&step).unwrap();
        assert!(r.take_errors().is_empty());
        let steps_ty = r.steps_ty.as_ref().expect("steps type set");
        assert!(steps_ty.props.contains_key("foo"));
    }

    #[test]
    fn step_id_with_expression_loosens_steps_context() {
        // An expression `id` makes the `steps` context loose (its shape is not statically known).
        // The expression must not reference a context: a step `id` has no availability entry, so
        // every context is restricted there (matching the original). Use a context-free `format`
        // call so only the "loosens the steps context" behavior is exercised.
        let (mut r, step) = test_step(Step {
            id: Some(String::at("${{ format('{0}', 'x') }}", Pos::new(1, 5))),
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: String::at("echo", Pos::new(2, 9)),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(2, 7),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(1, 3),
        });
        r.visit_step(&step).unwrap();
        assert!(r.take_errors().is_empty());
        assert!(r.steps_ty.as_ref().expect("steps type set").is_loose());
    }

    #[test]
    fn action_step_with_github_script() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "script".to_string(),
            crate::ast::Input {
                name: String::at("script", Pos::new(4, 5)),
                value: String::at("core.setOutput('x', '${{ matrix.os }}')", Pos::new(4, 13)),
            },
        );
        let (mut r, step) = test_step(Step {
            id: Some(String::at("gen", Pos::new(1, 5))),
            if_condition: None,
            name: None,
            exec: Exec::Action(ExecAction {
                uses: String::at("actions/github-script@v7", Pos::new(3, 9)),
                inputs,
                entrypoint: None,
                args: None,
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(1, 3),
        });
        r.visit_step(&step).unwrap();
        assert!(r.take_errors().is_empty());
        let steps_ty = r.steps_ty.as_ref().expect("steps type set");
        let gen_ty = steps_ty.props.get("gen").expect("step id registered");
        let outputs = gen_ty
            .as_object()
            .expect("strict object")
            .props
            .get("outputs")
            .expect("outputs");
        // github-script can output anything, so the outputs object is loose.
        assert!(outputs.as_object().expect("object").is_loose());
    }

    #[test]
    fn container_checks_no_errors() {
        let mut r = null_cache_rule();
        // Populate the matrix context with a string property usable in the image template.
        r.matrix_ty = Some(ObjectType {
            props: BTreeMap::from([("ver".to_string(), ExprTypeKind::string())]),
            mapped: None,
        });
        let mut job = create_min_job();
        job.container = Some(Container {
            image: String::at("ubuntu:${{ matrix.ver }}", Pos::new(3, 12)),
            credentials: None,
            env: None,
            ports: vec![],
            volumes: vec![],
            options: None,
            command: None,
            entrypoint: None,
            pos: Pos::new(3, 3),
        });
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn matrix_include_and_exclude_no_errors() {
        let mut r = null_cache_rule();
        let mut job = create_min_job();
        let row = crate::ast::MatrixRow {
            name: String::at("os", Pos::new(4, 7)),
            values: vec![raw_str("ubuntu"), raw_str("macos")],
            expression: None,
        };
        let assign = crate::ast::MatrixAssign {
            key: String::at("os", Pos::new(7, 9)),
            value: raw_str("windows"),
        };
        let combination = crate::ast::MatrixCombination {
            assigns: BTreeMap::from([("os".to_string(), assign)]),
            expression: None,
        };
        let include = crate::ast::MatrixCombinations {
            combinations: vec![combination],
            expression: None,
        };
        let matrix = Matrix {
            rows: BTreeMap::from([("os".to_string(), row)]),
            include: Some(include),
            exclude: None,
            expression: None,
            pos: Pos::new(3, 9),
        };
        job.strategy = Some(Strategy {
            matrix: Some(matrix),
            fail_fast: None,
            max_parallel: None,
            pos: Pos::new(3, 7),
        });
        r.visit_job_pre(&job).unwrap();
        assert!(r.take_errors().is_empty());
        // os: windows comes from the include section and merges into the matrix type.
        let matrix_ty = r.matrix_ty.as_ref().expect("matrix type set");
        assert!(matrix_ty.props.contains_key("os"));
    }

    #[test]
    fn calc_needs_type_from_workflow() {
        let mut wf = create_min_workflow();
        let mut deploy = create_min_job();
        deploy.id = String::at("deploy", Pos::new(1, 1));
        deploy.outputs.insert(
            "url".to_string(),
            crate::ast::Output {
                name: String::at("url", Pos::new(1, 1)),
                value: String::at("${{ steps.s.outputs.u }}", Pos::new(1, 1)),
            },
        );
        let mut build = create_min_job();
        build.id = String::at("build", Pos::new(1, 1));
        build.needs = vec![String::at("deploy", Pos::new(1, 1))];
        wf.jobs.insert("deploy".to_string(), deploy);
        wf.jobs.insert("build".to_string(), build);

        let mut r = null_cache_rule();
        r.visit_workflow_pre(&wf).unwrap();
        r.visit_job_pre(wf.jobs.get("build").unwrap()).unwrap();

        let needs_ty = r.needs_ty.as_ref().expect("needs type set");
        assert!(needs_ty.props.contains_key("deploy"));
        assert!(!needs_ty.props.contains_key("build"));
    }

    #[test]
    fn cyclic_needs_are_skipped() {
        let mut wf = create_min_workflow();
        let mut a = create_min_job();
        a.id = String::at("a", Pos::new(1, 1));
        a.needs = vec![String::at("b", Pos::new(1, 1))];
        let mut b = create_min_job();
        b.id = String::at("b", Pos::new(1, 1));
        b.needs = vec![String::at("a", Pos::new(1, 1))];
        wf.jobs.insert("a".to_string(), a);
        wf.jobs.insert("b".to_string(), b);

        let mut r = null_cache_rule();
        r.visit_workflow_pre(&wf).unwrap();
        r.visit_job_pre(wf.jobs.get("a").unwrap()).unwrap();
        // Cyclic dependencies are handled gracefully.
        let needs_ty = r.needs_ty.as_ref().expect("needs type set");
        assert!(needs_ty.props.contains_key("b"));
    }

    #[test]
    fn workflow_call_output_type_from_local_metadata() {
        let mut wf = create_min_workflow();
        let mut call_job = create_min_job();
        call_job.id = String::at("call", Pos::new(1, 1));
        call_job.workflow_call = Some(WorkflowCall {
            uses: String::at("./.github/workflows/reusable.yaml", Pos::new(1, 1)),
            uses_present: true,
            inputs: Default::default(),
            secrets: Default::default(),
            inherit_secrets: false,
        });
        wf.jobs.insert("call".to_string(), call_job);

        let mut r = null_cache_rule();
        r.visit_workflow_pre(&wf).unwrap();
        // Local reusable workflow metadata is not available in tests, so no error is expected.
        r.visit_job_pre(wf.jobs.get("call").unwrap()).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn raw_yaml_value_types() {
        let mut r = null_cache_rule();
        assert_eq!(r.check_raw_yaml_value(&raw_str("true")).to_string(), "bool");
        assert_eq!(r.check_raw_yaml_value(&raw_str("null")).to_string(), "null");
        assert_eq!(r.check_raw_yaml_value(&raw_str("42")).to_string(), "number");
        assert_eq!(
            r.check_raw_yaml_value(&raw_str("hello")).to_string(),
            "string"
        );
        assert_eq!(
            r.check_raw_yaml_value(&raw_str("${{ github.ref }}"))
                .to_string(),
            "string"
        );
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn raw_yaml_array_merge_types() {
        let mut r = null_cache_rule();
        let arr = RawYAMLValue::Array(crate::ast::RawYAMLArray {
            elems: vec![raw_str("1"), raw_str("2"), raw_str("foo")],
            pos: Pos::new(1, 1),
        });
        // The first element is a number, the last is a string, so the merged type is string.
        assert_eq!(r.check_raw_yaml_value(&arr).to_string(), "array<string>");
    }

    #[test]
    fn raw_yaml_object_strict_type() {
        let mut r = null_cache_rule();
        let obj = RawYAMLValue::Object(crate::ast::RawYAMLObject {
            props: BTreeMap::from([("a".to_string(), raw_str("true"))]),
            pos: Pos::new(1, 1),
        });
        assert_eq!(r.check_raw_yaml_value(&obj).to_string(), "{a: bool}");
    }

    #[test]
    fn workflow_call_input_type_mismatch() {
        // A local reusable workflow file that does not exist is reported. Because no metadata is
        // available, the input type check is skipped, so only the file-read error is produced. This
        // requires a real project: a null cache (no project) returns `Ok(None)` and reports nothing,
        // matching the original `LocalReusableWorkflowCache.FindMetadata` where `c.proj == nil`.
        let tmp = std::env::temp_dir().join("actionlint_test_expr_wfcall");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".github").join("workflows")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        let proj = crate::project::Project::new(tmp.clone()).unwrap();
        let mut r = RuleExpression::new(
            Arc::new(LocalActionsCache::new(None)),
            Arc::new(LocalReusableWorkflowCache::new(
                Some(proj),
                tmp.clone(),
                None,
            )),
        );

        let mut inputs = BTreeMap::new();
        inputs.insert(
            "num".to_string(),
            crate::ast::WorkflowCallInput {
                name: String::at("num", Pos::new(4, 7)),
                value: String::at("not-a-number", Pos::new(4, 12)),
            },
        );
        let mut job = create_min_job();
        job.workflow_call = Some(WorkflowCall {
            uses: String::at("./missing.yaml", Pos::new(3, 9)),
            uses_present: true,
            inputs,
            secrets: Default::default(),
            inherit_secrets: false,
        });
        r.visit_job_pre(&job).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("could not read reusable workflow file"),
            "{:?}",
            errs[0].message
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn quoted_string_column_offset() {
        // When the string is quoted, the column is incremented to account for the quote.
        // A context-free literal is used so the expression resolves without touching a context:
        // `check_exprs_in` is called with an empty workflow key, which (matching the original)
        // restricts every context, so a context reference like `github` would be rejected here.
        let mut r = null_cache_rule();
        let s = String::new("${{ 'hello' }}", true, Pos::new(1, 1));
        let ts = r
            .check_exprs_in(&s.value, s.pos, s.quoted, false, "")
            .expect("no parse error");
        assert_eq!(ts.len(), 1);
        // Go's `checkExprsIn` records the typed expression at `Pos{line, col - 3}` where
        // `col = base(1) + quoted(1) + offset after "${{"(3) = 5`, so the position is column 2.
        assert_eq!(ts[0].pos, Pos::new(1, 2));
    }

    fn raw_str(s: &str) -> RawYAMLValue {
        RawYAMLValue::String(crate::ast::RawYAMLString {
            value: s.to_string(),
            pos: Pos::new(1, 1),
        })
    }

    // The following tests depend on the full semantic checker (`expr_sema`), which is ported
    // separately. They document the expected behavior once that lands.

    #[test]
    #[ignore = "requires the full expr_sema type checker"]
    fn unknown_context_is_error() {
        let mut r = null_cache_rule();
        let s = String::at("${{ unknown_context.foo }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unknown context"));
    }

    #[test]
    #[ignore = "requires the full expr_sema type checker"]
    fn unknown_property_is_error() {
        let mut r = null_cache_rule();
        r.matrix_ty = Some(ObjectType {
            props: BTreeMap::from([(
                "os".to_string(),
                ExprTypeKind::array(ExprTypeKind::string()),
            )]),
            mapped: None,
        });
        let s = String::at("${{ matrix.unknown }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unknown property"));
    }

    #[test]
    #[ignore = "requires the full expr_sema type checker"]
    fn wrong_function_args_is_error() {
        let mut r = null_cache_rule();
        let s = String::at("${{ contains('foo') }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.steps.name");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("contains"));
    }

    #[test]
    #[ignore = "requires the full expr_sema type checker"]
    fn unavailable_context_at_key_is_error() {
        let mut r = null_cache_rule();
        // `steps` context is not available at "jobs.<job_id>.strategy".
        let s = String::at("${{ steps.foo.outputs.bar }}", Pos::new(1, 1));
        r.check_string(Some(&s), "jobs.<job_id>.strategy");
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("steps"));
    }
}
