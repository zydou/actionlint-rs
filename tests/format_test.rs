//! Integration tests for the `--format` option, ported from the original `linter_test.go`
//! (`TestLinterFormatErrorMessageOK` and `TestLinterFormatErrorMessageInSARIF`).
//!
//! Each test lints `testdata/format/test.yaml` with a Go-template format string and compares the
//! rendered output against the expected file: byte-for-byte for `test.json`/`test.jsonl`/`test.md`,
//! and structurally (parsed JSON, as the original does) for the SARIF fixture.

use std::path::PathBuf;

use actionlint::config::Config;
use actionlint::linter::{Linter, LinterOptions};
use actionlint::project::Project;

fn format_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("format")
}

/// Lints the shared `test.yaml` fixture with the given format template and returns the rendered
/// output.
fn render(format: &str) -> String {
    let dir = format_dir();
    let proj = Project::new(dir.clone()).expect("project");
    let infile = dir.join("test.yaml");
    let content = std::fs::read(&infile).unwrap();

    // The error `filepath` is populated verbatim from the path passed to `lint`, and the expected
    // fixtures (matching the original Go test) use the CWD-relative path. Cargo runs tests with
    // the package root as the working directory, so derive the relative path instead of passing
    // the absolute one.
    let rel = infile
        .strip_prefix(std::env::current_dir().unwrap())
        .unwrap_or(&infile);

    let opts = LinterOptions {
        format: format.to_string(),
        ..LinterOptions::default()
    };
    let mut out = Vec::new();
    let mut linter =
        Linter::new_with_config(&mut out, opts, Some(Config::default())).expect("linter");
    let errs = linter
        .lint(rel.to_string_lossy().as_ref(), &content, Some(proj))
        .expect("lint");
    assert!(
        !errs.is_empty(),
        "no error was reported for the format fixture"
    );
    String::from_utf8(out).expect("output is utf-8")
}

fn expect(file: &str, format: &str) {
    let want = std::fs::read_to_string(format_dir().join(file)).unwrap();
    let have = render(format);
    assert_eq!(have, want, "formatted output mismatch for {file}");
}

#[test]
fn format_error_message_ok() {
    expect("test.json", "{{json .}}");
    expect("test.jsonl", "{{range $err := .}}{{json $err}}{{end}}");
    expect(
        "test.md",
        "{{range $ := .}}### Error at line {{$.Line}}, col {{$.Column}} of `{{$.Filepath}}`\n\n{{$.Message}}\n\n```\n{{$.Snippet}}\n```\n\n{{end}}",
    );
}

#[test]
fn format_error_message_in_sarif() {
    let dir = format_dir();
    let format = std::fs::read_to_string(dir.join("sarif_template.txt")).unwrap();
    let want = std::fs::read_to_string(dir.join("test.sarif")).unwrap();
    let have = render(&format);

    // The original Go test (`TestLinterFormatErrorMessageInSARIF`) unmarshals both the rendered
    // output and the expected file into JSON and compares the resulting structures rather than
    // the raw bytes. The template's literal indentation differs from the re-marshaled fixture,
    // so a byte-for-byte comparison is not meaningful. Mirror that by comparing the parsed JSON.
    let mut have_json: serde_json::Value =
        serde_json::from_str(&have).expect("rendered SARIF is JSON");
    let mut want_json: serde_json::Value =
        serde_json::from_str(&want).expect("expected SARIF is JSON");

    // The `driver.version` field comes from the build (Go's `(devel)` build info vs this crate's
    // Cargo version), so it is not reproducible across toolchains. Normalize it on both sides
    // before comparing, the same way the Go build injects its own version at test time.
    let version_path = "/runs/0/tool/driver/version";
    *have_json.pointer_mut(version_path).expect("have version") = serde_json::Value::Null;
    *want_json.pointer_mut(version_path).expect("want version") = serde_json::Value::Null;

    assert_eq!(have_json, want_json, "formatted SARIF output mismatch");
}
