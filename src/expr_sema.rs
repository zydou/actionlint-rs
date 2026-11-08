//! Semantic checker for GitHub Actions expressions.
//!
//! Rust counterpart of the original `expr_sema.go`. It computes the static type of a value produced
//! by an expression and reports type errors. See
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use crate::availability::special_function_names;
use crate::expr::ExprError;
use crate::expr_ast::{ExprNode, ExprNodeKind, FuncCallNode, ObjectDerefNode, VariableNode};
use crate::expr_insecure::{UntrustedInputChecker, builtin_untrusted_inputs};
use crate::expr_lexer::Token;
use crate::expr_type::{ArrayType, ExprType, ExprTypeKind, ExprTypeKindInner, ObjectType};
use crate::quotes::{quotes, sorted_quotes};

/// Formats an ordinal like `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
#[must_use]
pub fn ordinal(i: usize) -> String {
    let suffix = match i % 10 {
        1 if i % 100 != 11 => "st",
        2 if i % 100 != 12 => "nd",
        3 if i % 100 != 13 => "rd",
        _ => "th",
    };
    format!("{i}{suffix}")
}

/// Parses the format string passed to `format()` calls and returns the set of placeholder indices it
/// contains.
///
/// See
/// <https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/evaluate-expressions-in-workflows-and-actions#format>.
#[must_use]
pub fn parse_format_func_specifiers(f: &str, n: usize) -> BTreeSet<usize> {
    const NONE: usize = usize::MAX;
    let mut ret: BTreeSet<usize> = BTreeSet::new();

    let mut start: usize = NONE;
    let mut end: usize = NONE;

    for (i, r) in f.char_indices() {
        // Before specifier
        if start == NONE {
            if r == '{' {
                // Opening brace
                start = i + 1;
            }
            continue;
        }

        // Inside specifier
        if end == NONE {
            if r == '{' && i == start {
                // Escaped '{'
                start = NONE;
            } else if r == '}' {
                if i == start {
                    // Empty specifier '{}'
                    start = NONE;
                } else {
                    // Closing brace.
                    end = i;
                }
            } else if !r.is_ascii_digit() {
                if r == '{' {
                    start = i + 1;
                } else {
                    start = NONE;
                }
            }
            continue;
        }

        // After specifier
        if r == '}' {
            // Parsing specifier needs to be delayed while '}' continues.
            continue;
        }
        if (i - end) % 2 == 1 {
            // Odd number of closing braces means the specifier closed. For example '{0}}}' contains
            // a specifier but '{0}}}}' doesn't.
            let v = f[start..end].parse::<usize>().unwrap_or(0);
            ret.insert(v);
        }
        if r == '{' {
            start = i + 1;
        } else {
            start = NONE;
        }
        end = NONE;
    }

    // When the input ends while '}' continues at end of specifier
    if start != NONE && end != NONE && (f.len() - end) % 2 == 1 {
        let v = f[start..end].parse::<usize>().unwrap_or(0);
        ret.insert(v);
    }

    let _ = n;
    ret
}

/// The signature of a builtin function.
#[derive(Debug, Clone)]
pub struct FuncSignature {
    /// The types of the function parameters. The last element may be repeated as a variable length
    /// argument when [`FuncSignature::variable_length_params`] is set.
    pub params: Vec<ExprTypeKind>,
    /// The return type of the function.
    pub ret: ExprTypeKind,
    /// When set, the last parameter may be specified multiple times (including zero times).
    pub variable_length_params: bool,
    /// Whether the function returns a constant when all of its parameters are constants.
    pub is_const_func: bool,
}

impl FuncSignature {
    /// Formats this signature like the Go original's `FuncSignature#String`. For a variadic function
    /// an ellipsis follows the final (repeated) parameter.
    #[must_use]
    pub fn display(&self, name: &str) -> String {
        let ts: Vec<String> = self.params.iter().map(ExprTypeKind::to_string).collect();
        let joined = ts.join(", ");
        let elip = if self.variable_length_params {
            "..."
        } else {
            ""
        };
        format!("{name}({joined}{elip}) -> {ret}", ret = self.ret)
    }
}

/// Builds a non-variadic, constant-returning signature.
fn sig2(ret: ExprTypeKind, params: Vec<ExprTypeKind>) -> FuncSignature {
    FuncSignature {
        params,
        ret,
        variable_length_params: false,
        is_const_func: true,
    }
}

fn string_ty() -> ExprTypeKind {
    ExprTypeKind::string()
}

fn array_of(elem: ExprTypeKind) -> ExprTypeKind {
    ExprTypeKind::new(ExprTypeKindInner::Array(ArrayType { elem, deref: false }))
}

/// Formats a single string value Go-style (`%q`): a double-quoted, escaped string.
fn q(s: impl AsRef<str>) -> String {
    quotes(&[s])
}

/// Formats a type Go-style (`%q` on its string form): a double-quoted type name.
fn q_ty(t: &ExprTypeKind) -> String {
    q(t.to_string())
}

/// Returns the table of all builtin function signatures, keyed by lower-cased function name.
///
/// All function names are in lower case because function names are compared in case insensitive.
/// See <https://docs.github.com/en/actions/learn-github-actions/expressions#functions>.
#[must_use]
pub fn builtin_func_signatures() -> &'static HashMap<&'static str, Vec<FuncSignature>> {
    static TABLE: OnceLock<HashMap<&'static str, Vec<FuncSignature>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let s = || ExprTypeKind::string();
        let b = || ExprTypeKind::bool();
        let a = || ExprTypeKind::any();
        let s_arr = || array_of(ExprTypeKind::string());
        let a_arr = || array_of(ExprTypeKind::any());
        // The final entry in `params` is the repeated (variable length) parameter, matching the Go
        // original where `VariableLengthParams` marks the last element of `Params` as repeatable.
        let variadic = |ret: ExprTypeKind, params: Vec<ExprTypeKind>| FuncSignature {
            params,
            ret,
            variable_length_params: true,
            is_const_func: true,
        };
        // `hashFiles` is variadic but its result depends on the filesystem, so it is not constant.
        let variadic_non_const = |ret: ExprTypeKind, params: Vec<ExprTypeKind>| FuncSignature {
            params,
            ret,
            variable_length_params: true,
            is_const_func: false,
        };
        let non_const = |ret: ExprTypeKind, params: Vec<ExprTypeKind>| FuncSignature {
            params,
            ret,
            variable_length_params: false,
            is_const_func: false,
        };
        let no_args = |ret: ExprTypeKind| FuncSignature {
            params: Vec::new(),
            ret,
            variable_length_params: false,
            is_const_func: false,
        };

        HashMap::from([
            (
                "contains",
                vec![sig2(b(), vec![s(), s()]), sig2(b(), vec![a_arr(), a()])],
            ),
            ("startswith", vec![sig2(b(), vec![s(), s()])]),
            ("endswith", vec![sig2(b(), vec![s(), s()])]),
            ("format", vec![variadic(s(), vec![s()])]),
            (
                "join",
                vec![
                    sig2(s(), vec![s_arr(), s()]),
                    // When the second parameter is omitted, values are concatenated with ','.
                    sig2(s(), vec![s_arr()]),
                ],
            ),
            ("tojson", vec![sig2(s(), vec![a()])]),
            ("fromjson", vec![non_const(a(), vec![s()])]),
            ("hashfiles", vec![variadic_non_const(s(), vec![s()])]),
            ("success", vec![no_args(b())]),
            ("always", vec![no_args(b())]),
            ("cancelled", vec![no_args(b())]),
            ("failure", vec![no_args(b())]),
            ("case", vec![variadic(a(), vec![b(), a(), a()])]),
        ])
    })
}

/// Builds the type of the `github` context.
fn github_context() -> ExprTypeKind {
    let mut props: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    let s = string_ty();
    for n in [
        "action",
        "action_path", // Note: Composite actions only
        "action_ref",
        "action_repository",
        "action_status", // Note: Composite actions only
        "actor",
        "actor_id",
        "api_url",
        "base_ref",
        "env",
        "event_name",
        "event_path",
        "graphql_url",
        "head_ref",
        "job",
        "output", // Note: Undocumented
        "path",
        "ref",
        "ref_name",
        "ref_type",
        "repository",
        "repository_id",
        "repository_owner",
        "repository_owner_id",
        "repository_visibility", // Note: Undocumented
        "repositoryurl",         // repositoryUrl
        "run_attempt",
        "run_id",
        "run_number",
        "secret_source",
        "server_url",
        "sha",
        "state",        // Note: Undocumented
        "step_summary", // Note: Undocumented
        "token",
        "triggering_actor",
        "workflow",
        "workflow_ref",
        "workflow_sha",
        "workspace",
    ] {
        props.insert(n.to_string(), s.clone());
    }
    // Note: Undocumented
    props.insert(
        "artifact_cache_size_limit".to_string(),
        ExprTypeKind::number(),
    );
    props.insert("ref_protected".to_string(), ExprTypeKind::bool());
    props.insert("retention_days".to_string(), ExprTypeKind::number());
    // Note: Stricter type check for this payload would be possible
    props.insert("event".to_string(), ExprTypeKind::empty_object());
    ExprTypeKind::strict_object(props)
}

/// Builds the type of the `job` context.
fn job_context() -> ExprTypeKind {
    let s = string_ty();
    let n = ExprTypeKind::number();

    let mut container: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    container.insert("id".to_string(), s.clone());
    container.insert("network".to_string(), s.clone());

    let mut service: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    service.insert("id".to_string(), s.clone()); // job.services.<service id>.id
    service.insert("network".to_string(), s.clone());
    service.insert("ports".to_string(), ExprTypeKind::map_object(s.clone()));

    let mut services: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    services.insert("*".to_string(), ExprTypeKind::strict_object(service));

    let mut props: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    props.insert("check_run_id".to_string(), n);
    props.insert(
        "container".to_string(),
        ExprTypeKind::strict_object(container),
    );
    props.insert(
        "services".to_string(),
        ExprTypeKind::strict_object(services),
    );
    props.insert("status".to_string(), s);
    ExprTypeKind::strict_object(props)
}

/// Builds the type of the `runner` context.
fn runner_context() -> ExprTypeKind {
    let mut props: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    let s = string_ty();
    for n in [
        "name",
        "os",
        "arch",
        "temp",
        "tool_cache",
        "debug",
        "environment", // https://github.com/github/docs/issues/32443
    ] {
        props.insert(n.to_string(), s.clone());
    }
    ExprTypeKind::strict_object(props)
}

/// Builds the type of the `strategy` context.
fn strategy_context() -> ExprTypeKind {
    let mut props: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    props.insert("fail-fast".to_string(), ExprTypeKind::bool());
    props.insert("job-index".to_string(), ExprTypeKind::number());
    props.insert("job-total".to_string(), ExprTypeKind::number());
    props.insert("max-parallel".to_string(), ExprTypeKind::number());
    ExprTypeKind::object(props)
}

/// Builds the default types of all global variables (contexts).
///
/// Counterpart of the Go `BuiltinGlobalVariableTypes`. All context variables are documented at
/// <https://docs.github.com/en/actions/learn-github-actions/contexts>.
#[must_use]
pub fn builtin_global_variable_types() -> BTreeMap<String, ExprTypeKind> {
    let mut m: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
    m.insert("github".to_string(), github_context());
    m.insert("env".to_string(), ExprTypeKind::map_object(string_ty())); // env.<env_name>
    m.insert("job".to_string(), job_context());
    m.insert("steps".to_string(), ExprTypeKind::empty_strict_object()); // Updated contextually
    m.insert("runner".to_string(), runner_context());
    m.insert("secrets".to_string(), ExprTypeKind::map_object(string_ty()));
    m.insert("strategy".to_string(), strategy_context());
    m.insert("matrix".to_string(), ExprTypeKind::empty_strict_object()); // Updated contextually
    m.insert("needs".to_string(), ExprTypeKind::empty_strict_object()); // Updated contextually
    m.insert("inputs".to_string(), ExprTypeKind::empty_strict_object());
    m.insert("vars".to_string(), ExprTypeKind::map_object(string_ty())); // vars.<var_name>
    m
}

/// A semantic checker for expressions.
#[derive(Clone)]
pub struct ExprSemanticsChecker {
    /// Whether to check for untrusted inputs.
    pub check_untrusted_input: bool,
    /// The config variables defined in the repository config, if any.
    pub config_vars: Option<Vec<String>>,
    /// The types of the `matrix` context.
    pub matrix: ObjectType,
    /// Whether the `matrix` context was explicitly set (a `strategy.matrix` exists). This
    /// distinguishes "no matrix" (keep the strict builtin) from "matrix set to an empty loose
    /// object" (e.g. `matrix: ${{ fromJSON(...) }}`, which must stay loose), since both are
    /// indistinguishable via [`Self::is_set`]. Counterpart of Go's nil-able `matrixTy`.
    matrix_set: bool,
    /// The types of the `steps` context.
    pub steps: ObjectType,
    /// The types of the `needs` context.
    pub needs: ObjectType,
    /// The types of the `secrets` context.
    pub secrets: ObjectType,
    /// The types of the `inputs` context.
    pub inputs: ObjectType,
    /// The types of the `workflow_dispatch` inputs, used to type `github.event.inputs` (whose
    /// values are always strings, unlike `inputs.*`). `None` when `workflow_dispatch` is not a
    /// trigger.
    dispatch_inputs: Option<ObjectType>,
    /// The types of the `jobs` context.
    pub jobs: ObjectType,
    /// The contexts available at the current workflow key.
    pub available_contexts: Vec<String>,
    /// Whether [`Self::available_contexts`] was explicitly set. When `false`, any context is allowed
    /// (mirroring the Go original's `nil` slice).
    pub contexts_restricted: bool,
    /// The special functions available at the current workflow key.
    pub available_special_funcs: Vec<String>,
}

impl ExprSemanticsChecker {
    /// Creates a new checker. `check_untrusted_input` enables untrusted input detection. `config_vars`
    /// is the list of config variables defined in the repository config, if any.
    #[must_use]
    pub fn new(check_untrusted_input: bool, config_vars: Option<Vec<String>>) -> Self {
        Self {
            check_untrusted_input,
            config_vars,
            matrix: ObjectType::default(),
            matrix_set: false,
            steps: ObjectType::default(),
            needs: ObjectType::default(),
            secrets: ObjectType::default(),
            // The `inputs` context defaults to a *strict* empty object, matching the builtin
            // `inputs` type (`NewEmptyStrictObjectType` in the original). This matters because
            // `update_dispatch_inputs` replaces (rather than merges into) the inputs when they are
            // still unset, and a loose default would wrongly merge and stay loose.
            inputs: ObjectType {
                props: BTreeMap::new(),
                mapped: None,
            },
            dispatch_inputs: None,
            jobs: ObjectType::default(),
            available_contexts: Vec::new(),
            contexts_restricted: false,
            available_special_funcs: Vec::new(),
        }
    }

    /// Restricts the available context names while semantics checks. Some contexts limit where they
    /// can be used.
    ///
    /// Elements of `avail` must be in lower case to check context names in case-insensitive.
    ///
    /// If this method is not called before checks, the checker considers any contexts are available
    /// by default.
    pub fn set_context_availability(&mut self, avail: Vec<String>) {
        self.available_contexts = avail;
        self.contexts_restricted = true;
    }

    /// Updates the `inputs` context and `github.event.inputs` to the `workflow_dispatch` inputs
    /// type. Counterpart of `UpdateDispatchInputs`.
    ///
    /// Unlike `inputs.*`, the type of `github.event.inputs.*` is always `string`, so a new
    /// all-`string` object is built from `ty` (e.g. `{foo: boolean}` -> `{foo: string}`).
    /// Sets the `matrix` context type and marks it as explicitly provided. Counterpart of Go's
    /// `UpdateMatrix`. Unlike the [`Self::is_set`] heuristic, this records that a `strategy.matrix`
    /// exists even when it resolves to an empty loose object (e.g. `matrix: ${{ fromJSON(...) }}`).
    pub fn update_matrix(&mut self, ty: ObjectType) {
        self.matrix = ty;
        self.matrix_set = true;
    }

    pub fn update_dispatch_inputs(&mut self, ty: ObjectType) {
        // When both `workflow_call` and `workflow_dispatch` trigger the workflow, the `inputs`
        // context is usable by both events, so merge them into one object type (#263).
        if self.inputs.props.is_empty() && self.inputs.is_strict() {
            self.inputs = ty.clone();
        } else {
            let merged = ExprTypeKind::new(ExprTypeKindInner::Object(self.inputs.clone()))
                .merge(&ExprTypeKind::new(ExprTypeKindInner::Object(ty.clone())));
            if let ExprTypeKindInner::Object(o) = merged.inner() {
                self.inputs = o.clone();
            }
        }
        self.dispatch_inputs = Some(ty);
    }

    /// Sets names of available special functions while semantics checks. Some functions limit where
    /// they can be used.
    ///
    /// Elements of `avail` must be in lower case to check function names in case-insensitive.
    pub fn set_special_function_availability(&mut self, avail: Vec<String>) {
        self.available_special_funcs = avail;
    }

    /// Builds the map from context name to its type for this checker, applying the contextual
    /// overrides configured on the struct.
    fn vars(&self) -> BTreeMap<String, ExprTypeKind> {
        let mut vars = builtin_global_variable_types();

        // `matrix`, `steps`, `needs`, and `jobs` are updated contextually. `matrix` uses an explicit
        // presence flag because a `strategy.matrix` that resolves to an empty loose object must
        // stay loose rather than falling back to the strict builtin. The others keep the `is_set`
        // heuristic: an unset override carries no props and is loose, so the default (empty strict
        // object) value is kept.
        if self.matrix_set {
            vars.insert("matrix".to_string(), Self::object_var(&self.matrix));
        }
        if Self::is_set(&self.steps) {
            vars.insert("steps".to_string(), Self::object_var(&self.steps));
        }
        if Self::is_set(&self.needs) {
            vars.insert("needs".to_string(), Self::object_var(&self.needs));
        }
        if Self::is_set(&self.jobs) {
            vars.insert("jobs".to_string(), Self::object_var(&self.jobs));
        }

        // Merge automatically supplied secrets with manually defined secrets.
        // ACTIONS_STEP_DEBUG and ACTIONS_RUNNER_DEBUG seem supplied from caller of the workflow.
        if Self::is_set(&self.secrets) {
            let mut merged: BTreeMap<String, ExprTypeKind> = BTreeMap::new();
            merged.insert("github_token".to_string(), ExprTypeKind::string());
            merged.insert("actions_step_debug".to_string(), ExprTypeKind::string());
            merged.insert("actions_runner_debug".to_string(), ExprTypeKind::string());
            for (n, v) in &self.secrets.props {
                merged.insert(n.clone(), v.clone());
            }
            vars.insert("secrets".to_string(), ExprTypeKind::strict_object(merged));
        }

        // Merge `inputs` contexts to cover both `workflow_call` and `workflow_dispatch` triggers.
        if Self::is_set(&self.inputs) {
            let default = ExprTypeKind::empty_strict_object();
            let override_ty = ExprTypeKind::new(ExprTypeKindInner::Object(self.inputs.clone()));
            // When the default `inputs` context is empty and strict, replace it. Otherwise merge so
            // that both `workflow_call` and `workflow_dispatch` inputs are covered (#263).
            let merged = if default
                .as_object()
                .is_some_and(|o| o.props.is_empty() && o.is_strict())
            {
                override_ty
            } else {
                default.merge(&override_ty)
            };
            vars.insert("inputs".to_string(), merged);
        }

        // Override `github.event.inputs` with an all-`string` object built from the
        // `workflow_dispatch` inputs, mirroring `UpdateDispatchInputs`.
        if let Some(disp) = &self.dispatch_inputs
            && let Some(gh) = vars.get_mut("github")
            && let ExprTypeKindInner::Object(gh_obj) = gh.inner_mut()
            && let Some(ev) = gh_obj.props.get_mut("event")
            && let ExprTypeKindInner::Object(ev_obj) = ev.inner_mut()
        {
            let p: BTreeMap<String, ExprTypeKind> = disp
                .props
                .keys()
                .map(|n| (n.clone(), ExprTypeKind::string()))
                .collect();
            ev_obj
                .props
                .insert("inputs".to_string(), ExprTypeKind::strict_object(p));
        }

        vars
    }

    /// Returns whether a contextual override object carries any information.
    fn is_set(o: &ObjectType) -> bool {
        !o.props.is_empty() || o.is_strict()
    }

    /// Wraps an override object into an `ExprTypeKind`, falling back to the default empty object
    /// when the override carries no information.
    fn object_var(o: &ObjectType) -> ExprTypeKind {
        if o.props.is_empty() && o.mapped.is_none() {
            ExprTypeKind::empty_strict_object()
        } else {
            ExprTypeKind::new(ExprTypeKindInner::Object(o.clone()))
        }
    }

    fn check_available_context(&self, n: &VariableNode, errs: &mut Vec<ExprError>) {
        if !self.contexts_restricted {
            return; // Any context is available by default
        }
        let ctx = n.name.to_lowercase();
        if self.available_contexts.iter().any(|c| c == &ctx) {
            return;
        }

        let notes = match self.available_contexts.len() {
            0 => "no context is available here".to_string(),
            1 => format!("available context is {}", quotes(&self.available_contexts)),
            _ => format!(
                "available contexts are {}",
                quotes(&self.available_contexts)
            ),
        };
        let msg = format!(
            "context {} is not allowed here. {notes}. see https://docs.github.com/en/actions/learn-github-actions/contexts#context-availability for more details",
            q(&n.name)
        );
        errs.push(error_at_expr(&n.tok, msg));
    }

    fn check_special_function_availability(&self, n: &FuncCallNode, errs: &mut Vec<ExprError>) {
        let f = n.callee.to_lowercase();

        let Some(allowed) = special_function_names().get(f.as_str()) else {
            return; // This function is not special
        };

        if self.available_special_funcs.iter().any(|sp| sp == &f) {
            return;
        }

        let msg = format!(
            "calling function {} is not allowed here. {} is only available in {}. see https://docs.github.com/en/actions/learn-github-actions/contexts#context-availability for more details",
            q(&n.callee),
            q(&n.callee),
            quotes(allowed)
        );
        errs.push(error_at_expr(&n.tok, msg));
    }

    fn check_variable(
        &self,
        n: &VariableNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
    ) -> ExprTypeKind {
        let Some(v) = vars.get(&n.name) else {
            let ss: Vec<String> = vars.keys().cloned().collect();
            let msg = format!(
                "undefined variable {}. available variables are {}",
                q(&n.tok.value),
                sorted_quotes(ss)
            );
            errs.push(error_at_expr(&n.tok, msg));
            return ExprTypeKind::any();
        };

        self.check_available_context(n, errs);
        v.clone()
    }

    fn check_config_variables(&self, n: &ObjectDerefNode, tok: &Token, errs: &mut Vec<ExprError>) {
        let prop = &n.property;
        // https://docs.github.com/en/actions/learn-github-actions/variables#naming-conventions-for-configuration-variables
        if prop.starts_with("github_") {
            let msg = format!(
                "configuration variable name {} must not start with the GITHUB_ prefix (case insensitive). note: see the convention at https://docs.github.com/en/actions/learn-github-actions/variables#naming-conventions-for-configuration-variables",
                q(prop)
            );
            errs.push(error_at_expr(tok, msg));
            return;
        }
        for r in prop.chars() {
            // Note: `prop` was already converted to lower case by parser
            // Note: First character cannot be number, but it was already checked by parser
            if r.is_ascii_digit() || r.is_ascii_lowercase() || r == '_' {
                continue;
            }
            let msg = format!(
                "configuration variable name {} can only contain alphabets, decimal numbers, and '_'. note: see the convention at https://docs.github.com/en/actions/learn-github-actions/variables#naming-conventions-for-configuration-variables",
                q(prop)
            );
            errs.push(error_at_expr(tok, msg));
            return;
        }

        let Some(config_vars) = &self.config_vars else {
            return;
        };
        if config_vars.is_empty() {
            let msg = format!(
                "no configuration variable is allowed since the variables list is empty in actionlint.yaml. you may forget adding the variable {} to the list",
                q(prop)
            );
            errs.push(error_at_expr(tok, msg));
            return;
        }

        if config_vars.iter().any(|v| v.eq_ignore_ascii_case(prop)) {
            return;
        }

        let msg = format!(
            "undefined configuration variable {}. defined configuration variables in actionlint.yaml are {}",
            q(prop),
            sorted_quotes(config_vars.clone())
        );
        errs.push(error_at_expr(tok, msg));
    }

    fn check_object_deref(
        &self,
        n: &ObjectDerefNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        // A well-formed deref always has a receiver; treat a missing one as `any`.
        let Some(receiver) = &n.receiver else {
            return ExprTypeKind::any();
        };
        let tok = receiver.token().cloned().unwrap_or_else(empty_token);
        let ty = self.check_inner(receiver, vars, errs, untrusted);

        match ty.inner() {
            ExprTypeKindInner::Any => ExprTypeKind::any(),
            ExprTypeKindInner::Object(o) => {
                if let Some(t) = o.props.get(&n.property) {
                    return t.clone();
                }
                if let Some(mapped) = &o.mapped {
                    if let Some(ExprNodeKind::Variable(v)) = Some(receiver.as_ref())
                        && v.name == "vars"
                    {
                        self.check_config_variables(n, &tok, errs);
                    }
                    return mapped.clone();
                }
                if o.is_strict() {
                    let msg = format!(
                        "property {} is not defined in object type {}",
                        q(&n.property),
                        o
                    );
                    errs.push(error_at_expr(&tok, msg));
                }
                ExprTypeKind::any()
            }
            ExprTypeKindInner::Array(a) => {
                if !a.deref {
                    let msg = format!(
                        "receiver of object dereference {} must be type of object but got {}",
                        q(&n.property),
                        q_ty(&ty)
                    );
                    errs.push(error_at_expr(&tok, msg));
                    return ExprTypeKind::any();
                }
                match a.elem.inner() {
                    ExprTypeKindInner::Any => {
                        // When element type is any, map the any type to any. Reuse `ty`.
                        ty
                    }
                    ExprTypeKindInner::Object(et) => {
                        // Map element type of dereferenced array
                        let elem = if let Some(t) = et.props.get(&n.property) {
                            t.clone()
                        } else if let Some(mapped) = &et.mapped {
                            mapped.clone()
                        } else if et.is_strict() {
                            let msg = format!(
                                "property {} is not defined in object type {} as element of filtered array",
                                q(&n.property),
                                et
                            );
                            errs.push(error_at_expr(&tok, msg));
                            ExprTypeKind::any()
                        } else {
                            ExprTypeKind::any()
                        };
                        ExprTypeKind::new(ExprTypeKindInner::Array(ArrayType { elem, deref: true }))
                    }
                    _ => {
                        let msg = format!(
                            "property filtered by {} at object filtering must be type of object but got {}",
                            q(&n.property),
                            q_ty(&a.elem)
                        );
                        errs.push(error_at_expr(&tok, msg));
                        ExprTypeKind::any()
                    }
                }
            }
            _ => {
                let msg = format!(
                    "receiver of object dereference {} must be type of object but got {}",
                    q(&n.property),
                    q_ty(&ty)
                );
                errs.push(error_at_expr(&tok, msg));
                ExprTypeKind::any()
            }
        }
    }

    fn check_array_deref(
        &self,
        receiver: Option<&ExprNode>,
        errs: &mut Vec<ExprError>,
        vars: &BTreeMap<String, ExprTypeKind>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        let Some(receiver) = receiver else {
            return ExprTypeKind::any();
        };
        let tok = receiver.token().cloned().unwrap_or_else(empty_token);
        let ty = self.check_inner(receiver, vars, errs, untrusted);

        match ty.inner() {
            ExprTypeKindInner::Any => ExprTypeKind::deref_array(ExprTypeKind::any()),
            ExprTypeKindInner::Array(a) => {
                let mut copied = a.clone();
                copied.deref = true;
                ExprTypeKind::new(ExprTypeKindInner::Array(copied))
            }
            ExprTypeKindInner::Object(o) => {
                // Object filtering is available for objects, not only arrays (#66)
                if let Some(mapped) = &o.mapped {
                    // For map object or loose object at receiver of .*
                    return match mapped.inner() {
                        ExprTypeKindInner::Any => ExprTypeKind::deref_array(ExprTypeKind::any()),
                        ExprTypeKindInner::Object(mty) => {
                            ExprTypeKind::new(ExprTypeKindInner::Array(ArrayType {
                                elem: ExprTypeKind::new(ExprTypeKindInner::Object(mty.clone())),
                                deref: true,
                            }))
                        }
                        _ => {
                            let msg = format!(
                                "elements of object at receiver of object filtering `.*` must be type of object but got {}. the type of receiver was {}",
                                q_ty(mapped),
                                q_ty(&ty)
                            );
                            errs.push(error_at_expr(&tok, msg));
                            ExprTypeKind::any()
                        }
                    };
                }

                // For strict object at receiver of .*
                let found = o
                    .props
                    .values()
                    .any(|t| matches!(t.inner(), ExprTypeKindInner::Object(_)));
                if !found {
                    let msg = format!(
                        "object type {} cannot be filtered by object filtering `.*` since it has no object element",
                        q_ty(&ty)
                    );
                    errs.push(error_at_expr(&tok, msg));
                    return ExprTypeKind::any();
                }

                ExprTypeKind::deref_array(ExprTypeKind::any())
            }
            _ => {
                let msg = format!(
                    "receiver of object filtering `.*` must be type of array or object but got {}",
                    q_ty(&ty)
                );
                errs.push(error_at_expr(&tok, msg));
                ExprTypeKind::any()
            }
        }
    }

    fn check_index_access(
        &self,
        operand: Option<&ExprNode>,
        index: Option<&ExprNode>,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        let (Some(operand), Some(index)) = (operand, index) else {
            return ExprTypeKind::any();
        };
        // Note: Index must be visited before Operand to make UntrustedInputChecker work correctly
        // even if the expression has some nest like foo[aaa.bbb].bar.
        let idx = self.check_inner(index, vars, errs, untrusted);
        let ty = self.check_inner(operand, vars, errs, untrusted);

        match ty.inner() {
            ExprTypeKindInner::Any => ExprTypeKind::any(),
            ExprTypeKindInner::Array(a) => match idx.inner() {
                ExprTypeKindInner::Any | ExprTypeKindInner::Number => a.elem.clone(),
                _ => {
                    let msg = format!(
                        "index access of array must be type of number but got {}",
                        q_ty(&idx)
                    );
                    errs.push(error_at_expr(&index_token(index), msg));
                    ExprTypeKind::any()
                }
            },
            ExprTypeKindInner::Object(o) => match idx.inner() {
                ExprTypeKindInner::Any => ExprTypeKind::any(),
                ExprTypeKindInner::String => {
                    // Index access with string literal like foo['bar']
                    if let Some(ExprNodeKind::String(lit)) = Some(index.as_ref()) {
                        if let Some(prop) = o.props.get(&lit.value) {
                            return prop.clone();
                        }
                        if let Some(mapped) = &o.mapped {
                            return mapped.clone();
                        }
                        if o.is_strict() {
                            let msg = format!(
                                "property {} is not defined in object type {}",
                                q(&lit.value),
                                o
                            );
                            errs.push(error_at_expr(&operand_token(operand), msg));
                        }
                    }
                    if let Some(mapped) = &o.mapped {
                        return mapped.clone();
                    }
                    ExprTypeKind::any() // Fallback
                }
                _ => {
                    let msg = format!(
                        "property access of object must be type of string but got {}",
                        q_ty(&idx)
                    );
                    errs.push(error_at_expr(&index_token(index), msg));
                    ExprTypeKind::any()
                }
            },
            _ => {
                let msg = format!(
                    "index access operand must be type of object or array but got {}",
                    q_ty(&ty)
                );
                errs.push(error_at_expr(&operand_token(operand), msg));
                ExprTypeKind::any()
            }
        }
    }

    fn check_builtin_func_call(
        &self,
        n: &FuncCallNode,
        sig: &FuncSignature,
        errs: &mut Vec<ExprError>,
    ) -> ExprTypeKind {
        self.check_special_function_availability(n, errs);

        // Special checks for specific built-in functions
        match n.callee.to_lowercase().as_str() {
            "format" => {
                let Some(ExprNodeKind::String(lit)) =
                    n.args.first().map(std::convert::AsRef::as_ref)
                else {
                    return sig.ret.clone();
                };
                let l = n.args.len() - 1; // -1 means removing first format string argument

                let mut holders = parse_format_func_specifiers(&lit.value, l);

                for i in 0..l {
                    if !holders.remove(&i) {
                        let msg = format!(
                            "format string {} does not contain placeholder {{{i}}}. remove argument which is unused in the format string",
                            q(&lit.value)
                        );
                        errs.push(error_at_expr(&n.tok, msg));
                    }
                }

                for i in &holders {
                    let msg = format!(
                        "format string {} contains placeholder {{{i}}} but only {l} arguments are given to format",
                        q(&lit.value)
                    );
                    errs.push(error_at_expr(&n.tok, msg));
                }
            }
            "fromjson" => {
                let Some(ExprNodeKind::String(lit)) =
                    n.args.first().map(std::convert::AsRef::as_ref)
                else {
                    return sig.ret.clone();
                };
                match serde_json::from_str::<serde_json::Value>(&lit.value) {
                    Ok(v) => return crate::expr_type::type_of_json_value(&v),
                    // Go's json.Unmarshal reports a `*json.SyntaxError` carrying a byte offset and
                    // a message for both malformed syntax and premature end-of-input. serde_json
                    // classifies premature end-of-input as `Category::Eof` rather than `Syntax`,
                    // so both are accepted here to match Go's behavior.
                    Err(e) if e.is_syntax() || e.is_eof() => {
                        // Premature end-of-input: Go emits the fixed message
                        // "unexpected end of JSON input" at the end-of-input byte offset. Other
                        // syntax errors report the byte offset of the offending character.
                        let (offset, msg) = if e.is_eof() {
                            (lit.value.len(), "unexpected end of JSON input".to_owned())
                        } else {
                            (
                                byte_offset_at(&lit.value, e.line(), e.column()),
                                strip_json_error_position(&e.to_string()),
                            )
                        };
                        let msg = format!(
                            "broken JSON string is passed to fromJSON() at offset {offset}: {msg}"
                        );
                        errs.push(error_at_expr(&lit.tok, msg));
                    }
                    Err(_) => {}
                }
            }
            "case" if n.args.len().is_multiple_of(2) => {
                let msg = format!(
                    "case() requires an odd number of arguments (pred/value pairs + default) but got {}",
                    n.args.len()
                );
                errs.push(error_at_expr(&n.tok, msg));
            }
            _ => {}
        }

        sig.ret.clone()
    }

    fn check_func_call(
        &self,
        n: &FuncCallNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        // Check function name in case insensitive. For example, toJson and toJSON are the same.
        let callee = n.callee.to_lowercase();
        let Some(sigs) = builtin_func_signatures().get(callee.as_str()) else {
            let ss: Vec<String> = builtin_func_signatures()
                .keys()
                .map(std::string::ToString::to_string)
                .collect();
            let msg = format!(
                "undefined function {}. available functions are {}",
                q(&n.callee),
                sorted_quotes(ss)
            );
            errs.push(error_at_expr(&n.tok, msg));
            return ExprTypeKind::any();
        };

        let mut tys = Vec::with_capacity(n.args.len());
        for a in &n.args {
            tys.push(self.check_inner(a, vars, errs, untrusted));
        }

        // Check all overloads
        let mut sig_errs: Vec<ExprError> = Vec::new();
        for sig in sigs {
            match check_func_signature(n, sig, &tys) {
                Ok(()) => {
                    // When one of overload passes type check, overload was resolved correctly
                    return self.check_builtin_func_call(n, sig, errs);
                }
                Err(err) => {
                    sig_errs.push(err);
                }
            }
        }

        // All candidates failed
        errs.append(&mut sig_errs);

        ExprTypeKind::any()
    }

    fn check_not_op(
        &self,
        operand: Option<&ExprNode>,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        let ty = match operand {
            Some(o) => self.check_inner(o, vars, errs, untrusted),
            None => ExprTypeKind::any(),
        };
        if !ExprTypeKind::bool().assignable(&ty) {
            let msg = format!(
                "type of operand of ! operator {} is not assignable to type \"bool\"",
                q_ty(&ty)
            );
            if let Some(tok) = operand.and_then(|o| o.token()) {
                errs.push(error_at_expr(tok, msg));
            }
        }
        ExprTypeKind::bool()
    }

    fn check_compare_op(
        &self,
        kind: crate::expr_ast::CompareOpKind,
        left: Option<&ExprNode>,
        right: Option<&ExprNode>,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        let l = match left {
            Some(x) => self.check_inner(x, vars, errs, untrusted),
            None => ExprTypeKind::any(),
        };
        let r = match right {
            Some(x) => self.check_inner(x, vars, errs, untrusted),
            None => ExprTypeKind::any(),
        };

        if !validate_compare_op_operands(kind, &l, &r) {
            let msg = format!(
                "{} value cannot be compared to {} value with {} operator",
                q_ty(&l),
                q_ty(&r),
                q(kind.to_string())
            );
            if let Some(tok) = left.and_then(|x| x.token()) {
                errs.push(error_at_expr(tok, msg));
            }
        }

        ExprTypeKind::bool()
    }

    /// Checks the type of the given expression with type narrowing. Type narrowing narrows down the
    /// type of the expression by assuming its value. For example, `l && r` is typed as
    /// `typeof(l) | typeof(r)` usually. However when the expression is assumed to be true, its type
    /// can be narrowed down to `typeof(r)`.
    ///
    /// This analysis is useful to make type checking more accurate. For example,
    /// `some_var && 60 || 20` can be typed as `number` instead of `typeof(some_var) | number`. (#384)
    fn check_with_narrowing(
        &self,
        n: &ExprNode,
        is_truthy: bool,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        match n.as_ref() {
            ExprNodeKind::LogicalOp(l) => match l.kind {
                crate::expr_ast::LogicalOpKind::And => {
                    // When `l && r` is true, narrow its type to `typeof(r)`
                    if is_truthy {
                        if let Some(left) = &l.left {
                            self.check_inner(left, vars, errs, untrusted);
                        }
                        return match &l.right {
                            Some(right) => self.check_inner(right, vars, errs, untrusted),
                            None => ExprTypeKind::any(),
                        };
                    }
                    self.check_logical_op(l, vars, errs, untrusted)
                }
                crate::expr_ast::LogicalOpKind::Or => {
                    // When `l || r` is false, narrow its type to `typeof(r)`
                    if !is_truthy {
                        if let Some(left) = &l.left {
                            self.check_inner(left, vars, errs, untrusted);
                        }
                        return match &l.right {
                            Some(right) => self.check_inner(right, vars, errs, untrusted),
                            None => ExprTypeKind::any(),
                        };
                    }
                    self.check_logical_op(l, vars, errs, untrusted)
                }
                crate::expr_ast::LogicalOpKind::Invalid => {
                    self.check_logical_op(l, vars, errs, untrusted)
                }
            },
            ExprNodeKind::NotOp(not) => match &not.operand {
                Some(o) => self.check_with_narrowing(o, !is_truthy, vars, errs, untrusted),
                None => ExprTypeKind::any(),
            },
            _ => self.check_inner(n, vars, errs, untrusted),
        }
    }

    fn check_logical_op(
        &self,
        n: &crate::expr_ast::LogicalOpNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        match n.kind {
            crate::expr_ast::LogicalOpKind::And => {
                // When `l` is false in `l && r`, its type is `typeof(l)`. Otherwise `typeof(r)`.
                // Narrow the type of LHS expression by assuming its value is falsy.
                let l = match &n.left {
                    Some(left) => self.check_with_narrowing(left, false, vars, errs, untrusted),
                    None => ExprTypeKind::any(),
                };
                let r = match &n.right {
                    Some(right) => self.check_inner(right, vars, errs, untrusted),
                    None => ExprTypeKind::any(),
                };
                l.merge(&r)
            }
            crate::expr_ast::LogicalOpKind::Or => {
                // When `l` is true in `l || r`, its type is `typeof(l)`. Otherwise `typeof(r)`.
                // Narrow the type of LHS expression by assuming its value is truthy.
                let l = match &n.left {
                    Some(left) => self.check_with_narrowing(left, true, vars, errs, untrusted),
                    None => ExprTypeKind::any(),
                };
                let r = match &n.right {
                    Some(right) => self.check_inner(right, vars, errs, untrusted),
                    None => ExprTypeKind::any(),
                };
                l.merge(&r)
            }
            crate::expr_ast::LogicalOpKind::Invalid => {
                if let Some(left) = &n.left {
                    self.check_inner(left, vars, errs, untrusted);
                }
                if let Some(right) = &n.right {
                    self.check_inner(right, vars, errs, untrusted);
                }
                ExprTypeKind::any()
            }
        }
    }

    /// Wraps [`Self::check_dispatch`] with the untrusted input checker callbacks. Calls the enter
    /// callback before visiting children and the leave callback after, mirroring the Go defer
    /// ordering. Every node must flow through this method so the whole tree is visited.
    fn check_inner(
        &self,
        expr: &ExprNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        if let Some(u) = untrusted {
            u.on_visit_node_enter(expr);
        }
        let ty = self.check_dispatch(expr, vars, errs, untrusted);
        if let Some(u) = untrusted {
            u.on_visit_node_leave(expr);
        }
        ty
    }

    /// Dispatches on the kind of the given expression node, mirroring the Go `check` method.
    fn check_dispatch(
        &self,
        expr: &ExprNode,
        vars: &BTreeMap<String, ExprTypeKind>,
        errs: &mut Vec<ExprError>,
        untrusted: &mut Option<UntrustedInputChecker>,
    ) -> ExprTypeKind {
        match expr.as_ref() {
            ExprNodeKind::Variable(v) => self.check_variable(v, vars, errs),
            ExprNodeKind::Null(_) => ExprTypeKind::null(),
            ExprNodeKind::Bool(_) => ExprTypeKind::bool(),
            ExprNodeKind::String(_) => ExprTypeKind::string(),
            ExprNodeKind::Int(_) | ExprNodeKind::Float(_) => ExprTypeKind::number(),
            ExprNodeKind::ObjectDeref(d) => self.check_object_deref(d, vars, errs, untrusted),
            ExprNodeKind::ArrayDeref(d) => {
                self.check_array_deref(d.receiver.as_ref(), errs, vars, untrusted)
            }
            ExprNodeKind::IndexAccess(i) => {
                self.check_index_access(i.operand.as_ref(), i.index.as_ref(), vars, errs, untrusted)
            }
            ExprNodeKind::FuncCall(f) => self.check_func_call(f, vars, errs, untrusted),
            ExprNodeKind::NotOp(not) => {
                self.check_not_op(not.operand.as_ref(), vars, errs, untrusted)
            }
            ExprNodeKind::CompareOp(c) => self.check_compare_op(
                c.kind,
                c.left.as_ref(),
                c.right.as_ref(),
                vars,
                errs,
                untrusted,
            ),
            ExprNodeKind::LogicalOp(l) => self.check_logical_op(l, vars, errs, untrusted),
        }
    }

    /// Checks the given expression, returning its static type and any errors found.
    #[must_use]
    pub fn check(&self, expr: &ExprNode) -> (ExprTypeKind, Vec<ExprError>) {
        let vars = self.vars();
        let mut errs: Vec<ExprError> = Vec::new();

        let mut untrusted = if self.check_untrusted_input {
            Some(UntrustedInputChecker::new(builtin_untrusted_inputs()))
        } else {
            None
        };
        if let Some(u) = untrusted.as_mut() {
            u.init();
        }

        let ty = self.check_inner(expr, &vars, &mut errs, &mut untrusted);

        if let Some(u) = untrusted.as_mut() {
            u.on_visit_end();
            errs.append(&mut u.take_errs());
        }
        (ty, errs)
    }

    /// Returns whether the given expression is a constant. For example the following expressions are
    /// constants.
    ///
    /// - `1 == 2`
    /// - `(!true && 'foo') || 'bar'`
    /// - `startsWith('foobar', 'foo')`
    /// - `format('{} + {} = {}', 1, 2, 3)`
    #[must_use]
    pub fn is_constant(expr: &ExprNode) -> bool {
        match expr.as_ref() {
            ExprNodeKind::Null(_)
            | ExprNodeKind::Bool(_)
            | ExprNodeKind::Int(_)
            | ExprNodeKind::Float(_)
            | ExprNodeKind::String(_) => true,
            ExprNodeKind::Variable(_)
            | ExprNodeKind::ObjectDeref(_)
            | ExprNodeKind::ArrayDeref(_)
            | ExprNodeKind::IndexAccess(_) => false,
            ExprNodeKind::NotOp(not) => match &not.operand {
                Some(o) => Self::is_constant(o),
                None => false,
            },
            ExprNodeKind::CompareOp(c) => {
                let l = c.left.as_ref().is_some_and(Self::is_constant);
                let r = c.right.as_ref().is_some_and(Self::is_constant);
                l && r
            }
            ExprNodeKind::LogicalOp(l) => {
                let lg = l.left.as_ref().is_some_and(Self::is_constant);
                let rg = l.right.as_ref().is_some_and(Self::is_constant);
                lg && rg
            }
            ExprNodeKind::FuncCall(f) => {
                for a in &f.args {
                    if !Self::is_constant(a) {
                        return false;
                    }
                }
                let Some(sigs) = builtin_func_signatures().get(f.callee.to_lowercase().as_str())
                else {
                    return false;
                };
                sigs.iter().all(|s| s.is_const_func)
            }
        }
    }
}

fn error_at_expr(t: &Token, msg: String) -> ExprError {
    ExprError {
        message: msg,
        offset: t.offset,
        line: t.line,
        column: t.column,
    }
}

/// Converts a 1-based (line, column) position within `s` into a 0-based byte offset. This mirrors
/// how the Go original reports `json.SyntaxError.Offset` (a byte offset), while `serde_json` exposes
/// the error position as a line and column pair.
fn byte_offset_at(s: &str, line: usize, col: usize) -> usize {
    if line <= 1 {
        return col.saturating_sub(1).min(s.len());
    }
    let mut cur_line = 1usize;
    for (i, c) in s.char_indices() {
        if c == '\n' {
            cur_line += 1;
            if cur_line == line {
                // The start of the target line is the byte after this newline.
                let line_start = i + c.len_utf8();
                return (line_start + col.saturating_sub(1)).min(s.len());
            }
        }
    }
    s.len()
}

/// Strips the trailing " at line `L` column `C`" suffix that `serde_json` appends to its error messages,
/// so the text matches the position-free message that Go's `json.SyntaxError` produces.
fn strip_json_error_position(msg: &str) -> String {
    match msg.find(" at line ") {
        Some(i) => msg[..i].to_owned(),
        None => msg.to_owned(),
    }
}

fn empty_token() -> Token {
    Token {
        kind: crate::expr_lexer::TokenKind::Unknown,
        value: String::new(),
        offset: 0,
        line: 0,
        column: 0,
    }
}

fn index_token(index: &ExprNode) -> Token {
    index.token().cloned().unwrap_or_else(empty_token)
}

fn operand_token(operand: &ExprNode) -> Token {
    operand.token().cloned().unwrap_or_else(empty_token)
}

/// Validates the given arguments against a function signature, returning an error on the first
/// mismatch.
fn check_func_signature(
    n: &FuncCallNode,
    sig: &FuncSignature,
    args: &[ExprTypeKind],
) -> Result<(), ExprError> {
    let lp = sig.params.len();
    let la = args.len();
    if (sig.variable_length_params && lp > la) || (!sig.variable_length_params && lp != la) {
        let at_least = if sig.variable_length_params {
            "at least "
        } else {
            ""
        };
        let msg = format!(
            "number of arguments is wrong. function {} takes {at_least}{lp} parameters but {la} arguments are given",
            q(sig.display(&n.callee))
        );
        return Err(error_at_expr(&n.tok, msg));
    }

    for i in 0..sig.params.len() {
        let p = &sig.params[i];
        let a = &args[i];
        if !p.assignable(a) {
            let msg = format!(
                "{} argument of function call is not assignable. {} cannot be assigned to {}. called function type is {}",
                ordinal(i + 1),
                q_ty(a),
                q_ty(p),
                q(sig.display(&n.callee))
            );
            let tok = n.args[i].token().cloned().unwrap_or_else(|| n.tok.clone());
            return Err(error_at_expr(&tok, msg));
        }
    }

    // Note: Unlike many languages, this check does not allow 0 argument for the variable length
    // parameter since it is useful for checking hashFiles() and format().
    if sig.variable_length_params {
        let p = &sig.params[lp - 1];
        for (i, a) in args[lp..].iter().enumerate() {
            if !p.assignable(a) {
                let msg = format!(
                    "{} argument of function call is not assignable. {} cannot be assigned to {}. called function type is {}",
                    ordinal(lp + i + 1),
                    q_ty(a),
                    q_ty(p),
                    q(sig.display(&n.callee))
                );
                let tok = n.args[lp + i]
                    .token()
                    .cloned()
                    .unwrap_or_else(|| n.tok.clone());
                return Err(error_at_expr(&tok, msg));
            }
        }
    }

    Ok(())
}

/// Validates the operands of a comparison operator. Returns `false` when the operands cannot be
/// compared with the operator.
fn validate_compare_op_operands(
    op: crate::expr_ast::CompareOpKind,
    l: &ExprTypeKind,
    r: &ExprTypeKind,
) -> bool {
    // Comparison behavior:
    // https://docs.github.com/en/actions/learn-github-actions/expressions#operators
    use crate::expr_ast::CompareOpKind as K;
    use ExprTypeKindInner as T;
    match op {
        K::Eq | K::NotEq => match l.inner() {
            T::Any | T::Null => true,
            T::Number | T::Bool | T::String => match r.inner() {
                // These are coerced to NaN hence the comparison result is always false
                T::Object(_) | T::Array(_) => false,
                _ => true,
            },
            T::Object(_) => matches!(r.inner(), T::Object(_) | T::Null | T::Any),
            T::Array(a) => match r.inner() {
                T::Array(b) => validate_compare_op_operands(op, &a.elem, &b.elem),
                T::Null | T::Any => true,
                _ => false,
            },
        },
        K::Less | K::LessEq | K::Greater | K::GreaterEq => {
            // null, bool, array, and object cannot be compared with these operators
            match l.inner() {
                T::Any | T::Number | T::String => {
                    !matches!(r.inner(), T::Null | T::Bool | T::Object(_) | T::Array(_))
                }
                T::Null | T::Bool | T::Object(_) | T::Array(_) => false,
            }
        }
        K::Invalid => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr_parser::parse_expression;

    fn parse_ok(src: &str) -> ExprNode {
        match parse_expression(src) {
            Ok((n, _)) => n,
            Err(e) => panic!("failed to parse {src:?}: {e}"),
        }
    }

    fn check_type(src: &str) -> (ExprTypeKind, Vec<ExprError>) {
        let node = parse_ok(src);
        ExprSemanticsChecker::new(false, None).check(&node)
    }

    fn check_type_with(
        src: &str,
        f: impl FnOnce(&mut ExprSemanticsChecker),
    ) -> (ExprTypeKind, Vec<ExprError>) {
        let node = parse_ok(src);
        let mut c = ExprSemanticsChecker::new(false, None);
        f(&mut c);
        c.check(&node)
    }

    #[test]
    fn ordinal_numbers() {
        let tt = [
            ("1st", 1, "st"),
            ("2nd", 2, "nd"),
            ("3rd", 3, "rd"),
            ("4th", 4, "th"),
            ("11th", 11, "th"),
            ("12th", 12, "th"),
            ("13th", 13, "th"),
            ("21st", 21, "st"),
            ("22nd", 22, "nd"),
        ];
        for (name, i, want) in tt {
            assert_eq!(ordinal(i), format!("{i}{want}"), "{name}");
        }
    }

    #[test]
    fn format_specifiers() {
        let tt: &[(&str, &str, &[usize])] = &[
            ("single", "{0}", &[0]),
            ("two", "{0} {1}", &[0, 1]),
            ("escaped", "{{0}}", &[]),       // escaped
            ("contains", "{0}}}", &[0]),     // contains a specifier
            ("not contains", "{0}}}}", &[]), // doesn't
            ("none", "no placeholders", &[]),
            ("multi digit", "{10}", &[10]),
        ];
        for (name, f, want) in tt {
            let got = parse_format_func_specifiers(f, 2);
            let want: BTreeSet<usize> = want.iter().copied().collect();
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn valid_expression_types() {
        let tt = [
            ("null literal", "null }}", "null"),
            ("bool literal", "true }}", "bool"),
            ("int literal", "42 }}", "number"),
            ("float literal", "3.5 }}", "number"),
            ("string literal", "'hi' }}", "string"),
            ("github context prop", "github.event_name }}", "string"),
            ("github number prop", "github.retention_days }}", "number"),
            ("github bool prop", "github.ref_protected }}", "bool"),
            ("loose event payload", "github.event.foo }}", "any"),
            ("env var", "env.FOO }}", "string"),
            ("vars", "vars.FOO }}", "string"),
            ("secrets", "secrets.FOO }}", "string"),
            ("runner", "runner.os }}", "string"),
            ("strategy", "strategy.fail-fast }}", "bool"),
            ("contains strings", "contains('foo', 'o') }}", "bool"),
            (
                "contains array",
                "contains(fromJSON('[1,2]'), 1) }}",
                "bool",
            ),
            ("startsWith", "startsWith('foo', 'f') }}", "bool"),
            ("endsWith", "endsWith('foo', 'o') }}", "bool"),
            ("toJSON", "toJSON(github.event) }}", "string"),
            ("fromJSON", "fromJSON('1') }}", "number"),
            ("fromJSON object", "fromJSON('{\"a\":1}').a }}", "number"),
            ("join", "join(fromJSON('[\"a\",\"b\"]'), ',') }}", "string"),
            ("format", "format('{0}', 'x') }}", "string"),
            ("case", "case(true, 'a', 'b') }}", "any"),
            ("not op", "!true }}", "bool"),
            ("eq op", "'a' == 'b' }}", "bool"),
            ("lt op", "1 < 2 }}", "bool"),
            ("and op", "true && false }}", "bool"),
            (
                "array deref",
                "github.event.commits.*.message }}",
                "array<any>",
            ),
            ("index access num", "fromJSON('[1,2]')[0] }}", "number"),
            ("index access str", "github['event_name'] }}", "string"),
            (
                "narrowing number",
                "(github.event_name == 'push') && 60 || 20 }}",
                "number",
            ),
        ];
        for (name, src, want) in tt {
            let (ty, errs) = check_type(src);
            assert!(
                errs.is_empty(),
                "{name}: unexpected errors: {:?}",
                errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
            );
            assert_eq!(ty.to_string(), want, "{name}");
        }
    }

    #[test]
    fn undefined_variable() {
        let (ty, errs) = check_type("foo }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "undefined variable \"foo\". available variables are \"env\", \"github\", \"inputs\", \"job\", \"matrix\", \"needs\", \"runner\", \"secrets\", \"steps\", \"strategy\", \"vars\""
        );
        assert_eq!(ty.to_string(), "any");
    }

    #[test]
    fn unknown_property_in_strict_object() {
        let (_, errs) = check_type("github.foo }}");
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .starts_with("property \"foo\" is not defined in object type {"),
            "{}",
            errs[0].message
        );
    }

    #[test]
    fn wrong_arg_count() {
        let (_, errs) = check_type("startsWith('foo') }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "number of arguments is wrong. function \"startsWith(string, string) -> bool\" takes 2 parameters but 1 arguments are given"
        );
    }

    #[test]
    fn wrong_arg_count_variadic() {
        let (_, errs) = check_type("hashFiles() }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "number of arguments is wrong. function \"hashFiles(string...) -> string\" takes at least 1 parameters but 0 arguments are given"
        );
    }

    #[test]
    fn wrong_arg_type() {
        // `array<number>` is not assignable to the `string` parameter of `startsWith`.
        let (_, errs) = check_type("startsWith(fromJSON('[1,2]'), 'foo') }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "1st argument of function call is not assignable. \"array<number>\" cannot be assigned to \"string\". called function type is \"startsWith(string, string) -> bool\""
        );

        // Too many args to a non-variadic function is also a type error free path.
        let (_, errs) = check_type("toJSON(github.event, 'x') }}");
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.starts_with("number of arguments is wrong."),
            "{}",
            errs[0].message
        );
    }

    #[test]
    fn unknown_function() {
        let (_, errs) = check_type("fooBar() }}");
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .starts_with("undefined function \"fooBar\". available functions are "),
            "{}",
            errs[0].message
        );
    }

    #[test]
    fn comparison_type_mismatch() {
        let (_, errs) = check_type("github.event < 'foo' }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "\"object\" value cannot be compared to \"string\" value with \"<\" operator"
        );
    }

    #[test]
    fn index_access_errors() {
        let (ty, errs) = check_type("fromJSON('[1,2]')['foo'] }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "index access of array must be type of number but got \"string\""
        );
        assert_eq!(ty.to_string(), "any");

        let (_, errs) = check_type("github.event[1] }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "property access of object must be type of string but got \"number\""
        );
    }

    #[test]
    fn array_deref_errors() {
        // `startsWith` returns a bool, which cannot be object-filtered.
        let (_, errs) = check_type("startsWith('foo', 'bar').* }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "receiver of object filtering `.*` must be type of array or object but got \"bool\""
        );
    }

    #[test]
    fn format_placeholder_checks() {
        // Unused argument
        let (_, errs) = check_type("format('{0}', 'a', 'b') }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "format string \"{0}\" does not contain placeholder {1}. remove argument which is unused in the format string"
        );

        // Missing argument
        let (_, errs) = check_type("format('{0} {1}', 'a') }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "format string \"{0} {1}\" contains placeholder {1} but only 1 arguments are given to format"
        );
    }

    #[test]
    fn broken_json_in_fromjson() {
        let (_, errs) = check_type("fromJSON('{\"a\":}') }}");
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .starts_with("broken JSON string is passed to fromJSON() at offset "),
            "{}",
            errs[0].message
        );
    }

    #[test]
    fn case_requires_odd_args() {
        // The signature check (at least 3 params) passes with 4 args, then the odd-count check fires.
        let (_, errs) = check_type("case(true, 'a', 'b', 'c') }}");
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "case() requires an odd number of arguments (pred/value pairs + default) but got 4"
        );
    }

    #[test]
    fn config_variables() {
        // Reserved GITHUB_ prefix
        let (_, errs) = check_type_with("vars.github_foo }}", |c| {
            c.config_vars = Some(vec!["foo".to_string()]);
        });
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.starts_with(
                "configuration variable name \"github_foo\" must not start with the GITHUB_ prefix"
            ),
            "{}",
            errs[0].message
        );

        // Empty config var list
        let (_, errs) = check_type_with("vars.foo }}", |c| {
            c.config_vars = Some(Vec::new());
        });
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.starts_with(
                "no configuration variable is allowed since the variables list is empty"
            ),
            "{}",
            errs[0].message
        );

        // Undefined config var
        let (_, errs) = check_type_with("vars.foo }}", |c| {
            c.config_vars = Some(vec!["bar".to_string()]);
        });
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "undefined configuration variable \"foo\". defined configuration variables in actionlint.yaml are \"bar\""
        );

        // Valid config var (case insensitive)
        let (_, errs) = check_type_with("vars.FOO }}", |c| {
            c.config_vars = Some(vec!["foo".to_string()]);
        });
        assert!(errs.is_empty());
    }

    #[test]
    fn context_availability() {
        let (_, errs) = check_type_with("github }}", |c| {
            c.set_context_availability(vec!["env".to_string()]);
        });
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0].message,
            "context \"github\" is not allowed here. available context is \"env\". see https://docs.github.com/en/actions/learn-github-actions/contexts#context-availability for more details"
        );

        let (_, errs) = check_type_with("github }}", |c| {
            c.set_context_availability(Vec::new());
        });
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no context is available here"));
    }

    #[test]
    fn special_function_availability() {
        let (_, errs) = check_type_with("success() }}", |c| {
            c.set_special_function_availability(Vec::new());
        });
        assert!(errs.iter().any(|e| {
            e.message
                .starts_with("calling function \"success\" is not allowed here")
        }));

        // Allowed when listed
        let (_, errs) = check_type_with("success() }}", |c| {
            c.set_special_function_availability(vec!["success".to_string()]);
        });
        assert!(errs.is_empty());
    }

    #[test]
    fn untrusted_input() {
        let node = parse_ok("github.event.issue.title }}");
        let c = ExprSemanticsChecker::new(true, None);
        let (_, errs) = c.check(&node);
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("\"github.event.issue.title\" is potentially untrusted"),
            "{}",
            errs[0].message
        );

        // Not reported when untrusted check is disabled
        let c = ExprSemanticsChecker::new(false, None);
        let (_, errs) = c.check(&node);
        assert!(errs.is_empty());
    }

    #[test]
    fn is_constant_cases() {
        let tt = [
            ("1 == 2", "1 == 2 }}", true),
            ("logical of constants", "(!true && 'foo') || 'bar' }}", true),
            ("startsWith", "startsWith('foobar', 'foo') }}", true),
            ("format", "format('{0} + {1}', 1, 2) }}", true),
            ("variable", "github }}", false),
            ("deref", "github.event }}", false),
            ("fromJSON is not const", "fromJSON('1') }}", false),
            ("hashFiles is not const", "hashFiles('foo') }}", false),
            ("success is not const", "success() }}", false),
        ];
        for (name, src, want) in tt {
            let node = parse_ok(src);
            assert_eq!(ExprSemanticsChecker::is_constant(&node), want, "{name}");
        }
    }

    #[test]
    fn matrix_context_override() {
        let node = parse_ok("matrix.os }}");
        let mut props = BTreeMap::new();
        props.insert("os".to_string(), ExprTypeKind::string());
        let mut c = ExprSemanticsChecker::new(false, None);
        c.update_matrix(ObjectType {
            props,
            mapped: None,
        });
        let (ty, errs) = c.check(&node);
        assert!(errs.is_empty());
        assert_eq!(ty.to_string(), "string");
    }
}
