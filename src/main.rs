//! Entry point of the `actionlint` command.

use std::io;
use std::process::ExitCode;

use actionlint::command::Command;

fn main() -> ExitCode {
    let mut cmd = Command::new(io::stdin(), io::stdout(), io::stderr());
    // `-verbose` / `-debug` diagnostics go to stderr, mirroring the original's
    // `opts.LogWriter = cmd.Stderr`. A fresh handle is independent of the one used for fatal errors.
    cmd.set_log_writer(Box::new(io::stderr()));
    ExitCode::from(cmd.main(std::env::args_os().collect()))
}
