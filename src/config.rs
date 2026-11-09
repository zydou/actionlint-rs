//! actionlint configuration.
//!
//! Rust counterpart of the original `config.go`. The configuration is read from an
//! `actionlint.yaml` file usually placed in the `.github` directory.

use std::path::Path;

use regex::Regex;
use saphyr::LoadableYamlNode;

/// A list of regular expressions used to filter errors by matching their messages.
#[derive(Debug, Clone, Default)]
pub struct IgnorePatterns(pub Vec<Regex>);

impl IgnorePatterns {
    /// Returns whether the given error message should be ignored.
    #[must_use]
    pub fn matches(&self, message: &str) -> bool {
        self.0.iter().any(|r| r.is_match(message))
    }
}

/// A configuration for a specific file path pattern, corresponding to a value of the `paths`
/// mapping in the configuration file.
#[derive(Debug, Clone, Default)]
pub struct PathConfig {
    /// Patterns used to ignore errors by matching their messages, similar to `--ignore`.
    pub ignore: IgnorePatterns,
}

/// A [`PathConfig`] paired with the compiled matcher for its glob pattern. The matcher is built
/// once during parsing so [`Config::path_configs`] can reuse it, and it uses `literal_separator`
/// semantics so a single `*` does not match `/` (while `**` still spans directories), matching the
/// original's use of `doublestar`.
#[derive(Debug, Clone)]
pub struct CompiledPathConfig {
    /// The glob pattern as written in the config file.
    pub pattern: String,
    /// The compiled matcher for [`Self::pattern`].
    pub matcher: globset::GlobMatcher,
    /// The per-path configuration.
    pub config: PathConfig,
}

/// The actionlint configuration, parsed from an `actionlint.yaml` file.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Label names for the self-hosted runner.
    pub self_hosted_runner_labels: Vec<String>,
    /// Names of configuration variables used in the checked workflows. When `None`, property
    /// names of the `vars` context are not checked. An empty list means no variable is allowed.
    pub config_variables: Option<Vec<String>>,
    /// The `paths` entries with their glob patterns compiled, in the order they appeared. Used by
    /// [`Config::path_configs`] to match file paths.
    pub compiled_paths: Vec<CompiledPathConfig>,
}

impl Config {
    /// Returns all [`PathConfig`] values matching the given file path. The path must be relative
    /// to the root of the project and use `/` as the separator.
    #[must_use]
    pub fn path_configs(&self, path: &str) -> Vec<&PathConfig> {
        let path = path.replace('\\', "/");
        let mut ret = Vec::new();
        for compiled in &self.compiled_paths {
            if compiled.matcher.is_match(&path) {
                ret.push(&compiled.config);
            }
        }
        ret
    }
}

/// Parses the given bytes as an actionlint config file. Returns an error when the YAML is
/// malformed or a value has the wrong type.
pub fn parse_config(b: &[u8]) -> Result<Config, String> {
    let text = std::str::from_utf8(b).map_err(|e| e.to_string())?;
    let docs = saphyr::Yaml::load_from_str(text).map_err(|e| e.to_string())?;
    let Some(doc) = docs.into_iter().next() else {
        // An empty (0-byte or whitespace-only) file unmarshals to the zero config in the original,
        // which then lints normally. Treat it as an empty config rather than an error.
        return Ok(Config::default());
    };
    // `null` is the zero value in the original's YAML unmarshalling, so it yields the default
    // config. Any other non-mapping document (sequence, plain scalar) is a type error.
    if matches!(doc, saphyr::Yaml::Value(saphyr::Scalar::Null)) {
        return Ok(Config::default());
    }

    let saphyr::Yaml::Mapping(mapping) = &doc else {
        return Err("config file must be a mapping".to_string());
    };

    let mut config = Config::default();
    for (k, v) in mapping {
        let Some(key) = scalar_str(k) else {
            continue;
        };
        match key.as_str() {
            "self-hosted-runner" => parse_self_hosted_runner(v, &mut config)?,
            "config-variables" => {
                // `null` disables the check (None), while `[]` means no variable is allowed.
                if !matches!(v, saphyr::Yaml::Value(saphyr::Scalar::Null)) {
                    config.config_variables = Some(parse_string_seq(v)?);
                }
            }
            "paths" => parse_paths(v, &mut config)?,
            _ => {}
        }
    }
    Ok(config)
}

/// Reads an actionlint config file from the given path.
pub fn read_config_file(path: &Path) -> Result<Config, String> {
    let b = std::fs::read(path)
        .map_err(|e| format!("could not read config file {}: {e}", path.display()))?;
    parse_config(&b).map_err(|e| format!("could not parse config file {}: {e}", path.display()))
}

/// Reads the repository's `.github/actionlint.yaml` or `.github/actionlint.yml`. Returns `None`
/// when neither file exists.
pub fn load_repo_config(root: &Path) -> Result<Option<Config>, String> {
    for f in ["actionlint.yaml", "actionlint.yml"] {
        let p = root.join(".github").join(f);
        match read_config_file(&p) {
            Ok(c) => return Ok(Some(c)),
            Err(e) if !is_not_found(&e) => return Err(e),
            _ => {}
        }
    }
    Ok(None)
}

fn is_not_found(err: &str) -> bool {
    err.contains("os error 2") || err.contains("No such file")
}

fn parse_self_hosted_runner(v: &saphyr::Yaml<'_>, config: &mut Config) -> Result<(), String> {
    // `null` is the zero value, but any other non-mapping (e.g. a scalar) is a type error, matching
    // the original's strict YAML unmarshalling into the config struct.
    if matches!(v, saphyr::Yaml::Value(saphyr::Scalar::Null)) {
        return Ok(());
    }
    let saphyr::Yaml::Mapping(mapping) = v else {
        return Err("self-hosted-runner must be a mapping node".to_string());
    };
    for (k, val) in mapping {
        if scalar_str(k).as_deref() == Some("labels") {
            config.self_hosted_runner_labels = parse_string_seq(val)?;
        }
    }
    Ok(())
}

fn parse_paths(v: &saphyr::Yaml<'_>, config: &mut Config) -> Result<(), String> {
    // `null` (e.g. `paths:` with the entries commented out) is the zero value in the original's
    // YAML unmarshalling, yielding an empty mapping.
    if matches!(v, saphyr::Yaml::Value(saphyr::Scalar::Null)) {
        return Ok(());
    }
    // The original unmarshals `paths` into a map, so a non-mapping value (e.g. `paths: 42`) is a
    // type error rather than an empty mapping.
    let saphyr::Yaml::Mapping(mapping) = v else {
        return Err("paths must be a mapping node".to_string());
    };
    for (k, val) in mapping {
        let Some(pattern) = scalar_str(k) else {
            continue;
        };
        // Validate the pattern (rejecting invalid globs) and compile a matcher that treats a single
        // `*` as not crossing `/`, matching the original's `doublestar` semantics.
        let matcher = globset::GlobBuilder::new(&pattern)
            .literal_separator(true)
            .build()
            .map_err(|_| format!("invalid glob pattern {pattern:?} in \"paths\""))?
            .compile_matcher();
        let mut pc = PathConfig::default();
        if let saphyr::Yaml::Mapping(m) = val {
            for (ik, iv) in m {
                if scalar_str(ik).as_deref() == Some("ignore") {
                    let patterns = parse_string_seq(iv)?;
                    let mut regexes = Vec::with_capacity(patterns.len());
                    for p in patterns {
                        let r = Regex::new(&p).map_err(|e| {
                            format!("invalid regular expression {p:?} in \"ignore\": {e}")
                        })?;
                        regexes.push(r);
                    }
                    pc.ignore = IgnorePatterns(regexes);
                }
            }
        }
        config.compiled_paths.push(CompiledPathConfig {
            pattern,
            matcher,
            config: pc,
        });
    }
    Ok(())
}

fn parse_string_seq(v: &saphyr::Yaml<'_>) -> Result<Vec<String>, String> {
    // The original unmarshals these fields into slices, so a non-sequence value (e.g. a plain
    // string) is a type error rather than an empty list.
    let saphyr::Yaml::Sequence(seq) = v else {
        return Err("expected a sequence node".to_string());
    };
    // Non-string scalars (numbers, booleans) are coerced to their string form, matching the
    // original's YAML unmarshalling into `[]string`.
    Ok(seq.iter().filter_map(scalar_str).collect())
}

/// Extracts a scalar string value from a YAML node, coercing non-string scalars (numbers,
/// booleans) to their string form. This mirrors the original's YAML unmarshalling into `[]string`
/// and string map keys, where a scalar such as `42` or `true` becomes `"42"` / `"true"`.
fn scalar_str(v: &saphyr::Yaml<'_>) -> Option<String> {
    match v {
        saphyr::Yaml::Value(saphyr::Scalar::String(c)) => Some(c.to_string()),
        saphyr::Yaml::Value(saphyr::Scalar::Boolean(b)) => Some(b.to_string()),
        saphyr::Yaml::Value(saphyr::Scalar::Integer(i)) => Some(i.to_string()),
        saphyr::Yaml::Value(saphyr::Scalar::FloatingPoint(f)) => Some(f.to_string()),
        saphyr::Yaml::Representation(c, _, _) => Some(c.to_string()),
        _ => None,
    }
}

/// The contents of a freshly generated default config file.
pub const DEFAULT_CONFIG: &str = r#"self-hosted-runner:
  # Labels of self-hosted runner in array of strings.
  labels: []

# Configuration variables in array of strings defined in your repository or
# organization. `null` means disabling configuration variables check.
# Empty array means no configuration variable is allowed.
config-variables: null

# Configuration for file paths. The keys are glob patterns to match to file
# paths relative to the repository root. The values are the configurations for
# the file paths. Note that the path separator is always '/'.
# The following configurations are available.
#
# "ignore" is an array of regular expression patterns. Matched error messages
# are ignored. This is similar to the "-ignore" command line option.
paths:
#  .github/workflows/**/*.yml:
#    ignore: []
"#;

/// Writes the default config file to the given path.
pub fn write_default_config_file(path: &Path) -> Result<(), String> {
    std::fs::write(path, DEFAULT_CONFIG).map_err(|e| {
        format!(
            "could not write default configuration file at {}: {e}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let c = parse_config(b"self-hosted-runner:\n  labels: [linux, gpu]\n").unwrap();
        assert_eq!(c.self_hosted_runner_labels, vec!["linux", "gpu"]);
        assert!(c.config_variables.is_none());
    }

    #[test]
    fn parse_config_variables() {
        let c = parse_config(b"config-variables: [foo, bar]\n").unwrap();
        assert_eq!(
            c.config_variables,
            Some(vec!["foo".to_string(), "bar".to_string()])
        );
    }

    #[test]
    fn parse_empty_config_variables() {
        let c = parse_config(b"config-variables: []\n").unwrap();
        assert_eq!(c.config_variables, Some(vec![]));
    }

    #[test]
    fn parse_paths_and_ignore() {
        let src = b"\
paths:
  .github/workflows/*.yml:
    ignore:
      - 'label \".+\" is unknown'
";
        let c = parse_config(src).unwrap();
        let pc = c.path_configs(".github/workflows/ci.yml");
        assert_eq!(pc.len(), 1);
        assert!(pc[0].ignore.matches("label \"foo\" is unknown"));
        assert!(!pc[0].ignore.matches("some other error"));
    }

    #[test]
    fn path_configs_no_match() {
        let src = b"paths:\n  docs/**/*.yml:\n    ignore: []\n";
        let c = parse_config(src).unwrap();
        assert!(c.path_configs(".github/workflows/ci.yml").is_empty());
    }

    #[test]
    fn invalid_glob_rejected() {
        let src = b"paths:\n  '[invalid':\n    ignore: []\n";
        assert!(parse_config(src).is_err());
    }

    #[test]
    fn invalid_regex_rejected() {
        let src = b"paths:\n  '**/*.yml':\n    ignore:\n      - '('\n";
        assert!(parse_config(src).is_err());
    }

    #[test]
    fn empty_config_is_default() {
        // A 0-byte config unmarshals to the zero config in the original, which then lints normally.
        let c = parse_config(b"").unwrap();
        assert!(c.self_hosted_runner_labels.is_empty());
        assert!(c.config_variables.is_none());
        assert!(c.compiled_paths.is_empty());
    }

    #[test]
    fn null_config_is_default() {
        // A top-level `null` is the zero value in the original's YAML unmarshalling.
        let c = parse_config(b"null").unwrap();
        assert!(c.self_hosted_runner_labels.is_empty());
        assert!(c.config_variables.is_none());
        assert!(c.compiled_paths.is_empty());
    }

    #[test]
    fn top_level_sequence_rejected() {
        assert!(parse_config(b"- a\n- b\n").is_err());
    }

    #[test]
    fn top_level_scalar_rejected() {
        assert!(parse_config(b"hello").is_err());
    }

    #[test]
    fn scalar_where_sequence_expected_rejected() {
        // A plain string where a sequence is required is a type error, matching the original's
        // strict unmarshalling into `[]string`.
        assert!(parse_config(b"self-hosted-runner:\n  labels: \"foo\"\n").is_err());
        assert!(parse_config(b"config-variables: \"foo\"\n").is_err());
        assert!(
            parse_config(b"paths:\n  '.github/workflows/*.yml':\n    ignore: \"x\"\n").is_err()
        );
    }

    #[test]
    fn paths_non_mapping_rejected() {
        assert!(parse_config(b"paths: 42\n").is_err());
    }

    #[test]
    fn integer_in_labels_coerced_to_string() {
        // Non-string scalars inside a sequence are coerced to their string form, matching the
        // original's unmarshalling into `[]string`.
        let c = parse_config(b"self-hosted-runner:\n  labels: [42, true]\n").unwrap();
        assert_eq!(
            c.self_hosted_runner_labels,
            vec!["42".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn single_glob_does_not_cross_separator() {
        // A single `*` must not match `/` (matching the original's `doublestar` semantics), so a
        // top-level pattern does not apply to a nested workflow file.
        let src = b"paths:\n  '.github/workflows/*.yml':\n    ignore: ['.']\n";
        let c = parse_config(src).unwrap();
        assert!(
            c.path_configs(".github/workflows/sub/nested.yml")
                .is_empty()
        );
        assert_eq!(c.path_configs(".github/workflows/ci.yml").len(), 1);
    }

    #[test]
    fn default_config_roundtrip() {
        let c = parse_config(DEFAULT_CONFIG.as_bytes()).unwrap();
        assert!(c.self_hosted_runner_labels.is_empty());
        assert!(c.config_variables.is_none());
        assert!(c.compiled_paths.is_empty());
    }
}
