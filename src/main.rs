//! Entry point of the `actionlint` command.

use std::io;
use std::process::ExitCode;

use actionlint::command::Command;

fn main() -> ExitCode {
    let mut cmd = Command::new(io::stdin(), io::stdout(), io::stderr());
    ExitCode::from(cmd.main(std::env::args_os().collect()))
}
