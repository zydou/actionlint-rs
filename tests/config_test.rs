//! Integration tests for reading config files, ported from the original `config_test.go`
//! (`TestConfigReadFileOK`, `TestConfigReadFileReadError`, `TestConfigReadFileParseError`).

use std::path::PathBuf;

use actionlint::config::read_config_file;

fn config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("config")
}

#[test]
fn read_file_ok() {
    let p = config_dir().join("ok.yml");
    let c = read_config_file(&p).expect("config should parse");
    assert_eq!(c.self_hosted_runner_labels, vec!["foo", "bar"]);
}

#[test]
fn read_file_read_error() {
    let p = config_dir().join("does-not-exist.yml");
    let err = read_config_file(&p).expect_err("error should occur");
    assert!(
        err.contains("could not read config file"),
        "unexpected error message: {err:?}"
    );
}

#[test]
fn read_file_parse_error() {
    let p = config_dir().join("broken.yml");
    let err = read_config_file(&p).expect_err("error should occur");
    assert!(
        err.contains("could not parse config file"),
        "unexpected error message: {err:?}"
    );
}
