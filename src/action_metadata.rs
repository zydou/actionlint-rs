//! Action metadata (`action.yml`) parsing and local action caching.
//!
//! Rust counterpart of the original `action_metadata.go`. It parses the metadata of actions and
//! caches local action metadata to avoid repeated reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use saphyr::LoadableYamlNode;
use serde::Deserialize;

/// Input metadata in the `inputs` section of an `action.yml` metadata file.
///
/// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#inputs>.
#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct ActionMetadataInput {
    /// The name of this input. Populated from the map key after deserialization.
    pub name: String,
    /// Whether this input is mandatory to run the action.
    pub required: bool,
    /// Whether this input is marked as deprecated.
    pub deprecated: bool,
    /// The deprecation message for a deprecated input.
    #[serde(rename = "deprecation-message")]
    pub deprecation_message: String,
}

/// A map from input ID to its metadata. Keys are lower cased since input names are
/// case-insensitive.
pub type ActionMetadataInputs = BTreeMap<String, ActionMetadataInput>;

/// Output metadata in the `outputs` section of an `action.yml` metadata file.
#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct ActionMetadataOutput {
    /// The name of the output. Populated from the map key after deserialization.
    pub name: String,
}

/// A map from output ID to its metadata. Keys are lower cased since output names are
/// case-insensitive.
pub type ActionMetadataOutputs = BTreeMap<String, ActionMetadataOutput>;

/// The `runs` section of `action.yaml`, defining how the action is run.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ActionMetadataRuns {
    /// The `using` configuration, defining what runner is used for the action.
    #[serde(default)]
    pub using: String,
    /// The `main` configuration for a JavaScript action.
    #[serde(default)]
    pub main: String,
    /// The `image` of `action.yaml` for a Docker action.
    #[serde(default)]
    pub image: String,
}

/// The `branding` section of `action.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ActionMetadataBranding {
    /// The `icon` of the action.
    #[serde(default)]
    pub icon: String,
    /// The `color` of the action.
    #[serde(default)]
    pub color: String,
}

/// The structure of `action.yaml`.
///
/// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions>.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ActionMetadata {
    #[serde(skip)]
    dir: PathBuf,
    #[serde(skip)]
    file: String,
    /// The `name` field of `action.yaml`.
    #[serde(default)]
    pub name: String,
    /// The `description` field of `action.yaml`.
    #[serde(default)]
    pub description: String,
    /// The `inputs` field of `action.yaml`.
    #[serde(default)]
    pub inputs: ActionMetadataInputs,
    /// The `outputs` field of `action.yaml`, keyed by output name.
    #[serde(default)]
    pub outputs: ActionMetadataOutputs,
    /// Flag to skip the inputs check for this action.
    #[serde(default, skip)]
    pub skip_inputs: bool,
    /// Flag to accept any properties on the outputs object in addition to typed props.
    #[serde(default, skip)]
    pub skip_outputs: bool,
    /// The `runs` field of `action.yaml`.
    #[serde(default)]
    pub runs: ActionMetadataRuns,
    /// The `branding` field of `action.yaml`.
    #[serde(default)]
    pub branding: ActionMetadataBranding,
}

impl ActionMetadata {
    /// Returns the directory path of the action.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the file path of the action's metadata file.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.file)
    }

    /// Parses action metadata from the given bytes. `dir` is the action directory and `file` is the
    /// metadata file name.
    pub fn parse(dir: PathBuf, file: &str, src: &[u8]) -> Result<Self, String> {
        let mut md: ActionMetadata = serde_yaml_from_slice(src)?;
        md.dir = dir;
        md.file = file.to_string();
        // Lower case input/output IDs for case-insensitive lookup.
        let inputs = md
            .inputs
            .into_iter()
            .map(|(k, mut v)| {
                v.name = k.clone();
                (k.to_lowercase(), v)
            })
            .collect();
        md.inputs = inputs;
        let outputs = md
            .outputs
            .into_iter()
            .map(|(k, mut v)| {
                v.name = k.clone();
                (k.to_lowercase(), v)
            })
            .collect();
        md.outputs = outputs;
        Ok(md)
    }
}

/// Parses YAML using saphyr and converts to the target type via serde.
///
/// We use saphyr (already a dependency for workflow parsing) plus a small serde bridge to avoid
/// pulling in an unmaintained `serde_yaml`.
fn serde_yaml_from_slice<T: for<'de> Deserialize<'de>>(src: &[u8]) -> Result<T, String> {
    let text = std::str::from_utf8(src).map_err(|e| e.to_string())?;
    let docs = saphyr::Yaml::load_from_str(text).map_err(|e| e.to_string())?;
    let doc = docs.into_iter().next().ok_or("empty metadata file")?;
    let value = yaml_to_serde(&doc);
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Converts a saphyr [`saphyr::Yaml`] node into a [`serde_json::Value`] so it can be deserialized.
pub fn yaml_to_serde(v: &saphyr::Yaml<'_>) -> serde_json::Value {
    match v {
        saphyr::Yaml::Value(s) => match s {
            saphyr::Scalar::Null => serde_json::Value::Null,
            saphyr::Scalar::Boolean(b) => serde_json::Value::Bool(*b),
            saphyr::Scalar::Integer(i) => serde_json::Value::from(*i),
            saphyr::Scalar::FloatingPoint(f) => serde_json::Value::from(f.into_inner()),
            saphyr::Scalar::String(c) => serde_json::Value::String(c.to_string()),
        },
        saphyr::Yaml::Representation(c, _, _) => serde_json::Value::String(c.to_string()),
        saphyr::Yaml::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_to_serde).collect())
        }
        saphyr::Yaml::Mapping(map) => {
            let mut m = serde_json::Map::new();
            for (k, val) in map {
                let key = match k {
                    saphyr::Yaml::Value(saphyr::Scalar::String(c)) => c.to_string(),
                    saphyr::Yaml::Representation(c, _, _) => c.to_string(),
                    other => format!("{other:?}"),
                };
                m.insert(key, yaml_to_serde(val));
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::Value::Null,
    }
}

/// A cache for local actions' metadata, avoiding repeated find/read/parse of `action.yml`.
///
/// This cache is not shared across repositories; one instance should be created per repository.
#[derive(Debug, Default)]
pub struct LocalActionsCache {
    proj: Option<PathBuf>,
    cache: RwLock<BTreeMap<String, Option<ActionMetadata>>>,
}

impl LocalActionsCache {
    /// Creates a new cache for the given project root. `proj` of `None` creates a null cache that
    /// never hits.
    #[must_use]
    pub fn new(proj: Option<PathBuf>) -> Self {
        Self {
            proj,
            cache: RwLock::new(BTreeMap::new()),
        }
    }

    /// Finds the metadata for the given local action `spec` (which must start with `./`).
    ///
    /// Returns `Ok(None)` when the action does not exist or was previously found invalid (the
    /// not-found/invalid result is cached to avoid repeating the same error).
    pub fn find_metadata(&self, spec: &str) -> Result<Option<ActionMetadata>, String> {
        let Some(proj) = &self.proj else {
            return Ok(None);
        };
        if !spec.starts_with("./") {
            return Ok(None);
        }

        if let Some(hit) = self.cache.read().unwrap().get(spec) {
            return Ok(hit.clone());
        }

        let dir = proj.join(spec.trim_start_matches("./"));
        let Some((bytes, file)) = read_local_action_metadata_file(&dir) else {
            // Do not complain when the local action does not exist (e.g. Git submodule).
            self.cache.write().unwrap().insert(spec.to_string(), None);
            return Ok(None);
        };

        match ActionMetadata::parse(dir, &file, &bytes) {
            Ok(meta) => {
                self.cache
                    .write()
                    .unwrap()
                    .insert(spec.to_string(), Some(meta.clone()));
                Ok(Some(meta))
            }
            Err(e) => {
                self.cache.write().unwrap().insert(spec.to_string(), None);
                Err(format!("error while parsing action metadata: {e}"))
            }
        }
    }
}

/// Reads the `action.yml` or `action.yaml` metadata file in the given directory.
fn read_local_action_metadata_file(dir: &Path) -> Option<(Vec<u8>, String)> {
    for f in ["action.yml", "action.yaml"] {
        let p = dir.join(f);
        if let Ok(b) = std::fs::read(&p) {
            return Some((b, f.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r"
name: My Action
description: Does things
inputs:
  my-input:
    description: an input
    required: true
  other:
    description: another
outputs:
  my-output:
    description: an output
runs:
  using: node20
  main: dist/index.js
";

    #[test]
    fn parse_metadata() {
        let md =
            ActionMetadata::parse(PathBuf::from("/foo"), "action.yml", META.as_bytes()).unwrap();
        assert_eq!(md.name, "My Action");
        assert_eq!(md.description, "Does things");
        assert_eq!(md.runs.using, "node20");
        assert!(md.inputs.contains_key("my-input"));
        assert!(md.inputs["my-input"].required);
        assert!(md.outputs.contains_key("my-output"));
        assert_eq!(md.path(), PathBuf::from("/foo/action.yml"));
    }

    #[test]
    fn input_ids_lowercased() {
        let src = "name: x\ninputs:\n  MyInput:\n    description: d\n";
        let md = ActionMetadata::parse(PathBuf::from("/"), "action.yml", src.as_bytes()).unwrap();
        assert!(md.inputs.contains_key("myinput"));
        assert_eq!(md.inputs["myinput"].name, "MyInput");
    }

    #[test]
    fn local_cache_find() {
        let tmp = std::env::temp_dir().join("actionlint_test_action");
        let dir = tmp.join("my-action");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("action.yml"), META).unwrap();

        let cache = LocalActionsCache::new(Some(tmp.clone()));
        let md = cache.find_metadata("./my-action").unwrap();
        assert!(md.is_some());
        assert_eq!(md.unwrap().name, "My Action");

        // Second lookup hits the cache.
        let md2 = cache.find_metadata("./my-action").unwrap();
        assert!(md2.is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn local_cache_missing_returns_none() {
        let tmp = std::env::temp_dir().join("actionlint_test_action_missing");
        let _ = std::fs::create_dir_all(&tmp);
        let cache = LocalActionsCache::new(Some(tmp.clone()));
        let md = cache.find_metadata("./nope").unwrap();
        assert!(md.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn null_cache_never_hits() {
        let cache = LocalActionsCache::new(None);
        assert!(cache.find_metadata("./anything").unwrap().is_none());
    }
}
