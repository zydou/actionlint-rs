//! Metadata for validating local reusable workflow files.
//!
//! Rust counterpart of the original `reusable_workflow.go`. It parses the `workflow_call` event
//! trigger of local reusable workflow files and caches them to avoid repeated reads. See
//! <https://docs.github.com/en/actions/using-workflows/reusing-workflows>.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use saphyr::LoadableYamlNode;

use crate::action_metadata::yaml_to_serde;
use crate::ast::{WorkflowCallEvent, WorkflowCallEventInputType};
use crate::expr_type::{ExprTypeKind, ExprTypeKindInner};
use crate::project::Project;

/// Input metadata for validating a local reusable workflow file.
#[derive(Debug, Clone)]
pub struct ReusableWorkflowMetadataInput {
    /// The name of the input defined in the reusable workflow.
    pub name: String,
    /// Whether the `required` field is set to true and no default value is set.
    pub required: bool,
    /// The type of the input. Unknown input types map to `any`.
    pub ty: ExprTypeKind,
}

/// A map from input name to reusable workflow input metadata. Keys are lower cased since input
/// names of workflow calls are case insensitive.
pub type ReusableWorkflowMetadataInputs = BTreeMap<String, ReusableWorkflowMetadataInput>;

/// Secret metadata for validating a local reusable workflow file.
#[derive(Debug, Clone)]
pub struct ReusableWorkflowMetadataSecret {
    /// The name of the secret in the reusable workflow.
    pub name: String,
    /// Whether the secret is required. When true, workflow calls must set it unless secrets are
    /// inherited.
    pub required: bool,
}

/// A map from secret name to reusable workflow secret metadata. Keys are lower cased.
pub type ReusableWorkflowMetadataSecrets = BTreeMap<String, ReusableWorkflowMetadataSecret>;

/// Output metadata for validating a local reusable workflow file.
#[derive(Debug, Clone)]
pub struct ReusableWorkflowMetadataOutput {
    /// The name of the output in the reusable workflow.
    pub name: String,
}

/// A map from output name to reusable workflow output metadata. Keys are lower cased.
pub type ReusableWorkflowMetadataOutputs = BTreeMap<String, ReusableWorkflowMetadataOutput>;

/// Metadata to validate local reusable workflows. Only the metadata necessary for validation is
/// contained.
#[derive(Debug, Clone, Default)]
pub struct ReusableWorkflowMetadata {
    /// The inputs of the reusable workflow.
    pub inputs: ReusableWorkflowMetadataInputs,
    /// The outputs of the reusable workflow.
    pub outputs: ReusableWorkflowMetadataOutputs,
    /// The secrets of the reusable workflow.
    pub secrets: ReusableWorkflowMetadataSecrets,
}

fn input_type_from_str(s: &str) -> ExprTypeKind {
    match s {
        "boolean" => ExprTypeKind::new(ExprTypeKindInner::Bool),
        "number" => ExprTypeKind::new(ExprTypeKindInner::Number),
        "string" => ExprTypeKind::new(ExprTypeKindInner::String),
        _ => ExprTypeKind::any(),
    }
}

fn parse_inputs(v: &serde_json::Value) -> Result<ReusableWorkflowMetadataInputs, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "yaml: on.workflow_call.inputs must be mapping node".to_string())?;
    let mut m = ReusableWorkflowMetadataInputs::new();
    for (k, val) in obj {
        let required = val
            .get("required")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let has_default = val.get("default").is_some_and(|x| !x.is_null());
        let ty = val
            .get("type")
            .and_then(|x| x.as_str())
            .map_or_else(ExprTypeKind::any, input_type_from_str);
        m.insert(
            k.to_lowercase(),
            ReusableWorkflowMetadataInput {
                name: k.clone(),
                required: required && !has_default,
                ty,
            },
        );
    }
    Ok(m)
}

fn parse_secrets(v: &serde_json::Value) -> Result<ReusableWorkflowMetadataSecrets, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "yaml: on.workflow_call.secrets must be mapping node".to_string())?;
    let mut m = ReusableWorkflowMetadataSecrets::new();
    for (k, val) in obj {
        let required = val
            .get("required")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        m.insert(
            k.to_lowercase(),
            ReusableWorkflowMetadataSecret {
                name: k.clone(),
                required,
            },
        );
    }
    Ok(m)
}

fn parse_outputs(v: &serde_json::Value) -> Result<ReusableWorkflowMetadataOutputs, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "yaml: on.workflow_call.outputs must be mapping node".to_string())?;
    let mut m = ReusableWorkflowMetadataOutputs::new();
    for (k, _) in obj {
        m.insert(
            k.to_lowercase(),
            ReusableWorkflowMetadataOutput { name: k.clone() },
        );
    }
    Ok(m)
}

/// Parses reusable workflow metadata from the given bytes.
fn parse_reusable_workflow_metadata(src: &[u8]) -> Result<ReusableWorkflowMetadata, String> {
    let text = std::str::from_utf8(src).map_err(|e| e.to_string())?;
    let docs = saphyr::Yaml::load_from_str(text).map_err(|e| e.to_string())?;
    let doc = docs.into_iter().next().ok_or("empty workflow file")?;
    let root = yaml_to_serde(&doc);
    let obj = root.as_object().ok_or("\"on:\" is not found")?;
    let on = obj
        .get("on")
        .or_else(|| obj.get("true"))
        .ok_or("\"on:\" is not found")?;

    // `on:` may be a mapping (`on:\n  workflow_call:`), a scalar (`on: workflow_call`), or a
    // sequence (`on: [workflow_call]`).
    if let Some(map) = on.as_object() {
        for (k, v) in map {
            if k.eq_ignore_ascii_case("workflow_call") {
                return Ok(ReusableWorkflowMetadata {
                    inputs: if v.get("inputs").is_some() {
                        parse_inputs(v.get("inputs").unwrap())?
                    } else {
                        BTreeMap::new()
                    },
                    outputs: if v.get("outputs").is_some() {
                        parse_outputs(v.get("outputs").unwrap())?
                    } else {
                        BTreeMap::new()
                    },
                    secrets: if v.get("secrets").is_some() {
                        parse_secrets(v.get("secrets").unwrap())?
                    } else {
                        BTreeMap::new()
                    },
                });
            }
        }
        return Err("\"workflow_call\" event trigger is not found in \"on:\"".to_string());
    }
    if let Some(arr) = on.as_array() {
        for e in arr {
            if let Some(s) = e.as_str()
                && s.eq_ignore_ascii_case("workflow_call")
            {
                return Ok(ReusableWorkflowMetadata::default());
            }
        }
        return Err("\"workflow_call\" event trigger is not found in \"on:\"".to_string());
    }
    if let Some(s) = on.as_str()
        && s.eq_ignore_ascii_case("workflow_call")
    {
        return Ok(ReusableWorkflowMetadata::default());
    }

    Err("\"workflow_call\" event trigger is not found in \"on:\"".to_string())
}

/// A cache for local reusable workflow metadata files. It avoids repeated find/read/parse of local
/// reusable workflow YAML files. This cache is dedicated to a single project (repository). One
/// instance should be created per project.
pub struct LocalReusableWorkflowCache {
    proj: Option<Project>,
    cache: RwLock<BTreeMap<String, Option<ReusableWorkflowMetadata>>>,
    cwd: PathBuf,
    dbg: Option<std::sync::Mutex<Box<dyn Write + Send>>>,
}

impl std::fmt::Debug for LocalReusableWorkflowCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalReusableWorkflowCache")
            .field("proj", &self.proj)
            .finish()
    }
}

impl LocalReusableWorkflowCache {
    /// Creates a new cache for the given project. `cwd` is the current working directory. `dbg` is
    /// an optional debug writer. A `proj` of `None` creates a null cache that never hits.
    #[must_use]
    pub fn new(proj: Option<Project>, cwd: PathBuf, dbg: Option<Box<dyn Write + Send>>) -> Self {
        Self {
            proj,
            cache: RwLock::new(BTreeMap::new()),
            cwd,
            dbg: dbg.map(std::sync::Mutex::new),
        }
    }

    /// Creates a null cache that never hits. It is used when a project is not found.
    #[must_use]
    pub fn new_null(dbg: Option<Box<dyn Write + Send>>) -> Self {
        Self::new(None, PathBuf::new(), dbg)
    }

    fn debug(&self, msg: &str) {
        if let Some(w) = &self.dbg
            && let Ok(mut w) = w.lock()
        {
            let _ = writeln!(w, "[LocalReusableWorkflowCache] {msg}");
        }
    }

    /// Finds/parses a reusable workflow metadata located by the `spec` argument. When no project is
    /// set or the spec does not start with `./`, this immediately returns `Ok(None)`.
    ///
    /// An error is not cached: once an invalid workflow has been reported, later lookups of the same
    /// spec return `Ok(None)` instead of the error. This prevents repeating the same error from
    /// multiple places.
    pub fn find_metadata(&self, spec: &str) -> Result<Option<ReusableWorkflowMetadata>, String> {
        let Some(proj) = &self.proj else {
            return Ok(None);
        };
        if !spec.starts_with("./") || crate::ast::contains_expression(spec) {
            return Ok(None);
        }

        if let Some(hit) = self.cache.read().unwrap().get(spec) {
            self.debug(&format!("Cache hit for {spec}"));
            return Ok(hit.clone());
        }

        let file = proj.root_dir().join(spec.trim_start_matches("./"));
        let src = match std::fs::read(&file) {
            Ok(src) => src,
            Err(e) => {
                self.cache.write().unwrap().insert(spec.to_string(), None);
                return Err(format!(
                    "could not read reusable workflow file for {spec:?}: {e}"
                ));
            }
        };

        match parse_reusable_workflow_metadata(&src) {
            Ok(m) => {
                self.debug(&format!(
                    "New reusable workflow metadata at {}",
                    file.display()
                ));
                self.cache
                    .write()
                    .unwrap()
                    .insert(spec.to_string(), Some(m.clone()));
                Ok(Some(m))
            }
            Err(e) => {
                self.cache.write().unwrap().insert(spec.to_string(), None);
                Err(format!(
                    "error while parsing reusable workflow {spec:?}: {}",
                    e.replace('\n', " ")
                ))
            }
        }
    }

    fn conv_workflow_path_to_spec(&self, p: &Path) -> Option<String> {
        let proj = self.proj.as_ref()?;
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.cwd.join(p)
        };
        let r = proj.root_dir();
        if !p.starts_with(r) {
            return None;
        }
        let rel = p.strip_prefix(r).ok()?;
        let mut s = rel.to_string_lossy().replace('\\', "/");
        if !s.starts_with("./") {
            s = format!("./{s}");
        }
        Some(s)
    }

    /// Writes `val` into the cache for `key`, overwriting any existing entry. Counterpart of the Go
    /// `writeCache`.
    ///
    /// The workflow-call rule uses this to remember that an invalid local `uses` spec (e.g. one that
    /// carries a `@ref`) already produced a format error, so that a later `find_metadata` lookup
    /// returns `Ok(None)` instead of emitting a redundant "could not read reusable workflow file"
    /// error.
    pub fn write_cache(&self, key: &str, val: Option<ReusableWorkflowMetadata>) {
        self.cache.write().unwrap().insert(key.to_string(), val);
    }

    /// Writes reusable workflow metadata by converting from a [`WorkflowCallEvent`] AST node.
    /// `wpath` is a path to the workflow file of the AST, relative to the project root or absolute.
    ///
    /// This does nothing when (1) no project is set, (2) the workflow path cannot be converted to a
    /// workflow call spec, or (3) a cache for the workflow already exists.
    pub fn write_workflow_call_event(&self, wpath: &Path, event: &WorkflowCallEvent) {
        let Some(spec) = self.conv_workflow_path_to_spec(wpath) else {
            return;
        };
        self.debug(&format!(
            "Workflow call spec from workflow path {}: {spec}",
            wpath.display()
        ));

        if self.cache.read().unwrap().contains_key(&spec) {
            return;
        }

        let mut m = ReusableWorkflowMetadata::default();
        for i in &event.inputs {
            let ty = match i.input_type {
                WorkflowCallEventInputType::Boolean => ExprTypeKind::bool(),
                WorkflowCallEventInputType::Number => ExprTypeKind::number(),
                WorkflowCallEventInputType::String => ExprTypeKind::string(),
                WorkflowCallEventInputType::Invalid => ExprTypeKind::any(),
            };
            m.inputs.insert(
                i.id.clone(),
                ReusableWorkflowMetadataInput {
                    ty,
                    required: i.is_required() && i.default.is_none(),
                    name: i.name.as_str().to_string(),
                },
            );
        }
        for (n, o) in &event.outputs {
            m.outputs.insert(
                n.clone(),
                ReusableWorkflowMetadataOutput {
                    name: o.name.as_str().to_string(),
                },
            );
        }
        for (n, s) in &event.secrets {
            m.secrets.insert(
                n.clone(),
                ReusableWorkflowMetadataSecret {
                    required: s.required.as_ref().is_some_and(|b| b.value),
                    name: s.name.as_str().to_string(),
                },
            );
        }

        self.cache.write().unwrap().insert(spec, Some(m));
    }
}

/// A factory object to create a [`LocalReusableWorkflowCache`] instance per project.
pub struct LocalReusableWorkflowCacheFactory {
    caches: BTreeMap<PathBuf, std::sync::Arc<LocalReusableWorkflowCache>>,
    cwd: PathBuf,
    dbg: Option<std::sync::Arc<Box<dyn Write + Send>>>,
}

impl std::fmt::Debug for LocalReusableWorkflowCacheFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalReusableWorkflowCacheFactory")
            .field("caches", &self.caches.len())
            .field("cwd", &self.cwd)
            .finish()
    }
}

impl LocalReusableWorkflowCacheFactory {
    /// Creates a new factory. `cwd` is the current working directory; `dbg` is an optional debug
    /// writer.
    #[must_use]
    pub fn new(cwd: PathBuf, dbg: Option<std::sync::Arc<Box<dyn Write + Send>>>) -> Self {
        Self {
            caches: BTreeMap::new(),
            cwd,
            dbg,
        }
    }

    /// Returns a new or existing cache instance for the given project. When an instance was already
    /// created for the project, the existing instance is returned.
    pub fn get_cache(&mut self, p: Option<&Project>) -> std::sync::Arc<LocalReusableWorkflowCache> {
        let Some(p) = p else {
            return std::sync::Arc::new(LocalReusableWorkflowCache::new_null(
                self.dbg
                    .clone()
                    .map(|_b| Box::new(NullWrite) as Box<dyn Write + Send>),
            ));
        };
        let r = p.root_dir().to_path_buf();
        if let Some(c) = self.caches.get(&r) {
            return std::sync::Arc::clone(c);
        }
        let c = std::sync::Arc::new(LocalReusableWorkflowCache::new(
            Some(p.clone()),
            self.cwd.clone(),
            None,
        ));
        self.caches.insert(r, std::sync::Arc::clone(&c));
        c
    }
}

struct NullWrite;

impl Write for NullWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REUSABLE: &str = r"
on:
  workflow_call:
    inputs:
      my-input:
        type: string
        required: true
      num:
        type: number
    secrets:
      my-secret:
        required: true
    outputs:
      my-output:
        value: ${{ jobs.test.outputs.out }}
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";

    #[test]
    fn parse_metadata() {
        let m = parse_reusable_workflow_metadata(REUSABLE.as_bytes()).unwrap();
        assert!(m.inputs.contains_key("my-input"));
        assert!(m.inputs["my-input"].required);
        assert!(matches!(
            m.inputs["my-input"].ty.inner(),
            ExprTypeKindInner::String
        ));
        assert!(matches!(
            m.inputs["num"].ty.inner(),
            ExprTypeKindInner::Number
        ));
        assert!(m.secrets["my-secret"].required);
        assert!(m.outputs.contains_key("my-output"));
    }

    #[test]
    fn on_scalar_workflow_call() {
        let src = b"on: workflow_call\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo\n";
        let m = parse_reusable_workflow_metadata(src).unwrap();
        assert!(m.inputs.is_empty());
    }

    #[test]
    fn on_sequence_workflow_call() {
        let src = b"on: [workflow_call, push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo\n";
        let m = parse_reusable_workflow_metadata(src).unwrap();
        assert!(m.inputs.is_empty());
    }

    #[test]
    fn workflow_call_not_found() {
        let src =
            b"on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo\n";
        assert!(parse_reusable_workflow_metadata(src).is_err());
    }

    #[test]
    fn null_cache_never_hits() {
        let c = LocalReusableWorkflowCache::new_null(None);
        assert!(c.find_metadata("./x.yaml").unwrap().is_none());
    }

    #[test]
    fn find_local_reusable_workflow() {
        let tmp = std::env::temp_dir().join("actionlint_test_reusable");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".github").join("workflows")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(
            tmp.join(".github").join("workflows").join("reusable.yaml"),
            REUSABLE,
        )
        .unwrap();

        let proj = Project::new(tmp.clone()).unwrap();
        let c = LocalReusableWorkflowCache::new(Some(proj), tmp.clone(), None);
        let m = c
            .find_metadata("./.github/workflows/reusable.yaml")
            .unwrap();
        assert!(m.is_some());
        assert!(m.unwrap().inputs.contains_key("my-input"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_workflow_call_event() {
        use crate::ast::*;
        let tmp = std::env::temp_dir().join("actionlint_test_reusable_write");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".github").join("workflows")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();

        let proj = Project::new(tmp.clone()).unwrap();
        let c = LocalReusableWorkflowCache::new(Some(proj), tmp.clone(), None);

        let inputs = vec![WorkflowCallEventInput {
            name: String::at("my-input", Pos::new(1, 1)),
            description: None,
            default: None,
            required: Some(Bool {
                value: true,
                expression: None,
                pos: Pos::new(1, 1),
            }),
            input_type: WorkflowCallEventInputType::String,
            id: "my-input".to_string(),
        }];
        let mut secrets = BTreeMap::new();
        secrets.insert(
            "my-secret".to_string(),
            WorkflowCallEventSecret {
                name: String::at("my-secret", Pos::new(1, 1)),
                description: None,
                required: Some(Bool {
                    value: true,
                    expression: None,
                    pos: Pos::new(1, 1),
                }),
            },
        );
        let event = WorkflowCallEvent {
            inputs,
            secrets,
            secrets_present: true,
            outputs: BTreeMap::new(),
            pos: Pos::new(1, 1),
        };

        let wpath = tmp.join(".github").join("workflows").join("caller.yaml");
        c.write_workflow_call_event(&wpath, &event);

        let spec = c.conv_workflow_path_to_spec(&wpath).unwrap();
        let m = c.find_metadata(&spec).unwrap().unwrap();
        assert!(m.inputs.contains_key("my-input"));
        assert!(m.inputs["my-input"].required);
        assert!(m.secrets["my-secret"].required);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
