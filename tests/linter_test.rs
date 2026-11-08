//! Integration tests ported from the original `linter_test.go`.
//!
//! These drive the full linter against the fixture workflows copied from the original project:
//! - `testdata/ok/*.yaml` must lint with no errors.
//! - `testdata/{examples,err}/*.yaml` must produce exactly the errors listed in the matching
//!   `.out` file (a line wrapped in `/.../` is matched as a regular expression).
//!
//! Fixtures that require the external `shellcheck`/`pyflakes` commands are skipped when the
//! command is not installed, matching the original tests.

use std::path::{Path, PathBuf};

use actionlint::config::Config;
use actionlint::error::{Error, compare_errors};
use actionlint::linter::{Linter, LinterOptions};
use actionlint::project::Project;

/// Returns the absolute path of a `testdata/<sub>` directory.
fn testdata_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(sub)
}

/// Returns whether the given path is an executable file (Unix executable-bit check).
fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Finds an executable `name` on `PATH`, mirroring Go's `exec.LookPath`.
fn find_tool(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
        .map(|p| p.to_string_lossy().into_owned())
}

/// Lists the `.yaml`/`.yml` files in a directory, sorted by path for deterministic order.
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut fs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()))
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file() && matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"))
        })
        .collect();
    fs.sort();
    fs
}

/// Whether a fixture needs `shellcheck`/`pyflakes` and the command is missing. Returns the skip
/// reason, if any.
fn skip_reason(
    file_name: &str,
    shellcheck: Option<&str>,
    pyflakes: Option<&str>,
) -> Option<String> {
    if file_name.contains("shellcheck") && shellcheck.is_none() {
        return Some(format!(
            "skipping {file_name} because \"shellcheck\" command does not exist"
        ));
    }
    if file_name.contains("pyflakes") && pyflakes.is_none() {
        return Some(format!(
            "skipping {file_name} because \"pyflakes\" command does not exist"
        ));
    }
    None
}

#[test]
fn lint_ok() {
    let dir = testdata_dir("ok");
    let proj = Project::new(dir.clone()).expect("project");
    let shellcheck = find_tool("shellcheck");
    let pyflakes = find_tool("pyflakes");

    for f in yaml_files(&dir) {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(reason) = skip_reason(&name, shellcheck.as_deref(), pyflakes.as_deref()) {
            eprintln!("{reason}");
            continue;
        }

        let opts = LinterOptions {
            shellcheck: shellcheck.clone().unwrap_or_default(),
            pyflakes: pyflakes.clone().unwrap_or_default(),
            ..LinterOptions::default()
        };
        let content = std::fs::read(&f).unwrap();
        let path = f.to_string_lossy().into_owned();

        let mut out = std::io::sink();
        let mut linter =
            Linter::new_with_config(&mut out, opts, Some(Config::default())).expect("linter");
        let errs = linter
            .lint(&path, &content, Some(proj.clone()))
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert!(
            errs.is_empty(),
            "{name}: expected no errors but got {}:\n{}",
            errs.len(),
            errs.iter()
                .map(Error::summary)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn lint_error() {
    let shellcheck = find_tool("shellcheck");
    let pyflakes = find_tool("pyflakes");
    // Collect every fixture mismatch so a single run reports all failing fixtures instead of
    // stopping at the first one. This is a dev-time convenience; the assertions below still fail
    // the test if any fixture mismatched.
    let failures: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    for sub in ["examples", "err"] {
        let dir = testdata_dir(sub);
        let proj = Project::new(dir.clone()).expect("project");

        for infile in yaml_files(&dir) {
            let name = infile.file_stem().unwrap().to_string_lossy().into_owned();
            if let Some(reason) = skip_reason(&name, shellcheck.as_deref(), pyflakes.as_deref()) {
                eprintln!("{reason}");
                continue;
            }

            let outfile = infile.with_extension("out");
            if !outfile.exists() {
                continue;
            }

            // Mirror the original: only enable shellcheck/pyflakes for fixtures whose name
            // references them, so unrelated fixtures don't get extra tool-reported errors.
            let opts = LinterOptions {
                shellcheck: if name.contains("shellcheck") {
                    shellcheck.clone().unwrap_or_default()
                } else {
                    String::new()
                },
                pyflakes: if name.contains("pyflakes") {
                    pyflakes.clone().unwrap_or_default()
                } else {
                    String::new()
                },
                ..LinterOptions::default()
            };
            let content = std::fs::read(&infile).unwrap();
            let expected = std::fs::read_to_string(&outfile).unwrap();

            let mut out = std::io::sink();
            let mut linter =
                Linter::new_with_config(&mut out, opts, Some(Config::default())).expect("linter");
            let errs = match linter.lint("test.yaml", &content, Some(proj.clone())) {
                Ok(errs) => errs,
                Err(e) => {
                    failures
                        .lock()
                        .unwrap()
                        .push(format!("{sub}/{name}: fatal: {e}"));
                    continue;
                }
            };

            check_errors(&format!("{sub}/{name}"), &expected, errs, &failures);
        }
    }

    let all = failures.into_inner().unwrap();
    assert!(
        all.is_empty(),
        "{} fixture(s) mismatched:\n{}",
        all.len(),
        all.join("\n----\n")
    );
}

/// Compares the produced errors against the expected lines in an `.out` file, recording any
/// mismatch into `failures` instead of panicking immediately. Counterpart of `checkErrors`.
fn check_errors(
    name: &str,
    expected_src: &str,
    mut errs: Vec<Error>,
    failures: &std::sync::Mutex<Vec<String>>,
) {
    let expected: Vec<&str> = expected_src.lines().collect();
    errs.sort_by(compare_errors);
    let have: Vec<String> = errs.iter().map(Error::summary).collect();

    if have.len() != expected.len() {
        failures.lock().unwrap().push(format!(
            "{name}: {} errors are expected but actually got {} errors:\n  want:\n    {}\n  have:\n    {}",
            expected.len(),
            have.len(),
            expected.join("\n    "),
            have.join("\n    ")
        ));
        return;
    }

    let mut problems: Vec<String> = Vec::new();
    for (i, (want, hv)) in expected.iter().zip(&have).enumerate() {
        if want.len() > 1 && want.starts_with('/') && want.ends_with('/') {
            let pattern = &want[1..want.len() - 1];
            let rust_pattern = adapt_go_regex(pattern);
            let re = match regex::Regex::new(&rust_pattern) {
                Ok(re) => re,
                Err(e) => {
                    problems.push(format!("{name}: invalid regex {rust_pattern:?}: {e}"));
                    continue;
                }
            };
            if !re.is_match(hv) {
                problems.push(format!(
                    "{name}: error message mismatch at {i}th error does not match to regular expression\n  want: /{pattern}/\n  have: {hv:?}"
                ));
            }
        } else if hv != want {
            problems.push(format!(
                "{name}: error message mismatch at {i}th error does not match exactly\n  want: {want:?}\n  have: {hv:?}"
            ));
        }
    }
    if !problems.is_empty() {
        failures.lock().unwrap().push(problems.join("\n"));
    }
}

/// Adapts a Go-regexp-flavored pattern for the `regex` crate by escaping bare `{`/`}` braces that
/// Go would treat as literals. Valid repetition quantifiers (`{n}`, `{n,}`, `{n,m}`) are preserved.
fn adapt_go_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                // Keep escape sequences verbatim.
                out.push('\\');
                if i + 1 < chars.len() {
                    i += 1;
                    out.push(chars[i]);
                }
            }
            '{' => {
                // A quantifier must be `{` + digits (+ optional `,` + digits) + `}`.
                let mut j = i + 1;
                let mut digits = 0;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                    digits += 1;
                }
                let mut is_quantifier = digits > 0;
                if is_quantifier && j < chars.len() && chars[j] == ',' {
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                is_quantifier = is_quantifier && j < chars.len() && chars[j] == '}';
                if is_quantifier {
                    while i <= j {
                        out.push(chars[i]);
                        i += 1;
                    }
                    continue;
                }
                out.push_str("\\{");
            }
            '}' => out.push_str("\\}"),
            c => out.push(c),
        }
        i += 1;
    }
    out
}
