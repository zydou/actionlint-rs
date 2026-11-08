//! Context and special-function availability for workflow keys.
//!
//! Rust counterpart of the original generated `availability.go`. It reports which contexts and
//! special functions are available at a given workflow key.
//!
//! Generated from
//! <https://docs.github.com/en/actions/learn-github-actions/contexts#context-availability>.

use std::collections::HashMap;

/// Returns the contexts and special functions available at the given workflow key.
///
/// The `key` represents a workflow key like `jobs.<job_id>.concurrency`. The first return value
/// lists the available contexts (an empty slice means any context is available). The second lists
/// the available special functions (an empty slice means none are available).
#[must_use]
pub fn workflow_key_availability(key: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match key {
        "jobs.<job_id>.outputs.<output_id>" => (
            &[
                "env", "github", "inputs", "job", "matrix", "needs", "runner", "secrets", "steps",
                "strategy", "vars",
            ],
            &[],
        ),
        "jobs.<job_id>.steps.continue-on-error"
        | "jobs.<job_id>.steps.env"
        | "jobs.<job_id>.steps.name"
        | "jobs.<job_id>.steps.run"
        | "jobs.<job_id>.steps.timeout-minutes"
        | "jobs.<job_id>.steps.with"
        | "jobs.<job_id>.steps.working-directory" => (
            &[
                "env", "github", "inputs", "job", "matrix", "needs", "runner", "secrets", "steps",
                "strategy", "vars",
            ],
            &["hashfiles"],
        ),
        "jobs.<job_id>.container.env.<env_id>"
        | "jobs.<job_id>.services.<service_id>.env.<env_id>" => (
            &[
                "env", "github", "inputs", "job", "matrix", "needs", "runner", "secrets",
                "strategy", "vars",
            ],
            &[],
        ),
        "jobs.<job_id>.environment.url" => (
            &[
                "env", "github", "inputs", "job", "matrix", "needs", "runner", "steps", "strategy",
                "vars",
            ],
            &[],
        ),
        "jobs.<job_id>.steps.if" => (
            &[
                "env", "github", "inputs", "job", "matrix", "needs", "runner", "steps", "strategy",
                "vars",
            ],
            &["always", "cancelled", "failure", "hashfiles", "success"],
        ),
        "jobs.<job_id>.container.credentials"
        | "jobs.<job_id>.services.<service_id>.credentials" => (
            &[
                "env", "github", "inputs", "matrix", "needs", "secrets", "strategy", "vars",
            ],
            &[],
        ),
        "jobs.<job_id>.defaults.run" => (
            &[
                "env", "github", "inputs", "matrix", "needs", "strategy", "vars",
            ],
            &[],
        ),
        "on.workflow_call.outputs.<output_id>.value" => {
            (&["github", "inputs", "jobs", "vars"], &[])
        }
        "jobs.<job_id>.env" | "jobs.<job_id>.secrets.<secrets_id>" => (
            &[
                "github", "inputs", "matrix", "needs", "secrets", "strategy", "vars",
            ],
            &[],
        ),
        "jobs.<job_id>.concurrency"
        | "jobs.<job_id>.container"
        | "jobs.<job_id>.container.image"
        | "jobs.<job_id>.continue-on-error"
        | "jobs.<job_id>.environment"
        | "jobs.<job_id>.name"
        | "jobs.<job_id>.runs-on"
        | "jobs.<job_id>.services"
        | "jobs.<job_id>.timeout-minutes"
        | "jobs.<job_id>.with.<with_id>" => (
            &["github", "inputs", "matrix", "needs", "strategy", "vars"],
            &[],
        ),
        "jobs.<job_id>.strategy" => (&["github", "inputs", "needs", "vars"], &[]),
        "jobs.<job_id>.if" => (
            &["github", "inputs", "needs", "vars"],
            &["always", "cancelled", "failure", "success"],
        ),
        "env" => (&["github", "inputs", "secrets", "vars"], &[]),
        "concurrency" | "on.workflow_call.inputs.<inputs_id>.default" | "run-name" => {
            (&["github", "inputs", "vars"], &[])
        }
        "jobs.<job_id>.snapshot.if" => (
            &["github", "inputs", "matrix", "needs", "strategy", "vars"],
            &[],
        ),
        _ => (&[], &[]),
    }
}

/// A map from special function name to the workflow keys where it is available.
pub fn special_function_names() -> &'static HashMap<&'static str, &'static [&'static str]> {
    static MAP: std::sync::OnceLock<HashMap<&'static str, &'static [&'static str]>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                "always",
                &["jobs.<job_id>.if", "jobs.<job_id>.steps.if"][..],
            ),
            (
                "cancelled",
                &["jobs.<job_id>.if", "jobs.<job_id>.steps.if"][..],
            ),
            (
                "failure",
                &["jobs.<job_id>.if", "jobs.<job_id>.steps.if"][..],
            ),
            (
                "hashfiles",
                &[
                    "jobs.<job_id>.steps.continue-on-error",
                    "jobs.<job_id>.steps.env",
                    "jobs.<job_id>.steps.if",
                    "jobs.<job_id>.steps.name",
                    "jobs.<job_id>.steps.run",
                    "jobs.<job_id>.steps.timeout-minutes",
                    "jobs.<job_id>.steps.with",
                    "jobs.<job_id>.steps.working-directory",
                ][..],
            ),
            (
                "success",
                &["jobs.<job_id>.if", "jobs.<job_id>.steps.if"][..],
            ),
        ])
    })
}

/// A map from context name to the workflow keys that allow the context.
pub fn all_contexts() -> &'static HashMap<&'static str, Vec<&'static str>> {
    static MAP: std::sync::OnceLock<HashMap<&'static str, Vec<&'static str>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        let insert = |m: &mut HashMap<&'static str, Vec<&'static str>>,
                      ctx: &'static str,
                      keys: &[&'static str]| {
            m.entry(ctx).or_default().extend_from_slice(keys);
        };

        let env_keys = [
            "jobs.<job_id>.container.credentials",
            "jobs.<job_id>.container.env.<env_id>",
            "jobs.<job_id>.defaults.run",
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.services.<service_id>.credentials",
            "jobs.<job_id>.services.<service_id>.env.<env_id>",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
        ];
        insert(&mut m, "env", &env_keys);

        let github_keys = [
            "concurrency",
            "env",
            "jobs.<job_id>.concurrency",
            "jobs.<job_id>.container",
            "jobs.<job_id>.container.credentials",
            "jobs.<job_id>.container.env.<env_id>",
            "jobs.<job_id>.container.image",
            "jobs.<job_id>.continue-on-error",
            "jobs.<job_id>.defaults.run",
            "jobs.<job_id>.env",
            "jobs.<job_id>.environment",
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.if",
            "jobs.<job_id>.name",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.runs-on",
            "jobs.<job_id>.secrets.<secrets_id>",
            "jobs.<job_id>.services",
            "jobs.<job_id>.services.<service_id>.credentials",
            "jobs.<job_id>.services.<service_id>.env.<env_id>",
            "jobs.<job_id>.snapshot.if",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
            "jobs.<job_id>.strategy",
            "jobs.<job_id>.timeout-minutes",
            "jobs.<job_id>.with.<with_id>",
            "on.workflow_call.inputs.<inputs_id>.default",
            "on.workflow_call.outputs.<output_id>.value",
            "run-name",
        ];
        insert(&mut m, "github", &github_keys);
        // inputs has the same availability as github.
        insert(&mut m, "inputs", &github_keys);

        let job_keys = [
            "jobs.<job_id>.container.env.<env_id>",
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.services.<service_id>.env.<env_id>",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
        ];
        insert(&mut m, "job", &job_keys);
        insert(&mut m, "runner", &job_keys);

        insert(
            &mut m,
            "jobs",
            &["on.workflow_call.outputs.<output_id>.value"][..],
        );

        let matrix_keys = [
            "jobs.<job_id>.concurrency",
            "jobs.<job_id>.container",
            "jobs.<job_id>.container.credentials",
            "jobs.<job_id>.container.env.<env_id>",
            "jobs.<job_id>.container.image",
            "jobs.<job_id>.continue-on-error",
            "jobs.<job_id>.defaults.run",
            "jobs.<job_id>.env",
            "jobs.<job_id>.environment",
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.name",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.runs-on",
            "jobs.<job_id>.secrets.<secrets_id>",
            "jobs.<job_id>.services",
            "jobs.<job_id>.services.<service_id>.credentials",
            "jobs.<job_id>.services.<service_id>.env.<env_id>",
            "jobs.<job_id>.snapshot.if",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
            "jobs.<job_id>.timeout-minutes",
            "jobs.<job_id>.with.<with_id>",
        ];
        insert(&mut m, "matrix", &matrix_keys);
        insert(&mut m, "needs", &matrix_keys);

        let steps_keys = [
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
        ];
        insert(&mut m, "steps", &steps_keys);

        insert(
            &mut m,
            "secrets",
            &[
                "env",
                "jobs.<job_id>.container.credentials",
                "jobs.<job_id>.container.env.<env_id>",
                "jobs.<job_id>.env",
                "jobs.<job_id>.outputs.<output_id>",
                "jobs.<job_id>.secrets.<secrets_id>",
                "jobs.<job_id>.services.<service_id>.credentials",
                "jobs.<job_id>.services.<service_id>.env.<env_id>",
                "jobs.<job_id>.steps.continue-on-error",
                "jobs.<job_id>.steps.env",
                "jobs.<job_id>.steps.name",
                "jobs.<job_id>.steps.run",
                "jobs.<job_id>.steps.timeout-minutes",
                "jobs.<job_id>.steps.with",
                "jobs.<job_id>.steps.working-directory",
            ][..],
        );

        insert(
            &mut m,
            "strategy",
            &[
                "jobs.<job_id>.concurrency",
                "jobs.<job_id>.container",
                "jobs.<job_id>.container.credentials",
                "jobs.<job_id>.container.env.<env_id>",
                "jobs.<job_id>.container.image",
                "jobs.<job_id>.continue-on-error",
                "jobs.<job_id>.defaults.run",
                "jobs.<job_id>.env",
                "jobs.<job_id>.environment",
                "jobs.<job_id>.environment.url",
                "jobs.<job_id>.name",
                "jobs.<job_id>.outputs.<output_id>",
                "jobs.<job_id>.runs-on",
                "jobs.<job_id>.secrets.<secrets_id>",
                "jobs.<job_id>.services",
                "jobs.<job_id>.services.<service_id>.credentials",
                "jobs.<job_id>.services.<service_id>.env.<env_id>",
                "jobs.<job_id>.snapshot.if",
                "jobs.<job_id>.steps.continue-on-error",
                "jobs.<job_id>.steps.env",
                "jobs.<job_id>.steps.if",
                "jobs.<job_id>.steps.name",
                "jobs.<job_id>.steps.run",
                "jobs.<job_id>.steps.timeout-minutes",
                "jobs.<job_id>.steps.with",
                "jobs.<job_id>.steps.working-directory",
                "jobs.<job_id>.timeout-minutes",
                "jobs.<job_id>.with.<with_id>",
            ][..],
        );

        let vars_keys = [
            "concurrency",
            "env",
            "jobs.<job_id>.concurrency",
            "jobs.<job_id>.container",
            "jobs.<job_id>.container.credentials",
            "jobs.<job_id>.container.env.<env_id>",
            "jobs.<job_id>.container.image",
            "jobs.<job_id>.continue-on-error",
            "jobs.<job_id>.defaults.run",
            "jobs.<job_id>.env",
            "jobs.<job_id>.environment",
            "jobs.<job_id>.environment.url",
            "jobs.<job_id>.if",
            "jobs.<job_id>.name",
            "jobs.<job_id>.outputs.<output_id>",
            "jobs.<job_id>.runs-on",
            "jobs.<job_id>.secrets.<secrets_id>",
            "jobs.<job_id>.services",
            "jobs.<job_id>.services.<service_id>.credentials",
            "jobs.<job_id>.services.<service_id>.env.<env_id>",
            "jobs.<job_id>.snapshot.if",
            "jobs.<job_id>.steps.continue-on-error",
            "jobs.<job_id>.steps.env",
            "jobs.<job_id>.steps.if",
            "jobs.<job_id>.steps.name",
            "jobs.<job_id>.steps.run",
            "jobs.<job_id>.steps.timeout-minutes",
            "jobs.<job_id>.steps.with",
            "jobs.<job_id>.steps.working-directory",
            "jobs.<job_id>.strategy",
            "jobs.<job_id>.timeout-minutes",
            "jobs.<job_id>.with.<with_id>",
            "on.workflow_call.inputs.<inputs_id>.default",
            "on.workflow_call.outputs.<output_id>.value",
            "run-name",
        ];
        insert(&mut m, "vars", &vars_keys);

        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_availability() {
        let (ctxs, funcs) = workflow_key_availability("jobs.<job_id>.outputs.<output_id>");
        assert!(ctxs.contains(&"job"));
        assert!(ctxs.contains(&"steps"));
        assert!(funcs.is_empty());
    }

    #[test]
    fn steps_if_has_special_funcs() {
        let (_, funcs) = workflow_key_availability("jobs.<job_id>.steps.if");
        assert!(funcs.contains(&"always"));
        assert!(funcs.contains(&"hashfiles"));
        assert!(funcs.contains(&"success"));
    }

    #[test]
    fn job_if_special_funcs() {
        let (_, funcs) = workflow_key_availability("jobs.<job_id>.if");
        assert!(funcs.contains(&"always"));
        assert!(!funcs.contains(&"hashfiles"));
    }

    #[test]
    fn strategy_no_job_context() {
        let (ctxs, _) = workflow_key_availability("jobs.<job_id>.strategy");
        assert!(!ctxs.contains(&"job"));
        assert!(!ctxs.contains(&"steps"));
    }

    #[test]
    fn unknown_key() {
        let (ctxs, funcs) = workflow_key_availability("unknown.key");
        assert!(ctxs.is_empty());
        assert!(funcs.is_empty());
    }

    #[test]
    fn special_function_names_lookup() {
        let m = special_function_names();
        assert!(m.contains_key("always"));
        assert!(m.contains_key("hashfiles"));
        assert!(!m.contains_key("contains"));
    }

    #[test]
    fn all_contexts_lookup() {
        let m = all_contexts();
        assert!(m.contains_key("github"));
        assert!(m.contains_key("steps"));
        // github is available in many places.
        assert!(m.get("github").unwrap().len() > 20);
    }
}
